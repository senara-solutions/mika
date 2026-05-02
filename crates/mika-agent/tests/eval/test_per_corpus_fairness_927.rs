//! # Per-corpus fairness in entity resolver (#927)
//!
//! Regression tests verifying that `resolve_pending(budget)` distributes
//! Stage-2 attempts fairly across corpora via round-robin interleaving.
//!
//! **Acceptance criteria covered:**
//! - AC #1/R1: non-zero attempts on every corpus that has pending entities.
//! - AC #3/R3: 4 corpora (1000, 50, 50, 50), budget=200, each ≥25 attempts.
//! - AC #5/R5: mika-platform + mika-cloud secondaries get >0 attempts.
//! - F3 edge cases: empty corpus, single-corpus regression, under-allocation.

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::kg::entity_resolver::SubjectEntityResolver;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const AGENT_ID: &str = "test-agent-927";

const CORPUS_PRIMARY: &str = "aaaa000000000001";
const CORPUS_SECONDARY_A: &str = "bbbb000000000002";
const CORPUS_SECONDARY_B: &str = "cccc000000000003";
const CORPUS_SECONDARY_C: &str = "dddd000000000004";

const DOCS_ROOT_PRIMARY: &str = "/test/mika/docs/solutions";
const DOCS_ROOT_SECONDARY_A: &str = "/test/mika-skills/docs/solutions";
const DOCS_ROOT_SECONDARY_B: &str = "/test/mika-platform/docs/solutions";
const DOCS_ROOT_SECONDARY_C: &str = "/test/mika-cloud/docs/solutions";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().unwrap();
    db.register_agent(AGENT_ID, AGENT_ID, "/tmp/mika-test-927")
        .unwrap();
    db.create_session("test-session-927", AGENT_ID, "test")
        .unwrap();
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// Register a corpus mapping for the test agent.
async fn register_corpus(db: &AsyncDatabase, docs_root_hash: &str, docs_root: &str) {
    let agent_id = AGENT_ID.to_string();
    let hash = docs_root_hash.to_string();
    let path = docs_root.to_string();
    db.with_db(move |db| db.register_agent_corpus(&agent_id, &hash, &path))
        .await
        .unwrap();
}

/// Seed `count` pending subject entities for a given corpus.
/// Entity names are `{prefix}-{i}` to ensure uniqueness across corpora.
async fn seed_pending_entities(
    db: &AsyncDatabase,
    docs_root_hash: &str,
    docs_root: &str,
    prefix: &str,
    count: usize,
) {
    let hash = docs_root_hash.to_string();
    let root = docs_root.to_string();
    let prefix = prefix.to_string();
    db.with_db(move |db| {
        for i in 0..count {
            let name = format!("{prefix}-{i}");
            let entity_key = format!("skill:{name}");
            db.execute_sql(
                "INSERT INTO kg_subject_entities \
                 (docs_root_hash, docs_root, entity_key, type, name, confidence) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                &[
                    &hash as &dyn rusqlite::types::ToSql,
                    &root,
                    &entity_key,
                    &"skill",
                    &name,
                    &0.95,
                ],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// AC #1, AC #3, R1, R3: 4 corpora (1000, 50, 50, 50), budget=200.
/// Each corpus contributes ≥25 attempted entities.
#[tokio::test]
async fn test_four_corpora_fairness() {
    let db = test_db();
    register_corpus(&db, CORPUS_PRIMARY, DOCS_ROOT_PRIMARY).await;
    register_corpus(&db, CORPUS_SECONDARY_A, DOCS_ROOT_SECONDARY_A).await;
    register_corpus(&db, CORPUS_SECONDARY_B, DOCS_ROOT_SECONDARY_B).await;
    register_corpus(&db, CORPUS_SECONDARY_C, DOCS_ROOT_SECONDARY_C).await;

    seed_pending_entities(&db, CORPUS_PRIMARY, DOCS_ROOT_PRIMARY, "primary", 1000).await;
    seed_pending_entities(&db, CORPUS_SECONDARY_A, DOCS_ROOT_SECONDARY_A, "sec-a", 50).await;
    seed_pending_entities(&db, CORPUS_SECONDARY_B, DOCS_ROOT_SECONDARY_B, "sec-b", 50).await;
    seed_pending_entities(&db, CORPUS_SECONDARY_C, DOCS_ROOT_SECONDARY_C, "sec-c", 50).await;

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None, // exact-match-only — no LLM
        vec![
            CORPUS_PRIMARY.to_string(),
            CORPUS_SECONDARY_A.to_string(),
            CORPUS_SECONDARY_B.to_string(),
            CORPUS_SECONDARY_C.to_string(),
        ],
        Some("test-fairness-927"),
    );

    let stats = resolver.resolve_pending(200).await.unwrap();

    // Every corpus should have been attempted.
    assert_eq!(
        stats.per_corpus_attempted.len(),
        4,
        "all 4 corpora should have entries in per_corpus_attempted"
    );

    for (hash, label) in [
        (CORPUS_PRIMARY, "primary"),
        (CORPUS_SECONDARY_A, "secondary-a"),
        (CORPUS_SECONDARY_B, "secondary-b"),
        (CORPUS_SECONDARY_C, "secondary-c"),
    ] {
        let attempted = stats.per_corpus_attempted.get(hash).copied().unwrap_or(0);
        assert!(
            attempted >= 25,
            "corpus {label} ({hash}) attempted {attempted}, expected ≥25"
        );
    }

    // Total attempted should be reasonable (not more than budget * oversupply).
    let total_attempted: u32 = stats.per_corpus_attempted.values().sum();
    assert!(
        total_attempted >= 200,
        "total attempted {total_attempted} should be ≥200 (budget)"
    );
}

/// AC #5, R5: secondary corpora (mika-platform, mika-cloud) get >0 attempts.
#[tokio::test]
async fn test_secondaries_get_attempts() {
    let db = test_db();
    register_corpus(&db, CORPUS_PRIMARY, DOCS_ROOT_PRIMARY).await;
    register_corpus(&db, CORPUS_SECONDARY_A, DOCS_ROOT_SECONDARY_A).await;
    register_corpus(&db, CORPUS_SECONDARY_B, DOCS_ROOT_SECONDARY_B).await;
    register_corpus(&db, CORPUS_SECONDARY_C, DOCS_ROOT_SECONDARY_C).await;

    // Mimic real production ratios: large primary, tiny secondaries.
    seed_pending_entities(&db, CORPUS_PRIMARY, DOCS_ROOT_PRIMARY, "pri", 5000).await;
    seed_pending_entities(&db, CORPUS_SECONDARY_A, DOCS_ROOT_SECONDARY_A, "sa", 30).await;
    seed_pending_entities(&db, CORPUS_SECONDARY_B, DOCS_ROOT_SECONDARY_B, "sb", 20).await;
    seed_pending_entities(&db, CORPUS_SECONDARY_C, DOCS_ROOT_SECONDARY_C, "sc", 15).await;

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec![
            CORPUS_PRIMARY.to_string(),
            CORPUS_SECONDARY_A.to_string(),
            CORPUS_SECONDARY_B.to_string(),
            CORPUS_SECONDARY_C.to_string(),
        ],
        Some("test-secondaries-927"),
    );

    let stats = resolver.resolve_pending(500).await.unwrap();

    for (hash, label) in [
        (CORPUS_SECONDARY_A, "mika-skills"),
        (CORPUS_SECONDARY_B, "mika-platform"),
        (CORPUS_SECONDARY_C, "mika-cloud"),
    ] {
        let attempted = stats.per_corpus_attempted.get(hash).copied().unwrap_or(0);
        assert!(
            attempted > 0,
            "secondary corpus {label} ({hash}) should have >0 attempts, got {attempted}"
        );
    }
}

/// F3 edge case: single-corpus regression — no fairness overhead for the common case.
#[tokio::test]
async fn test_single_corpus_regression() {
    let db = test_db();
    register_corpus(&db, CORPUS_PRIMARY, DOCS_ROOT_PRIMARY).await;

    seed_pending_entities(&db, CORPUS_PRIMARY, DOCS_ROOT_PRIMARY, "solo", 1000).await;

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec![CORPUS_PRIMARY.to_string()],
        Some("test-single-corpus-927"),
    );

    let stats = resolver.resolve_pending(200).await.unwrap();

    let attempted = stats
        .per_corpus_attempted
        .get(CORPUS_PRIMARY)
        .copied()
        .unwrap_or(0);
    assert!(
        attempted >= 200,
        "single corpus should get full budget utilization, got {attempted}"
    );
    assert_eq!(
        stats.per_corpus_attempted.len(),
        1,
        "only one corpus should be in the map"
    );
}

/// F3 edge case: empty corpus — 4 corpora, corpus #2 has 0 pending.
/// Corpus #2 attempts=0; others get redistributed budget.
#[tokio::test]
async fn test_empty_corpus_skipped() {
    let db = test_db();
    register_corpus(&db, CORPUS_PRIMARY, DOCS_ROOT_PRIMARY).await;
    register_corpus(&db, CORPUS_SECONDARY_A, DOCS_ROOT_SECONDARY_A).await;
    register_corpus(&db, CORPUS_SECONDARY_B, DOCS_ROOT_SECONDARY_B).await;
    register_corpus(&db, CORPUS_SECONDARY_C, DOCS_ROOT_SECONDARY_C).await;

    seed_pending_entities(&db, CORPUS_PRIMARY, DOCS_ROOT_PRIMARY, "pri", 1000).await;
    // CORPUS_SECONDARY_A: 0 pending (no entities seeded)
    seed_pending_entities(&db, CORPUS_SECONDARY_B, DOCS_ROOT_SECONDARY_B, "sb", 50).await;
    seed_pending_entities(&db, CORPUS_SECONDARY_C, DOCS_ROOT_SECONDARY_C, "sc", 50).await;

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec![
            CORPUS_PRIMARY.to_string(),
            CORPUS_SECONDARY_A.to_string(),
            CORPUS_SECONDARY_B.to_string(),
            CORPUS_SECONDARY_C.to_string(),
        ],
        Some("test-empty-corpus-927"),
    );

    let stats = resolver.resolve_pending(200).await.unwrap();

    // Empty corpus should have 0 or not present in the map.
    let empty_attempted = stats
        .per_corpus_attempted
        .get(CORPUS_SECONDARY_A)
        .copied()
        .unwrap_or(0);
    assert_eq!(
        empty_attempted, 0,
        "empty corpus should have 0 attempts, got {empty_attempted}"
    );

    // Other corpora should still get attempts.
    for (hash, label) in [
        (CORPUS_PRIMARY, "primary"),
        (CORPUS_SECONDARY_B, "secondary-b"),
        (CORPUS_SECONDARY_C, "secondary-c"),
    ] {
        let attempted = stats.per_corpus_attempted.get(hash).copied().unwrap_or(0);
        assert!(
            attempted > 0,
            "corpus {label} should have >0 attempts, got {attempted}"
        );
    }
}

/// F3 edge case: under-allocation — 4 corpora (500, 10, 10, 10), budget=200.
/// Corpora #2/#3/#4 each have only 10 pending — under-allocated.
/// They should each attempt exactly 10; primary gets the redistributed remainder.
#[tokio::test]
async fn test_under_allocation_redistribution() {
    let db = test_db();
    register_corpus(&db, CORPUS_PRIMARY, DOCS_ROOT_PRIMARY).await;
    register_corpus(&db, CORPUS_SECONDARY_A, DOCS_ROOT_SECONDARY_A).await;
    register_corpus(&db, CORPUS_SECONDARY_B, DOCS_ROOT_SECONDARY_B).await;
    register_corpus(&db, CORPUS_SECONDARY_C, DOCS_ROOT_SECONDARY_C).await;

    seed_pending_entities(&db, CORPUS_PRIMARY, DOCS_ROOT_PRIMARY, "pri", 500).await;
    seed_pending_entities(&db, CORPUS_SECONDARY_A, DOCS_ROOT_SECONDARY_A, "sa", 10).await;
    seed_pending_entities(&db, CORPUS_SECONDARY_B, DOCS_ROOT_SECONDARY_B, "sb", 10).await;
    seed_pending_entities(&db, CORPUS_SECONDARY_C, DOCS_ROOT_SECONDARY_C, "sc", 10).await;

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec![
            CORPUS_PRIMARY.to_string(),
            CORPUS_SECONDARY_A.to_string(),
            CORPUS_SECONDARY_B.to_string(),
            CORPUS_SECONDARY_C.to_string(),
        ],
        Some("test-under-alloc-927"),
    );

    let stats = resolver.resolve_pending(200).await.unwrap();

    // Each small corpus should attempt all 10 of its pending entities.
    for (hash, label) in [
        (CORPUS_SECONDARY_A, "secondary-a"),
        (CORPUS_SECONDARY_B, "secondary-b"),
        (CORPUS_SECONDARY_C, "secondary-c"),
    ] {
        let attempted = stats.per_corpus_attempted.get(hash).copied().unwrap_or(0);
        assert_eq!(
            attempted, 10,
            "under-allocated corpus {label} should attempt all 10, got {attempted}"
        );
    }

    // Primary should get the redistributed remainder (170 = 200 - 30).
    let primary_attempted = stats
        .per_corpus_attempted
        .get(CORPUS_PRIMARY)
        .copied()
        .unwrap_or(0);
    assert!(
        primary_attempted >= 100,
        "primary should get redistributed budget, got {primary_attempted}"
    );
}

/// Zero-corpus edge case — empty docs_root_hashes returns empty stats.
#[tokio::test]
async fn test_zero_corpora_returns_empty() {
    let db = test_db();

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec![], // no corpora
        Some("test-zero-corpora-927"),
    );

    let stats = resolver.resolve_pending(200).await.unwrap();

    assert_eq!(stats.total, 0, "zero corpora should yield empty stats");
    assert!(
        stats.per_corpus_attempted.is_empty(),
        "zero corpora should have empty per_corpus_attempted"
    );
}
