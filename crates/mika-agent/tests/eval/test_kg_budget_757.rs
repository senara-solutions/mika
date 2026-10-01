//! # KG extraction budget + idempotency (#757)
//!
//! Covers the Unit 2 acceptance criteria for the #757 rescue fix:
//!
//! - `extract_pending(budget)` honors the per-batch LLM call cap.
//! - `budget == 0` short-circuits with zero LLM calls.
//! - The pending-doc query matches on `(docs_root_hash, source_doc_path,
//!   source_doc_hash)` so stale markers re-fire only on actual content drift.
//! - `record_extraction` persists `source_doc_hash` so subsequent runs see a
//!   populated marker.
//!
//! These tests use `MockLlmProvider` only — no real-provider traffic.
//!
//! # Ce que mika#1833 a changé dans ce fichier, et pourquoi
//!
//! Quatre tests épinglaient ici le comportement **« budget nul laisse passer
//! gratuitement les exact matches de Stage-1 »**, ajouté délibérément par la
//! revue de #757 (finding P1). mika#1833 le retire : `budget == 0` est
//! désormais un no-op déclaré, court-circuité **avant toute requête**. Le
//! raisonnement complet — les deux mesures qui condamnent l'ancien
//! comportement, et le levier qui reste pour l'intention « exact-match
//! seulement » — vit à un seul endroit,
//! [`mika_agent::kg::budget::phase_is_disabled`], et n'est pas recopié ici.
//!
//! Deux des quatre gardent toute leur valeur et ne perdent que leur budget
//! nul : la non-starvation après un skip (le finding P1 lui-même) et la
//! composition exact-match / sans-LLM sont **orthogonales au budget**, donc
//! elles sont rejouées sous un budget non nul, où elles restent vraies et
//! toujours gardées contre une régression. Les deux autres portaient le
//! contrat retiré : ils épinglent maintenant son retrait, **à l'endroit même
//! où l'ancien contrat vivait** — un futur éditeur qui vient chercher
//! « pourquoi les exact matches ne passent plus » y trouve la réponse plutôt
//! qu'une absence.
//!
//! Le rejeu du défaut fondateur et le contrôle négatif « R2 décide ≠ R2 casse
//! la résolution » vivent dans `test_kg_zero_budget_1833.rs`.

use std::sync::Arc;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::kg::subject_extractor::SubjectExtractor;
use mika_common::llm::LlmProvider;
use mika_common::llm::mock::MockLlmProvider;

// ---------------------------------------------------------------------------
// Test DB helpers — scoped to this test file, independent of kg_fixtures
// (we want a clean agent_id and no FTS5 indexing overhead here).
// ---------------------------------------------------------------------------

fn test_db(agent_id: &str) -> AsyncDatabase {
    let db = Database::open_in_memory().unwrap();
    db.register_agent(agent_id, agent_id, "/tmp/mika-test-757")
        .unwrap();
    AsyncDatabase::new_with_agent(db, agent_id)
}

/// Default docs_root used by SubjectExtractor in these tests.
const TEST_DOCS_ROOT: &str = "/tmp/docs";

/// Pre-computed docs_root_hash for TEST_DOCS_ROOT.
fn test_docs_root_hash() -> String {
    mika_agent::kg::config::hash_docs_root(std::path::Path::new(TEST_DOCS_ROOT))
}

async fn insert_chunk(db: &AsyncDatabase, seq_id: i64, doc_path: &str, hash: &str) {
    let doc_path = doc_path.to_owned();
    let hash = hash.to_owned();
    let drh = test_docs_root_hash();
    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            &[
                &drh as &dyn rusqlite::types::ToSql,
                &TEST_DOCS_ROOT,
                &seq_id,
                &doc_path,
                &hash,
            ],
        )?;
        Ok(())
    })
    .await
    .unwrap();
}

async fn insert_extraction(db: &AsyncDatabase, doc_path: &str, hash: Option<&str>) {
    let doc_path = doc_path.to_owned();
    let hash = hash.map(|s| s.to_owned());
    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_extractions \
               (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model, \
                entities_extracted, relationships_extracted, extraction_trace_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 'trace-test')",
            &[
                &test_docs_root_hash() as &dyn rusqlite::types::ToSql,
                &TEST_DOCS_ROOT,
                &doc_path,
                &hash,
                &"mock/test",
            ],
        )?;
        Ok(())
    })
    .await
    .unwrap();
}

async fn count_pending(db: &AsyncDatabase) -> i64 {
    db.with_db(move |db| {
        Ok(db
            .query_scalar::<i64>(
                "SELECT COUNT(*) FROM (
                   SELECT DISTINCT c.source_doc_path
                   FROM kg_chunks c
                   WHERE c.docs_root_hash = ?1
                     AND NOT EXISTS (
                       SELECT 1 FROM kg_extractions e
                       WHERE e.docs_root_hash   = c.docs_root_hash
                         AND e.source_doc_path = c.source_doc_path
                         AND e.source_doc_hash = c.source_doc_hash
                     )
                 )",
                &[&test_docs_root_hash() as &dyn rusqlite::types::ToSql],
            )?
            .unwrap_or(0))
    })
    .await
    .unwrap()
}

async fn read_extraction_hash(db: &AsyncDatabase, doc_path: &str) -> Option<String> {
    let doc_path = doc_path.to_owned();
    db.with_db(move |db| {
        db.query_scalar::<Option<String>>(
            "SELECT source_doc_hash FROM kg_extractions \
             WHERE docs_root_hash = ?1 AND source_doc_path = ?2",
            &[
                &test_docs_root_hash() as &dyn rusqlite::types::ToSql,
                &doc_path,
            ],
        )
        .map(|v| v.flatten())
    })
    .await
    .unwrap()
}

fn mock_llm() -> Arc<MockLlmProvider> {
    // Zero-response mock: safe for tests that should make zero LLM calls.
    // If anything calls into it, MockLlmProvider panics — loud failure, good.
    Arc::new(MockLlmProvider::builder().build())
}

// ---------------------------------------------------------------------------
// Pending-query idempotency semantics (#757 R1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pending_query_treats_doc_without_extraction_row_as_pending() {
    let db = test_db("agent-a");
    insert_chunk(&db, 0, "docs/a.md", "HASH-A").await;
    insert_chunk(&db, 1, "docs/a.md", "HASH-A").await;

    assert_eq!(count_pending(&db).await, 1, "unextracted doc is pending");
}

#[tokio::test]
async fn pending_query_skips_doc_when_hash_matches() {
    let db = test_db("agent-b");
    insert_chunk(&db, 0, "docs/b.md", "HASH-B").await;
    insert_extraction(&db, "docs/b.md", Some("HASH-B")).await;

    assert_eq!(
        count_pending(&db).await,
        0,
        "doc with matching extraction hash is not pending"
    );
}

#[tokio::test]
async fn pending_query_treats_null_hash_as_stale() {
    // First-boot-after-v26 scenario: pre-existing extraction row has NULL hash.
    // The hash-equality predicate rejects NULL, so the doc re-extracts once.
    let db = test_db("agent-c");
    insert_chunk(&db, 0, "docs/c.md", "HASH-C").await;
    insert_extraction(&db, "docs/c.md", None).await;

    assert_eq!(
        count_pending(&db).await,
        1,
        "NULL source_doc_hash must be treated as stale"
    );
}

#[tokio::test]
async fn pending_query_treats_drifted_hash_as_stale() {
    // Lexical ingestor re-ingested the doc with new content; chunk hash is
    // now different from the hash stored in kg_extractions.
    let db = test_db("agent-d");
    insert_chunk(&db, 0, "docs/d.md", "HASH-NEW").await;
    insert_extraction(&db, "docs/d.md", Some("HASH-OLD")).await;

    assert_eq!(
        count_pending(&db).await,
        1,
        "drifted source_doc_hash must re-trigger extraction"
    );
}

// Note on corpus-scope: the pending query filters by `docs_root_hash`, so corpus
// isolation is by construction. Not testing it here would require sharing a
// single Database across two AsyncDatabase handles, which the current
// AsyncDatabase API does not support (it takes ownership of the Database).
// The filter is plainly correct by inspection and is exercised in practice by
// every multi-agent test that touches KG tables.

// ---------------------------------------------------------------------------
// Budget guard (#757 R2)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn extract_pending_with_zero_budget_short_circuits_cleanly() {
    // 3 pending docs, budget=0 → zéro appel LLM. Le contrat de #757 sur ce
    // point tient inchangé ; ce que mika#1833 déplace, c'est **où** le
    // court-circuit a lieu, et les deux assertions ci-dessous sont les deux
    // conséquences observables de ce déplacement.
    let db = test_db("agent-f");
    insert_chunk(&db, 0, "docs/f1.md", "HASH-F1").await;
    insert_chunk(&db, 0, "docs/f2.md", "HASH-F2").await;
    insert_chunk(&db, 0, "docs/f3.md", "HASH-F3").await;

    let llm = mock_llm();
    let extractor_ref = {
        let llm_trait: Arc<dyn LlmProvider> = llm.clone();
        SubjectExtractor::new(db, llm_trait, std::path::PathBuf::from("/tmp/docs"), None)
    };

    let stats = extractor_ref.extract_pending(0).await.unwrap();

    assert_eq!(stats.llm_calls, 0, "zero budget must not consume LLM calls");
    assert_eq!(stats.docs_extracted, 0);
    assert_eq!(
        llm.calls_made(),
        0,
        "MockLlmProvider must never be called when budget=0"
    );

    // mika#1833 — `aborted_budget` reste FAUX. « Épuisé » décrit un budget
    // consommé en route ; une phase que l'opérateur a désarmée n'a rien
    // consommé, et le WARN `kg_budget_exhausted` que l'ancien chemin émettait
    // ici disait donc le contraire de ce qui se passait. C'est aussi le champ
    // qui a fait lire l'incident du 2026-07-26 comme un épuisement.
    assert!(
        !stats.aborted_budget,
        "mika#1833 — un budget nul désarme la phase, il ne l'épuise pas"
    );

    // mika#1833 — et c'est l'assertion porteuse : trois documents sont
    // pending, et `docs_total` vaut malgré tout zéro. Un court-circuit placé
    // APRÈS `get_pending_docs` rendrait 3 ici tout en satisfaisant chacune
    // des assertions ci-dessus ; seule celle-ci distingue les deux
    // placements, et donc atteste que la requête de détection du pending
    // n'est pas payée pour rien. Le contrôle positif de ce silence est la
    // ligne de journal `kg_phase_disabled_by_zero_budget` émise au site du
    // court-circuit — sans elle, « la phase est désarmée » et « la phase n'a
    // pas tourné » rendraient des octets identiques (classe mika#2205).
    assert_eq!(
        stats.docs_total, 0,
        "mika#1833 — aucune requête de pending n'est émise sous budget nul, \
         donc `docs_total` ne peut pas refléter les 3 documents en attente"
    );
}

#[tokio::test]
async fn extract_pending_with_no_pending_docs_makes_zero_calls() {
    // No kg_chunks rows at all — pending query returns empty → early return.
    let db = test_db("agent-g");
    let llm = mock_llm();
    let extractor_ref = {
        let llm_trait: Arc<dyn LlmProvider> = llm.clone();
        SubjectExtractor::new(db, llm_trait, std::path::PathBuf::from("/tmp/docs"), None)
    };

    let stats = extractor_ref.extract_pending(10).await.unwrap();

    assert!(!stats.aborted_budget);
    assert_eq!(stats.llm_calls, 0);
    assert_eq!(stats.docs_total, 0);
    assert_eq!(llm.calls_made(), 0);
}

// Note on happy-path extraction tests: `extract_document` reads the real
// doc from disk via `std::fs::read_to_string`, so a full end-to-end test
// requires tempfile setup. The pending-query, zero-budget, and hash-drift
// tests above cover the load-bearing logic of the fix. End-to-end post-fix
// behavior is verified by the post-deploy signals described in the #757
// PR body (Signal A: extraction not re-running; Signal B: budget not
// exhausted; Signal C: resolver drains; Signal D: concrete cost prediction).

// ---------------------------------------------------------------------------
// Resolver budget guard (#757 Unit 3)
// ---------------------------------------------------------------------------

use mika_agent::kg::entity_resolver::SubjectEntityResolver;

/// Seed a domain entity + a case-matching subject entity so Stage-1 exact
/// match succeeds (no LLM needed). Returns the subject entity row ID.
async fn seed_exact_match_pair(
    db: &AsyncDatabase,
    entity_type: &str,
    name: &str,
    subject_confidence: f64,
) -> i64 {
    let domain_key = format!("{entity_type}:{name}");
    let subject_key = domain_key.clone();
    let entity_type_owned = entity_type.to_owned();
    let name_owned = name.to_owned();

    db.with_db(move |db| {
        // Domain entity
        db.execute_sql(
            "INSERT INTO kg_entities (entity_key, type, name) VALUES (?1, ?2, ?3)",
            &[
                &domain_key as &dyn rusqlite::types::ToSql,
                &entity_type_owned,
                &name_owned,
            ],
        )?;
        // Matching subject entity with high confidence (above exact-match threshold)
        db.execute_sql(
            "INSERT INTO kg_subject_entities \
                (docs_root_hash, docs_root, entity_key, type, name, confidence) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            &[
                &test_docs_root_hash() as &dyn rusqlite::types::ToSql,
                &TEST_DOCS_ROOT,
                &subject_key,
                &entity_type_owned,
                &name_owned,
                &subject_confidence,
            ],
        )?;
        Ok(db.last_insert_rowid())
    })
    .await
    .unwrap()
}

async fn count_resolution_log(db: &AsyncDatabase) -> i64 {
    let agent_id = db.agent_id.clone();
    db.with_db(move |db| {
        Ok(db
            .query_scalar::<i64>(
                "SELECT COUNT(*) FROM kg_resolutions_log WHERE agent_id = ?1",
                &[&agent_id as &dyn rusqlite::types::ToSql],
            )?
            .unwrap_or(0))
    })
    .await
    .unwrap()
}

/// mika#1833 — sous budget nul, **rien** ne résout, exact matches compris.
///
/// Ce test portait le contrat inverse (`..._still_processes_exact_matches`,
/// #757 finding P1) et il est conservé renommé plutôt que supprimé : c'est
/// l'endroit où un éditeur vient chercher « pourquoi les exact matches ne
/// passent plus sous budget nul », et un test supprimé ne dit pas pourquoi.
///
/// Le seeding est **inchangé** — deux exact matches parfaits, aux confiances
/// 0.95 et 0.92, donc tous deux au-dessus du seuil `> 0.9` strict. C'est la
/// population la plus favorable qui existe à l'ancien comportement, et c'est
/// ce qui fait de ce test la mesure exacte du retrait.
///
/// Le contrôle négatif — avec un budget non nul ces deux entités résolvent
/// toujours, donc « R2 décide » n'est pas « R2 casse la résolution » — vit en
/// V2 de `test_kg_zero_budget_1833.rs`, et `..._does_not_starve_...`
/// ci-dessous le rejoue sur ce seeding même.
#[tokio::test]
async fn resolve_pending_with_zero_budget_resolves_nothing_at_all() {
    let db = test_db("agent-rx1");
    seed_exact_match_pair(&db, "skill", "self-dev", 0.95).await;
    seed_exact_match_pair(&db, "tool", "run_gh", 0.92).await;

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec![test_docs_root_hash()],
        Some("trace-rx1"),
    );
    let stats = resolver.resolve_pending(0).await.unwrap();

    assert_eq!(
        stats.matched_exact, 0,
        "mika#1833 — la phase est désarmée avant la sélection, donc aucun \
         exact match n'est même examiné"
    );
    assert_eq!(stats.llm_calls, 0);
    assert!(
        !stats.aborted_budget,
        "un budget nul désarme la phase, il ne l'épuise pas"
    );

    // L'assertion porteuse, et elle est porteuse dans les DEUX sens. Avant
    // mika#1833 ce compte valait 2 ; après, zéro — mais surtout, zéro ligne
    // écrite est ce qui rend le no-op sûr : la tête de file n'est pas
    // consommée, donc rien n'est marqué résolu qui ne l'a pas été (le défaut
    // « la colonne devient pleine et reste muette », refusé en D3 du plan).
    assert_eq!(
        count_resolution_log(&db).await,
        0,
        "mika#1833 — aucune ligne `kg_resolutions_log` : la phase n'a pas tourné"
    );
    assert_eq!(
        stats.total, 0,
        "mika#1833 — `total` compte les entités sélectionnées, et la \
         sélection n'a pas eu lieu"
    );
}

#[tokio::test]
async fn resolve_pending_with_no_llm_and_no_exact_match_skips_cleanly() {
    // With no LLM configured and no exact match, entities get SkippedNoLlm
    // (not SkippedBudget). The budget does not fire because the code path
    // exits at Stage-2-entry with SkippedNoLlm before the budget guard.
    let db = test_db("agent-rx2");
    // Subject entity with no matching domain entity
    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_subject_entities \
                (docs_root_hash, docs_root, entity_key, type, name, confidence) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            &[
                &test_docs_root_hash() as &dyn rusqlite::types::ToSql,
                &TEST_DOCS_ROOT,
                &"skill:does-not-exist",
                &"skill",
                &"does-not-exist",
                &0.7f64,
            ],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec![test_docs_root_hash()],
        Some("trace-rx2"),
    );
    let stats = resolver.resolve_pending(10).await.unwrap();

    assert_eq!(stats.skipped_no_llm, 1);
    assert_eq!(stats.llm_calls, 0);
    assert!(!stats.aborted_budget);
}

#[tokio::test]
async fn resolve_pending_empty_set_returns_zero_calls() {
    // No kg_subject_entities → pending query returns empty → early return.
    let db = test_db("agent-rx3");

    let resolver =
        SubjectEntityResolver::new(db, None, vec![test_docs_root_hash()], Some("trace-rx3"));
    let stats = resolver.resolve_pending(10).await.unwrap();

    assert_eq!(stats.total, 0);
    assert_eq!(stats.llm_calls, 0);
    assert!(!stats.aborted_budget);
}

#[tokio::test]
async fn resolve_pending_budget_exhaustion_does_not_starve_later_exact_matches() {
    // Regression test for the #757 code review P1 finding: the resolver
    // must continue past a skipped entity so that later Stage-1 exact
    // matches still resolve. The previous `break`-on-first-skip starved
    // every entity behind the first one that could not resolve. This test
    // seeds a 3-entity batch where the MIDDLE entity would need Stage-2 (no
    // matching domain) and the bookending entities are exact matches.
    //
    // mika#1833 — ce test tournait sous `resolve_pending(0)` et tourne
    // désormais sous un budget non nul. **La propriété testée ne change
    // pas** : la non-starvation est une propriété de la BOUCLE (elle
    // `continue` au lieu de `break`), pas du budget, et le commentaire
    // d'origine le disait déjà en toutes lettres — « budget=0 is irrelevant
    // […] the test's value is not in this particular variant but in the
    // ordering ». Sous budget nul la boucle n'est plus atteinte du tout, donc
    // le test n'aurait plus gardé le finding P1 : il aurait gardé le
    // court-circuit, que trois autres tests gardent déjà. Le budget est pris
    // large (10 contre 3 entités) pour que l'épuisement ne soit jamais le
    // sujet — ce que ces trois lignes mesurent est l'ordre, et rien d'autre.
    let db = test_db("agent-rx5");
    seed_exact_match_pair(&db, "skill", "self-dev", 0.95).await;

    // A low-confidence subject entity with no matching domain entity —
    // Stage-1 fails, Stage-2 would fire but the resolver has no LLM
    // configured, producing SkippedNoLlm. The test's value is not in
    // this particular variant but in the ordering: the 3rd entity below
    // (an exact match) MUST still resolve.
    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_subject_entities \
                (docs_root_hash, docs_root, entity_key, type, name, confidence) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            &[
                &test_docs_root_hash() as &dyn rusqlite::types::ToSql,
                &TEST_DOCS_ROOT,
                &"skill:mystery-skill",
                &"skill",
                &"mystery-skill",
                &0.7f64,
            ],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    seed_exact_match_pair(&db, "tool", "run_gh", 0.92).await;

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec![test_docs_root_hash()],
        Some("trace-rx5"),
    );
    let stats = resolver.resolve_pending(10).await.unwrap();

    // Both exact matches must resolve — the middle SkippedNoLlm entity
    // does not starve the third entity.
    assert_eq!(stats.matched_exact, 2);
    assert_eq!(stats.skipped_no_llm, 1);
    assert_eq!(stats.llm_calls, 0);
    assert_eq!(
        count_resolution_log(&db).await,
        3,
        "all 3 entities should have kg_resolutions_log rows (2 MATCHED_EXACT + 1 SKIPPED_NO_LLM)"
    );
}

#[tokio::test]
async fn null_hash_populates_on_successful_extraction() {
    // The #757 plan claims the first post-v26 boot re-extracts NULL-hash
    // rows once, then the hash is populated so subsequent runs are no-ops.
    // This test verifies the two-phase contract WITHOUT invoking
    // extract_document (which reads from disk): we simulate a successful
    // extraction by writing the kg_extractions row directly with the
    // hash, then assert the pending count drops to 0.
    let db = test_db("agent-nh1");
    insert_chunk(&db, 0, "docs/nh.md", "HASH-NH").await;
    insert_extraction(&db, "docs/nh.md", None).await;

    // Phase 1: pre-extraction state — NULL hash, pending=1.
    assert_eq!(count_pending(&db).await, 1);
    assert_eq!(read_extraction_hash(&db, "docs/nh.md").await, None);

    // Phase 2: simulate a successful extraction by writing the hash.
    // This is the UPSERT the production extract_document runs inside
    // its transaction (#757 atomic marker write).
    db.with_db(move |db| {
        db.execute_sql(
            "UPDATE kg_extractions SET source_doc_hash = ?1 \
             WHERE docs_root_hash = ?2 AND source_doc_path = ?3",
            &[
                &"HASH-NH" as &dyn rusqlite::types::ToSql,
                &test_docs_root_hash(),
                &"docs/nh.md",
            ],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    // Phase 3: post-extraction — hash populated, pending=0.
    assert_eq!(
        read_extraction_hash(&db, "docs/nh.md").await,
        Some("HASH-NH".to_string())
    );
    assert_eq!(
        count_pending(&db).await,
        0,
        "after hash is populated, doc must not be pending again"
    );
}

#[tokio::test]
async fn resolve_pending_mixed_batch_logs_exact_and_no_llm() {
    // Mix exact-match and potential LLM-entity in one batch: the exact match
    // resolves in Stage-1, and the LLM-entity never gets to Stage-2 because
    // there's no LLM configured (SubjectEntityResolver built with None), so
    // SkippedNoLlm fires. The test documents that composition.
    //
    // mika#1833 — renommé (il s'appelait `..._zero_budget_...`) et rejoué
    // sous un budget non nul. Le budget était déjà **orthogonal** à ce que ce
    // test mesure, et son propre commentaire le disait ; ce qui a changé est
    // qu'un budget nul ne laisse plus rien atteindre Stage-1, donc le test
    // aurait cessé de mesurer la composition. Le nom perd `zero_budget`
    // plutôt que de le garder : un nom qui annonce un budget nul alors que le
    // test en passe un autre est la forme de test qu'on ne relit plus.
    let db = test_db("agent-rx4");
    seed_exact_match_pair(&db, "skill", "self-dev", 0.95).await;
    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_subject_entities \
                (docs_root_hash, docs_root, entity_key, type, name, confidence) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            &[
                &test_docs_root_hash() as &dyn rusqlite::types::ToSql,
                &TEST_DOCS_ROOT,
                &"skill:unknown-skill",
                &"skill",
                &"unknown-skill",
                &0.7f64,
            ],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    let resolver =
        SubjectEntityResolver::new(db, None, vec![test_docs_root_hash()], Some("trace-rx4"));
    let stats = resolver.resolve_pending(10).await.unwrap();

    assert_eq!(stats.matched_exact, 1);
    assert_eq!(stats.skipped_no_llm, 1);
    assert_eq!(stats.llm_calls, 0);
    assert!(
        !stats.aborted_budget,
        "no LLM configured → SkippedNoLlm never triggers budget abort"
    );
}

// Note: we do not test Stage-2 budget exhaustion with a mock LLM here because
// the resolver's LLM prompt interface expects a specific JSON response shape
// (validated in `entity_resolver::tests`). Stage-2 budget behavior is covered
// by the structural contract of `resolve_single_entity(entity, llm_call_allowed=false)`
// returning `SkippedBudget` without touching `self.llm` — this contract is
// verified at the module boundary in entity_resolver.rs unit tests. Post-deploy
// Signal B (kg_budget_exhausted WARN does not appear on healthy restarts)
// covers the real-world integration path.
