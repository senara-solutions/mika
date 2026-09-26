//! # Multi-agent corpus parity regression guard (#1155)
//!
//! Integration test that asserts mika-arch-style multi-corpus agents achieve
//! `search_content` index parity with single-corpus peer agents for a shared
//! docs corpus.
//!
//! **Acceptance criteria covered:**
//! - AC: mika-arch `search_content` row count reaches parity with peer agents
//!   on the shared corpus.
//! - AC: Regression guard: parity integration test for known multi-corpus scenario.
//!
//! The test reproduces the original bug: without the backfill fix, agent B
//! (multi-corpus, shared + unique dirs) would have fewer `search_content` rows
//! than agent A (single-corpus) for the shared corpus.

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::db::kg_schema::KG_CHUNK_SOURCE_TYPE;
use mika_agent::kg::config::hash_docs_root;
use mika_agent::kg::lexical_ingestor::LexicalIngestor;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const AGENT_SINGLE: &str = "agent-single-corpus";
const AGENT_MULTI: &str = "agent-multi-corpus";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a temporary docs directory with N markdown files.
fn setup_docs_dir(base: &std::path::Path, name: &str, doc_count: usize) -> std::path::PathBuf {
    let dir = base.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..doc_count {
        let content = format!(
            "---\ntitle: Doc {i}\ntags: [test]\n---\n\n## Section A\n\n\
             Content for document {i}, section A. This has enough words to form a chunk.\n\n\
             ## Section B\n\n\
             Content for document {i}, section B. Additional content for chunking.\n\n\
             ## Section C\n\n\
             Content for document {i}, section C. Even more content to ensure multiple chunks.\n"
        );
        std::fs::write(dir.join(format!("doc-{i}.md")), content).unwrap();
    }
    dir
}

/// Count search_content rows for an agent filtered by docs_root_hash.
async fn count_search_content_for_corpus(
    db: &AsyncDatabase,
    agent_id: &str,
    docs_root_hash: &str,
) -> usize {
    let agent_id = agent_id.to_owned();
    let hash = docs_root_hash.to_owned();
    db.with_db(move |db| {
        let count: Option<i64> = db.query_scalar(
            "SELECT COUNT(*) FROM search_content sc \
             WHERE sc.agent_id = ?1 \
               AND sc.source_type = ?2 \
               AND sc.source_id IN (SELECT id FROM kg_chunks WHERE docs_root_hash = ?3)",
            &[
                &agent_id as &dyn rusqlite::types::ToSql,
                &KG_CHUNK_SOURCE_TYPE,
                &hash,
            ],
        )?;
        Ok(count.unwrap_or(0) as usize)
    })
    .await
    .unwrap()
}

/// Count total search_content rows for an agent (all corpora).
async fn count_search_content_total(db: &AsyncDatabase, agent_id: &str) -> usize {
    let agent_id = agent_id.to_owned();
    db.with_db(move |db| {
        let count: Option<i64> = db.query_scalar(
            "SELECT COUNT(*) FROM search_content WHERE agent_id = ?1 AND source_type = ?2",
            &[
                &agent_id as &dyn rusqlite::types::ToSql,
                &KG_CHUNK_SOURCE_TYPE,
            ],
        )?;
        Ok(count.unwrap_or(0) as usize)
    })
    .await
    .unwrap()
}

/// Ensure an agent exists in the agents table.
async fn ensure_agent(db: &AsyncDatabase, agent_id: &str) {
    let agent_id = agent_id.to_owned();
    db.with_db(move |db| {
        db.execute_sql(
            "INSERT OR IGNORE INTO agents (id, name, home_dir) VALUES (?1, ?1, '')",
            &[&agent_id as &dyn rusqlite::types::ToSql],
        )?;
        Ok(())
    })
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Core parity test: a multi-corpus agent achieves the same search_content
/// count as a single-corpus agent for the shared docs_root.
///
/// Reproduces the #1155 bug scenario:
/// - Agent A (single-corpus): ingests only shared_docs/
/// - Agent B (multi-corpus): ingests shared_docs/ AND unique_docs/
///
/// Post-fix: both agents must have identical search_content row counts for
/// the shared corpus.
#[tokio::test]
async fn multi_corpus_agent_achieves_parity_on_shared_corpus() {
    let tmp = tempfile::tempdir().unwrap();

    // Create shared docs directory (simulates mika/docs/solutions).
    let shared_docs = setup_docs_dir(tmp.path(), "shared-docs", 5);
    // Create unique docs directory (simulates mika-cloud/docs/solutions).
    let unique_docs = setup_docs_dir(tmp.path(), "unique-docs", 3);

    let shared_hash = hash_docs_root(&shared_docs);
    let unique_hash = hash_docs_root(&unique_docs);
    assert_ne!(shared_hash, unique_hash);

    // Open shared DB (same SQLite file — mirrors production per-agent isolation).
    let tmp_db = tempfile::NamedTempFile::new().unwrap();
    let db = Database::open(tmp_db.path()).unwrap();
    let db_single = AsyncDatabase::new_with_agent(db, AGENT_SINGLE);
    ensure_agent(&db_single, AGENT_SINGLE).await;
    ensure_agent(&db_single, AGENT_MULTI).await;

    // --- Agent A (single-corpus): ingests shared_docs only ---
    let ingestor_a = LexicalIngestor::new(db_single.clone(), shared_docs.clone(), None);
    let stats_a = ingestor_a.ingest_all().await.unwrap();
    assert!(
        stats_a.chunks_added > 0,
        "single-corpus agent should ingest chunks"
    );

    let single_count =
        count_search_content_for_corpus(&db_single, AGENT_SINGLE, &shared_hash).await;
    assert!(
        single_count > 0,
        "single-corpus agent should have search_content rows"
    );

    // --- Agent B (multi-corpus): ingests shared_docs AND unique_docs ---
    let db_multi = db_single.with_agent(AGENT_MULTI);

    // Ingest shared corpus first.
    let ingestor_b_shared = LexicalIngestor::new(db_multi.clone(), shared_docs.clone(), None);
    let stats_b_shared = ingestor_b_shared.ingest_all().await.unwrap();

    // The shared corpus was already ingested by agent A — agent B should get
    // backfill (chunks exist, search_content doesn't for agent B).
    assert_eq!(
        stats_b_shared.docs_index_backfilled + stats_b_shared.docs_ingested,
        stats_a
            .docs_ingested
            .max(stats_b_shared.docs_scanned.min(stats_a.docs_scanned)),
        "all shared docs should be processed (ingested or backfilled)"
    );

    // Ingest unique corpus.
    let ingestor_b_unique = LexicalIngestor::new(db_multi.clone(), unique_docs.clone(), None);
    let stats_b_unique = ingestor_b_unique.ingest_all().await.unwrap();
    assert!(
        stats_b_unique.chunks_added > 0,
        "multi-corpus agent should ingest unique corpus chunks"
    );

    // --- PARITY ASSERTION: shared corpus counts must match ---
    let multi_shared_count =
        count_search_content_for_corpus(&db_multi, AGENT_MULTI, &shared_hash).await;
    assert_eq!(
        multi_shared_count, single_count,
        "PARITY VIOLATION: multi-corpus agent has {multi_shared_count} search_content rows \
         for shared corpus vs {single_count} for single-corpus agent"
    );

    // Agent B should additionally have rows for its unique corpus.
    let multi_unique_count =
        count_search_content_for_corpus(&db_multi, AGENT_MULTI, &unique_hash).await;
    assert!(
        multi_unique_count > 0,
        "multi-corpus agent should also have search_content for unique corpus"
    );

    let multi_total = count_search_content_total(&db_multi, AGENT_MULTI).await;
    assert_eq!(
        multi_total,
        multi_shared_count + multi_unique_count,
        "total should equal shared + unique"
    );
}

/// Order-independence: parity holds regardless of which agent ingests first.
///
/// This test inverts the order — multi-corpus agent ingests first, then
/// single-corpus agent ingests the same shared directory. Both must end up
/// with identical counts for the shared corpus.
#[tokio::test]
async fn parity_holds_regardless_of_ingestion_order() {
    let tmp = tempfile::tempdir().unwrap();

    let shared_docs = setup_docs_dir(tmp.path(), "shared-docs", 4);
    let unique_docs = setup_docs_dir(tmp.path(), "unique-docs", 2);

    let shared_hash = hash_docs_root(&shared_docs);

    let tmp_db = tempfile::NamedTempFile::new().unwrap();
    let db = Database::open(tmp_db.path()).unwrap();
    let db_multi = AsyncDatabase::new_with_agent(db, AGENT_MULTI);
    ensure_agent(&db_multi, AGENT_MULTI).await;
    ensure_agent(&db_multi, AGENT_SINGLE).await;

    // --- Multi-corpus agent ingests FIRST ---
    // Shared corpus.
    let ingestor_multi_shared = LexicalIngestor::new(db_multi.clone(), shared_docs.clone(), None);
    let stats_multi_shared = ingestor_multi_shared.ingest_all().await.unwrap();
    assert!(stats_multi_shared.chunks_added > 0);

    // Unique corpus.
    let ingestor_multi_unique = LexicalIngestor::new(db_multi.clone(), unique_docs.clone(), None);
    ingestor_multi_unique.ingest_all().await.unwrap();

    let multi_shared_count =
        count_search_content_for_corpus(&db_multi, AGENT_MULTI, &shared_hash).await;
    assert!(multi_shared_count > 0);

    // --- Single-corpus agent ingests SECOND ---
    let db_single = db_multi.with_agent(AGENT_SINGLE);
    let ingestor_single = LexicalIngestor::new(db_single.clone(), shared_docs.clone(), None);
    let stats_single = ingestor_single.ingest_all().await.unwrap();

    // Single-corpus agent should get backfill since chunks already exist.
    assert!(
        stats_single.docs_index_backfilled > 0 || stats_single.docs_ingested > 0,
        "single agent should process docs (fresh ingest or backfill)"
    );

    let single_shared_count =
        count_search_content_for_corpus(&db_single, AGENT_SINGLE, &shared_hash).await;

    // --- PARITY ASSERTION ---
    assert_eq!(
        single_shared_count, multi_shared_count,
        "PARITY VIOLATION (reversed order): single-corpus agent has {single_shared_count} \
         search_content rows vs {multi_shared_count} for multi-corpus agent on shared corpus"
    );
}

/// Three-agent parity: simulates the production topology where mika-arch
/// (6 corpora) shares one corpus with mika (1 corpus) and mika-dev (1 corpus).
///
/// All three agents must converge to the same search_content count for the
/// shared corpus regardless of ingestion order.
#[tokio::test]
async fn three_agent_shared_corpus_parity() {
    let tmp = tempfile::tempdir().unwrap();

    let shared_docs = setup_docs_dir(tmp.path(), "shared-solutions", 6);
    let arch_extra_1 = setup_docs_dir(tmp.path(), "cloud-solutions", 2);
    let arch_extra_2 = setup_docs_dir(tmp.path(), "skills-solutions", 2);

    let shared_hash = hash_docs_root(&shared_docs);

    let tmp_db = tempfile::NamedTempFile::new().unwrap();
    let db = Database::open(tmp_db.path()).unwrap();

    let agent_mika = "mika";
    let agent_dev = "mika-dev";
    let agent_arch = "mika-arch";

    let db_mika = AsyncDatabase::new_with_agent(db, agent_mika);
    ensure_agent(&db_mika, agent_mika).await;
    ensure_agent(&db_mika, agent_dev).await;
    ensure_agent(&db_mika, agent_arch).await;

    // mika ingests shared corpus.
    let ingestor_mika = LexicalIngestor::new(db_mika.clone(), shared_docs.clone(), None);
    ingestor_mika.ingest_all().await.unwrap();

    // mika-arch ingests shared corpus + 2 extra.
    let db_arch = db_mika.with_agent(agent_arch);
    let ingestor_arch_shared = LexicalIngestor::new(db_arch.clone(), shared_docs.clone(), None);
    ingestor_arch_shared.ingest_all().await.unwrap();
    let ingestor_arch_extra1 = LexicalIngestor::new(db_arch.clone(), arch_extra_1.clone(), None);
    ingestor_arch_extra1.ingest_all().await.unwrap();
    let ingestor_arch_extra2 = LexicalIngestor::new(db_arch.clone(), arch_extra_2.clone(), None);
    ingestor_arch_extra2.ingest_all().await.unwrap();

    // mika-dev ingests shared corpus.
    let db_dev = db_mika.with_agent(agent_dev);
    let ingestor_dev = LexicalIngestor::new(db_dev.clone(), shared_docs.clone(), None);
    ingestor_dev.ingest_all().await.unwrap();

    // --- PARITY ASSERTIONS ---
    let mika_count = count_search_content_for_corpus(&db_mika, agent_mika, &shared_hash).await;
    let arch_count = count_search_content_for_corpus(&db_arch, agent_arch, &shared_hash).await;
    let dev_count = count_search_content_for_corpus(&db_dev, agent_dev, &shared_hash).await;

    assert!(mika_count > 0, "mika must have indexed chunks");

    assert_eq!(
        arch_count, mika_count,
        "PARITY VIOLATION: mika-arch has {arch_count} search_content rows \
         vs mika's {mika_count} for shared corpus"
    );
    assert_eq!(
        dev_count, mika_count,
        "PARITY VIOLATION: mika-dev has {dev_count} search_content rows \
         vs mika's {mika_count} for shared corpus"
    );

    // mika-arch should have additional rows from its extra corpora.
    let arch_total = count_search_content_total(&db_arch, agent_arch).await;
    assert!(
        arch_total > arch_count,
        "mika-arch total ({arch_total}) should exceed shared-only ({arch_count})"
    );
}
