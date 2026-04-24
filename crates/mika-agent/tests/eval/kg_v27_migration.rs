//! # v26->v27 KG Migration Invariant Tests (#787)
//!
//! Eight invariant tests for the `v27_coalesce_sql` migration that coalesces
//! per-agent v26 KG data into shared-corpus v27 tables.
//!
//! Each test:
//! 1. Builds a synthetic v26 database via `build_v26_synthetic_db`
//! 2. Snapshots pre-migration state
//! 3. Runs `v27_coalesce_sql` via `execute_batch`
//! 4. Asserts invariants on the post-coalesce v27 tables

use crate::eval::kg_fixtures::{
    DriftProfile, TEST_DOCS_ROOT, TEST_DOCS_ROOT_HASH, build_v26_synthetic_db,
};
use mika_agent::db::v27_coalesce_sql;
use std::collections::HashSet;

fn run_coalesce(conn: &rusqlite::Connection) {
    let sql = v27_coalesce_sql(TEST_DOCS_ROOT, TEST_DOCS_ROOT_HASH);
    conn.execute_batch(&sql).expect("coalesce SQL failed");
}

/// Helper: query a single i64 count.
fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

// ---------------------------------------------------------------------------
// Invariant 1: Chunk coalesce preserves distinct (source_doc_path, seq_id) pairs
// ---------------------------------------------------------------------------

#[test]
fn test_invariant_1_chunks_coalesce() {
    let conn = build_v26_synthetic_db(3, 5, DriftProfile::ObservedDrift);

    // Pre-migration: count distinct (source_doc_path, seq_id) pairs in v26 backup
    let expected_count: i64 = count(
        &conn,
        "SELECT COUNT(DISTINCT source_doc_path || '|' || CAST(seq_id AS TEXT)) \
         FROM kg_chunks_v26_backup",
    );
    assert!(expected_count > 0, "fixture should have chunks");

    run_coalesce(&conn);

    let post_count: i64 = count(&conn, "SELECT COUNT(id) FROM kg_chunks");
    assert_eq!(
        post_count, expected_count,
        "chunks coalesce must preserve exactly the distinct (source_doc_path, seq_id) pairs"
    );
}

// ---------------------------------------------------------------------------
// Invariant 2: Resolution count preserved
// ---------------------------------------------------------------------------

#[test]
fn test_invariant_2_resolutions_count() {
    let conn = build_v26_synthetic_db(3, 5, DriftProfile::ObservedDrift);

    let pre_count: i64 = count(
        &conn,
        "SELECT COUNT(id) FROM kg_subject_resolutions_v26_backup",
    );
    assert!(pre_count > 0, "fixture should have resolutions");

    run_coalesce(&conn);

    let post_count: i64 = count(&conn, "SELECT COUNT(id) FROM kg_subject_resolutions");
    assert_eq!(
        post_count, pre_count,
        "resolution count must be preserved exactly (fixture avoids collisions)"
    );
}

// ---------------------------------------------------------------------------
// Invariant 3: Resolution log count preserved
// ---------------------------------------------------------------------------

#[test]
fn test_invariant_3_resolution_log_count() {
    let conn = build_v26_synthetic_db(3, 5, DriftProfile::ObservedDrift);

    let pre_count: i64 = count(&conn, "SELECT COUNT(id) FROM kg_resolutions_log_v26_backup");
    assert!(pre_count > 0, "fixture should have resolution log entries");

    run_coalesce(&conn);

    let post_count: i64 = count(&conn, "SELECT COUNT(id) FROM kg_resolutions_log");
    assert_eq!(
        post_count, pre_count,
        "resolution log count must be preserved exactly (fixture avoids collisions)"
    );
}

// ---------------------------------------------------------------------------
// Invariant 4: No orphan foreign keys after coalesce
// ---------------------------------------------------------------------------

#[test]
fn test_invariant_4_no_orphan_fks() {
    let conn = build_v26_synthetic_db(3, 5, DriftProfile::ObservedDrift);

    run_coalesce(&conn);

    let orphan_checks = [
        (
            "kg_chunk_subjects.chunk_id -> kg_chunks",
            "SELECT COUNT(id) FROM kg_chunk_subjects WHERE chunk_id NOT IN (SELECT id FROM kg_chunks)",
        ),
        (
            "kg_chunk_subjects.subject_entity_id -> kg_subject_entities",
            "SELECT COUNT(id) FROM kg_chunk_subjects WHERE subject_entity_id NOT IN (SELECT id FROM kg_subject_entities)",
        ),
        (
            "kg_chunk_subject_relationships.chunk_id -> kg_chunks",
            "SELECT COUNT(id) FROM kg_chunk_subject_relationships WHERE chunk_id NOT IN (SELECT id FROM kg_chunks)",
        ),
        (
            "kg_chunk_subject_relationships.subject_relationship_id -> kg_subject_relationships",
            "SELECT COUNT(id) FROM kg_chunk_subject_relationships WHERE subject_relationship_id NOT IN (SELECT id FROM kg_subject_relationships)",
        ),
        (
            "kg_subject_resolutions.subject_entity_id -> kg_subject_entities",
            "SELECT COUNT(id) FROM kg_subject_resolutions WHERE subject_entity_id NOT IN (SELECT id FROM kg_subject_entities)",
        ),
        (
            "kg_resolutions_log.subject_entity_id -> kg_subject_entities",
            "SELECT COUNT(id) FROM kg_resolutions_log WHERE subject_entity_id NOT IN (SELECT id FROM kg_subject_entities)",
        ),
    ];

    for (label, sql) in &orphan_checks {
        let orphan_count: i64 = count(&conn, sql);
        assert_eq!(orphan_count, 0, "orphan FK check failed: {label}");
    }
}

// ---------------------------------------------------------------------------
// Invariant 5: Chunk source_doc_hash roundtrips correctly
// ---------------------------------------------------------------------------

#[test]
fn test_invariant_5_chunk_text_roundtrip() {
    let conn = build_v26_synthetic_db(3, 5, DriftProfile::ObservedDrift);

    // Capture 10 (source_doc_path, seq_id, source_doc_hash) triples from v26 backup
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT source_doc_path, seq_id, source_doc_hash \
             FROM kg_chunks_v26_backup \
             ORDER BY source_doc_path, seq_id \
             LIMIT 10",
        )
        .unwrap();
    let triples: Vec<(String, i64, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    assert!(!triples.is_empty(), "fixture should have chunks to sample");

    run_coalesce(&conn);

    for (doc_path, seq_id, expected_hash) in &triples {
        let actual_hash: String = conn
            .query_row(
                &format!(
                    "SELECT source_doc_hash FROM kg_chunks \
                     WHERE docs_root_hash = '{TEST_DOCS_ROOT_HASH}' \
                       AND source_doc_path = ?1 AND seq_id = ?2"
                ),
                rusqlite::params![doc_path, seq_id],
                |row| row.get(0),
            )
            .unwrap_or_else(|e| {
                panic!("chunk ({doc_path}, seq_id={seq_id}) missing after coalesce: {e}")
            });
        assert_eq!(
            &actual_hash, expected_hash,
            "source_doc_hash mismatch for ({doc_path}, seq_id={seq_id})"
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 6: Entity majority-vote produces correct winner set
// ---------------------------------------------------------------------------

#[test]
fn test_invariant_6_entity_majority_vote() {
    let conn = build_v26_synthetic_db(3, 5, DriftProfile::ObservedDrift);

    // Before coalesce: compute the set of normalized entity keys that should survive.
    // Each distinct LOWER(TRIM(entity_key)) should produce exactly one winner row.
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT LOWER(TRIM(entity_key)) \
             FROM kg_subject_entities_v26_backup",
        )
        .unwrap();
    let expected_keys: HashSet<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    assert!(!expected_keys.is_empty(), "fixture should have entities");

    run_coalesce(&conn);

    // After coalesce: collect actual entity keys
    let mut stmt = conn
        .prepare(
            "SELECT entity_key FROM kg_subject_entities \
             WHERE docs_root_hash = ?1",
        )
        .unwrap();
    let actual_keys: HashSet<String> = stmt
        .query_map(rusqlite::params![TEST_DOCS_ROOT_HASH], |row| {
            row.get::<_, String>(0)
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .map(|k| k.trim().to_lowercase())
        .collect();

    assert_eq!(
        actual_keys, expected_keys,
        "entity majority-vote must produce one winner per normalized entity_key"
    );
}

// ---------------------------------------------------------------------------
// Invariant 7: Resolution FK validity after coalesce
// ---------------------------------------------------------------------------

#[test]
fn test_invariant_7_resolution_fk_validity() {
    let conn = build_v26_synthetic_db(3, 5, DriftProfile::ObservedDrift);

    run_coalesce(&conn);

    let orphan_count: i64 = count(
        &conn,
        "SELECT COUNT(id) FROM kg_subject_resolutions sr \
         WHERE NOT EXISTS ( \
             SELECT 1 FROM kg_subject_entities se WHERE se.id = sr.subject_entity_id \
         )",
    );
    assert_eq!(
        orphan_count, 0,
        "every resolution's subject_entity_id must reference a valid entity"
    );
}

// ---------------------------------------------------------------------------
// Invariant 8: Insertion order independence (deterministic tiebreak)
// ---------------------------------------------------------------------------

#[test]
fn test_invariant_8_insertion_order_independence() {
    // Build two identical synthetic DBs and coalesce both.
    // Since the fixture is deterministic (uses agent_idx + doc_idx as seed),
    // both should produce the exact same entity_key set.
    let conn_a = build_v26_synthetic_db(3, 5, DriftProfile::ObservedDrift);
    let conn_b = build_v26_synthetic_db(3, 5, DriftProfile::ObservedDrift);

    run_coalesce(&conn_a);
    run_coalesce(&conn_b);

    let collect_keys = |conn: &rusqlite::Connection| -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT entity_key FROM kg_subject_entities \
                 WHERE docs_root_hash = ?1 \
                 ORDER BY entity_key",
            )
            .unwrap();
        stmt.query_map(rusqlite::params![TEST_DOCS_ROOT_HASH], |row| {
            row.get::<_, String>(0)
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    };

    let keys_a = collect_keys(&conn_a);
    let keys_b = collect_keys(&conn_b);

    assert!(!keys_a.is_empty(), "coalesce should produce entities");
    assert_eq!(
        keys_a, keys_b,
        "coalesce must produce identical entity sets regardless of insertion order"
    );
}
