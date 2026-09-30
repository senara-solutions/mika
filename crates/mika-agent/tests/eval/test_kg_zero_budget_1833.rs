//! mika#1833 — `MIKA_KG_BATCH_BUDGET=0` désactive réellement la phase.
//!
//! # Le défaut rejoué ici
//!
//! Mesuré le 2026-07-26, sur chaque agent bien connu et à chaque
//! `kg_resolver_tick` :
//!
//! ```text
//! pending: 1288-1493  resolved_in_tick: 0  duration_ms: 26000-48000
//! aborted_budget: true  llm_calls: 0
//! ```
//!
//! Le couple `aborted_budget: true` avec `llm_calls: 0` n'a qu'une cause
//! possible : `llm_call_allowed = stats.llm_calls < budget` faux à zéro appel,
//! donc `budget == 0`. Le ticket a lu le symptôme comme un graphe de domaine
//! vide ; c'était un **interblocage déterministe à budget nul**, et le graphe
//! n'a pas besoin d'être vide pour le produire.
//!
//! La chaîne, et aucun maillon n'est fautif isolément : un exact match de
//! Stage-1 à la confiance **modale 0.9** ne passe pas le seuil `> 0.9`
//! **strict**, donc il escalade en Stage-2 ; sous budget nul le Stage-2 rend
//! `SkippedBudget` ; `SkippedBudget` fait `continue` **sans appeler
//! `apply_result`**, donc sans écrire de ligne `kg_resolutions_log` ; et la
//! sélection est `ORDER BY e.id ASC LIMIT 50`, donc la même tête de file
//! revient au tick suivant. Indéfiniment, toutes les 30 minutes, par agent.
//!
//! # Ce que ces trois tests séparent
//!
//! V1 atteste que la phase est un no-op **avant toute requête** — et le prouve
//! par construction plutôt que par comptage. V2 est le contrôle négatif sans
//! lequel « R2 décide » serait indistinguable de « R2 casse la résolution ».
//! V3 rejoue la forme exacte du défaut fondateur.

use super::kg_fixtures::*;
use mika_agent::kg::entity_resolver::SubjectEntityResolver;

/// Rend la table des sujets illisible, pour que toute requête de sélection
/// **échoue** au lieu de rendre zéro ligne.
///
/// C'est ce qui fait de V1 une preuve par construction : avec la table en
/// place, « aucune entité sélectionnée » et « aucune requête émise » rendent
/// le même `ResolutionStats::default()` et ne se distinguent pas. Sans la
/// table, la seule façon de rendre `Ok` est de n'avoir rien demandé.
async fn drop_the_pending_source(db: &mika_agent::async_db::AsyncDatabase) {
    db.with_db(|db| {
        db.execute_sql("DROP TABLE kg_subject_entities", &[])?;
        Ok(())
    })
    .await
    .expect("drop must succeed");
}

fn resolver(db: &mika_agent::async_db::AsyncDatabase) -> SubjectEntityResolver {
    SubjectEntityResolver::new(
        db.clone(),
        None,
        vec![TEST_DOCS_ROOT_HASH.to_string()],
        Some("test-trace-1833"),
    )
}

/// V1 — sous budget nul, la résolution rend `default()` **sans avoir émis la
/// moindre requête** (mika#1833 R2).
///
/// Le coût mesuré de 26-48 s par tick n'était pas le travail de résolution —
/// sous budget nul il n'y a aucun appel LLM. C'était la sous-requête corrélée
/// de détection du pending, payée trois fois par tick pour un agent
/// mono-corpus et dix-huit fois pour mika-arch. Court-circuiter **après** la
/// requête aurait laissé ce coût entier.
#[tokio::test]
async fn mika1833_budget_zero_resolution_is_a_declared_noop() {
    let db = test_db();
    assert_schema_version(&db).await;
    drop_the_pending_source(&db).await;

    let stats = resolver(&db).resolve_pending(0).await.expect(
        "un budget nul ne doit émettre AUCUNE requête — s'il en émet une, \
                 elle échoue sur la table absente et ce `expect` explose",
    );

    assert_eq!(stats.llm_calls, 0);
    assert_eq!(stats.matched_exact, 0);
    assert_eq!(stats.matched_llm, 0);
    assert!(
        !stats.aborted_budget,
        "`aborted_budget` décrit un budget consommé en route, pas une phase \
         que l'opérateur a désarmée — et c'est le champ qui a fait lire \
         l'incident comme un épuisement"
    );

    // Le contrôle qui rend le `expect` ci-dessus signifiant : avec un budget
    // non nul, la requête EST émise, donc elle échoue sur la table absente.
    // Sans ce second volet, un `resolve_pending` totalement inerte passerait.
    let err = resolver(&db).resolve_pending(5).await;
    assert!(
        err.is_err(),
        "un budget non nul doit interroger la base — sinon le test ci-dessus \
         ne prouve rien sur l'absence de requête"
    );
}

/// V2 — contrôle négatif : un budget non nul résout toujours (mika#1833, AC8).
///
/// C'est lui qui distingue « R2 décide » de « R2 casse la résolution ». La
/// confiance est 0.95, donc au-dessus du seuil `> 0.9` : l'entité résout en
/// Stage-1, sans LLM.
#[tokio::test]
async fn mika1833_a_nonzero_budget_still_resolves() {
    let db = test_db();
    assert_schema_version(&db).await;

    let domain_id = seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            properties_json: None,
        },
    )
    .await;

    let subject_id = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            confidence: 0.95,
            properties_json: None,
        },
    )
    .await;

    let chunk_id = seed_chunk(
        &db,
        &ChunkSpec {
            seq_id: 0,
            source_doc_path: "docs/solutions/x.md",
            source_doc_hash: "hash-1833-v2",
            text: "the self-dev skill handles autonomous implementation",
        },
    )
    .await;
    seed_chunk_subject(&db, chunk_id, subject_id).await;

    let stats = resolver(&db)
        .resolve_pending(5)
        .await
        .expect("resolution must run");

    assert_eq!(
        stats.matched_exact, 1,
        "un exact match au-dessus du seuil doit résoudre en Stage-1"
    );
    assert_eq!(stats.llm_calls, 0, "…et sans appel LLM");
    assert!(!stats.aborted_budget);

    let resolved = get_resolution(&db, subject_id).await;
    assert_eq!(
        resolved.map(|(id, _)| id),
        Some(domain_id),
        "l'arête sujet → domaine doit être écrite"
    );
}

/// V3 — le rejeu du défaut fondateur : l'interblocage ne peut plus se produire
/// (mika#1833, AC1).
///
/// **Le test qui échouerait avant le correctif.** La forme est celle qui a été
/// mesurée : budget nul, et trois sujets dont un exact match à la confiance
/// **modale 0.9** — celle qu'un LLM émet pour une extraction confiante, et que
/// le seuil `> 0.9` **strict** rejette d'un cheveu.
///
/// Avant mika#1833, ce tour rendait `aborted_budget: true` et
/// `resolved_in_tick: 0`, n'écrivait **aucune** ligne `kg_resolutions_log`, et
/// laissait la même tête de file re-sélectionnable au tick suivant. Après, la
/// phase est un no-op déclaré : rien n'est sélectionné, donc rien n'est jeté.
#[tokio::test]
async fn mika1833_the_founding_livelock_no_longer_recurs() {
    let db = test_db();
    assert_schema_version(&db).await;

    seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            properties_json: None,
        },
    )
    .await;

    // Le sujet qui produit l'interblocage : exact match parfait, confiance
    // modale, donc escalade en Stage-2 sous l'ancien seuil strict.
    let modal = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            confidence: 0.9,
            properties_json: None,
        },
    )
    .await;
    // Deux sujets sans contrepartie de domaine — la population qui reste
    // `SkippedBudget` quoi qu'il arrive.
    let orphan_a = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "skill",
            name: "ghost-alpha",
            confidence: 0.8,
            properties_json: None,
        },
    )
    .await;
    let orphan_b = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "tool",
            name: "ghost-beta",
            confidence: 0.85,
            properties_json: None,
        },
    )
    .await;

    let chunk_id = seed_chunk(
        &db,
        &ChunkSpec {
            seq_id: 0,
            source_doc_path: "docs/solutions/livelock.md",
            source_doc_hash: "hash-1833-livelock",
            text: "self-dev, ghost-alpha and ghost-beta appear in this chunk",
        },
    )
    .await;
    for subject in [modal, orphan_a, orphan_b] {
        seed_chunk_subject(&db, chunk_id, subject).await;
    }

    // Un modèle de résolution EST configuré — c'est la condition qui rendait
    // `SkippedNoLlm` (qui, lui, draine) inatteignable, et donc l'interblocage
    // possible. Ici il n'a pas à être appelé : la phase ne démarre pas.
    let stats = resolver(&db)
        .resolve_pending(0)
        .await
        .expect("a disabled phase must not error");

    assert!(
        !stats.aborted_budget,
        "mika#1833 — c'est l'assertion qui échoue avant le correctif : \
         l'ancien chemin sélectionnait 50 entités, escaladait celle à 0.9 en \
         Stage-2 et posait `aborted_budget = true` sans rien écrire"
    );
    assert_eq!(stats.llm_calls, 0);

    // Et surtout : aucune entité n'a été touchée, donc aucune n'est laissée
    // dans un état que le tick suivant re-sélectionnerait pour rien.
    for subject in [modal, orphan_a, orphan_b] {
        assert!(
            get_resolution_log(&db, subject).await.is_none(),
            "aucune ligne de log ne doit être écrite : la phase n'a pas tourné"
        );
        assert!(
            get_resolution(&db, subject).await.is_none(),
            "…et aucune arête non plus"
        );
    }

    // Le contrôle négatif de V3 : la MÊME base, budget non nul. L'exact match
    // à 0.9 escalade toujours en Stage-2 (le seuil strict est hors périmètre,
    // D1), mais la file **avance** — ce qui est la différence entre un budget
    // serré et un interblocage.
    let stats = resolver(&db)
        .resolve_pending(5)
        .await
        .expect("resolution must run");
    assert!(
        stats.skipped_no_llm > 0,
        "sans modèle de résolution configuré, les entités drainent en \
         `skipped_no_llm` — c'est le levier terminant que R2 nomme, et il \
         écrit des lignes, donc la file se vide"
    );
    assert!(
        get_resolution_log(&db, modal).await.is_some(),
        "…et la tête de file ne revient pas au tick suivant"
    );
}
