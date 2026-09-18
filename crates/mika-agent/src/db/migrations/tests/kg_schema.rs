//! Harnais de convergence avant du schéma KG (v24 → v25) — mika#2321.
//!
//! Descendu de `db::tests` avec l'échelle de migrations : chacun de ses cas
//! appelle une `migrate_vN_to_vM` privée, et le harnais de snapshot de schéma
//! qu'il partage avec eux n'avait aucun autre lecteur.

use super::*;

#[test]
fn test_v24_to_v25_migration_adds_kg_tables() {
    let db = db();
    // Fresh DB starts at v25 via migrate_v1 — tables should exist
    for table in KG_TABLES {
        assert!(
            table_exists(&db.conn, table),
            "KG table '{table}' should exist after fresh DB creation"
        );
    }

    // Verify key indexes exist
    let expected_indexes = [
        "idx_kg_entities_type",
        "idx_kg_rel_from",
        "idx_kg_rel_to",
        "idx_kg_chunks_docs_root_hash_doc",
        "idx_kg_subj_entities_drh_type",
        "idx_kg_resolutions_agent_subj",
        "idx_kg_resolutions_agent_dom",
        "idx_kg_subj_rel_from",
        "idx_kg_subj_rel_to",
        "idx_kg_subj_rel_type",
        "idx_kg_cs_chunk",
        "idx_kg_cs_entity",
        "idx_kg_cs_trace",
        "idx_kg_csr_chunk",
        "idx_kg_csr_rel",
        "idx_kg_extractions_drh",
        "idx_kg_res_log_pending",
    ];
    for idx in expected_indexes {
        assert!(index_exists(&db.conn, idx), "KG index '{idx}' should exist");
    }
}

#[test]
fn test_v46_served_content_table_and_indexes() {
    let db = db();
    assert!(
        table_exists(&db.conn, "served_content"),
        "served_content table should exist after fresh DB creation (v46, mika#1867)"
    );
    assert!(
        index_exists(&db.conn, "idx_served_content_person_cat"),
        "idx_served_content_person_cat should exist"
    );
    assert!(
        index_exists(&db.conn, "idx_served_content_hash"),
        "idx_served_content_hash should exist"
    );
}

#[test]
fn test_v46_served_content_category_check_constraint() {
    let db = db();
    // Register a valid person for FK
    let pid = db
        .upsert_person("mika", "Test", None, None)
        .expect("insert person");

    // Valid category should insert
    db.conn
        .execute(
            "INSERT INTO served_content (agent_id, person_id, category, content_text, content_hash)
                 VALUES ('mika', ?1, 'proverb', 'x', 'h')",
            rusqlite::params![pid],
        )
        .expect("valid category should insert");

    // Invalid category should fail CHECK constraint
    let result = db.conn.execute(
        "INSERT INTO served_content (agent_id, person_id, category, content_text, content_hash)
             VALUES ('mika', ?1, 'riddle', 'y', 'h2')",
        rusqlite::params![pid],
    );
    assert!(
        result.is_err(),
        "invalid category 'riddle' should be rejected by CHECK constraint"
    );
}

#[test]
fn test_v25_entity_key_check_constraint() {
    let db = db();
    // Valid entity — should succeed
    db.conn
        .execute(
            "INSERT INTO kg_entities (entity_key, type, name) VALUES ('skill:self-dev', 'skill', 'self-dev')",
            [],
        )
        .expect("valid entity should insert");

    // Invalid entity — entity_key doesn't match type:name
    let result = db.conn.execute(
        "INSERT INTO kg_entities (entity_key, type, name) VALUES ('wrong-key', 'skill', 'self-dev')",
        [],
    );
    assert!(
        result.is_err(),
        "entity_key CHECK constraint should reject mismatched key"
    );
}

#[test]
fn test_v25_subject_entity_confidence_constraint() {
    let db = db();
    db.conn
        .execute(
            "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
                 VALUES ('0000000000000000', '/test', 'failure_mode:oom', 'failure_mode', 'oom', 0.85)",
            [],
        )
        .expect("valid confidence should insert");

    // Confidence > 1.0 should fail
    let result = db.conn.execute(
        "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
             VALUES ('0000000000000000', '/test', 'failure_mode:crash', 'failure_mode', 'crash', 1.5)",
        [],
    );
    assert!(
        result.is_err(),
        "confidence > 1.0 should be rejected by CHECK constraint"
    );

    // Confidence < 0.0 should fail
    let result = db.conn.execute(
        "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
             VALUES ('0000000000000000', '/test', 'failure_mode:hang', 'failure_mode', 'hang', -0.1)",
        [],
    );
    assert!(
        result.is_err(),
        "confidence < 0.0 should be rejected by CHECK constraint"
    );
}

#[test]
fn test_v25_kg_chunks_agent_cascade_delete() {
    let db = db();
    // Enable FK enforcement
    db.conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

    db.conn
        .execute(
            "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
                 VALUES ('0000000000000000', '/test', 0, 'docs/test.md', 'abc123hash')",
            [],
        )
        .unwrap();

    let count: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM kg_chunks WHERE docs_root_hash = '0000000000000000'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    // Delete the agent — v27 shared-layer chunks should NOT cascade
    db.conn
        .execute("DELETE FROM agents WHERE id = 'mika'", [])
        .unwrap();

    let count: i64 = db
        .conn
        .query_row("SELECT count(*) FROM kg_chunks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 1,
        "kg_chunks should persist after agent deletion (v27 shared-layer)"
    );
}

#[test]
fn test_v25_kg_relationships_entity_cascade_delete() {
    let db = db();
    db.conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

    db.conn
        .execute(
            "INSERT INTO kg_entities (entity_key, type, name) VALUES ('skill:a', 'skill', 'a')",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO kg_entities (entity_key, type, name) VALUES ('tool:b', 'tool', 'b')",
            [],
        )
        .unwrap();

    let from_id: i64 = db
        .conn
        .query_row(
            "SELECT id FROM kg_entities WHERE entity_key = 'skill:a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let to_id: i64 = db
        .conn
        .query_row(
            "SELECT id FROM kg_entities WHERE entity_key = 'tool:b'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    db.conn
        .execute(
            "INSERT INTO kg_relationships (from_entity_id, to_entity_id, type) VALUES (?1, ?2, 'PROVIDES')",
            rusqlite::params![from_id, to_id],
        )
        .unwrap();

    // Delete from_entity — relationship should cascade
    db.conn
        .execute("DELETE FROM kg_entities WHERE entity_key = 'skill:a'", [])
        .unwrap();

    let count: i64 = db
        .conn
        .query_row("SELECT count(*) FROM kg_relationships", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "kg_relationships should cascade-delete when from_entity is deleted"
    );
}

#[test]
fn test_v25_resolutions_log_outcome_check() {
    let db = db();
    db.conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();

    // Valid outcome
    db.conn
        .execute(
            "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
                 VALUES ('mika', 1, 'matched_exact', 'trace-1')",
            [],
        )
        .expect("valid outcome should insert");

    // Invalid outcome
    let result = db.conn.execute(
        "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
             VALUES ('mika', 2, 'invalid_outcome', 'trace-2')",
        [],
    );
    assert!(
        result.is_err(),
        "invalid outcome should be rejected by CHECK constraint"
    );
}

#[test]
fn test_v1_and_incremental_schemas_converge() {
    // DB1: fresh install via migrate_v1 (reaches CURRENT_SCHEMA_VERSION directly).
    let db1 = Database::open_in_memory().unwrap();
    let snap1 = snapshot_schema(&db1.conn);

    // DB2: simulate a v24 DB, then migrate incrementally through v25,
    // v26 (#757), and v27 (#786). Strategy: create a fresh DB, extract all non-KG DDL from
    // sqlite_master, replay it on a new connection to get a v24 DB, then
    // run migrate_v24_to_v25 and migrate_v25_to_v26 and compare schemas.
    let fresh = Database::open_in_memory().unwrap();
    let mut stmt = fresh
        .conn
        .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY rowid")
        .unwrap();
    let ddl_stmts: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    drop(stmt);

    init_sqlite_vec();
    let conn2 = rusqlite::Connection::open_in_memory().unwrap();
    conn2.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

    // Replay all DDL except KG tables, virtual tables, views, and schema_meta
    conn2.execute_batch("BEGIN;").unwrap();
    for ddl in &ddl_stmts {
        let lower = ddl.to_lowercase();
        if lower.contains("kg_entities")
            || lower.contains("kg_relationships")
            || lower.contains("kg_chunks")
            || lower.contains("kg_subject_")
            || lower.contains("kg_chunk_subject")
            || lower.contains("kg_extractions")
            || lower.contains("kg_resolutions_log")
            || lower.contains("agent_kg_corpora")
            || lower.contains("idx_kg_")
            || lower.contains("idx_agent_kg_corpora")
            || lower.contains("schema_meta")
            || lower.contains("operational_items")
            || lower.contains("auto_pull_stats")
        {
            continue;
        }
        if lower.contains("fts5") || lower.contains("vec0") {
            continue;
        }
        if lower.contains("unified_timeline") {
            continue;
        }
        if lower.contains("sqlite_sequence") {
            continue;
        }
        conn2.execute_batch(ddl).unwrap_or_else(|e| {
            if !e.to_string().contains("already exists") {
                panic!("DDL failed: {ddl}: {e}");
            }
        });
    }
    conn2.execute_batch("COMMIT;").unwrap();

    // Recreate view and virtual tables
    conn2.execute_batch(UNIFIED_TIMELINE_VIEW_SQL).unwrap();
    let _ = conn2.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS fts_search
                 USING fts5(content, content='search_content', content_rowid='id');
             CREATE VIRTUAL TABLE IF NOT EXISTS vec_search
                 USING vec0(embedding float[512]);",
    );

    // Set version to 24 and insert default agent
    conn2
        .execute_batch(
            "DELETE FROM schema_version WHERE version > 24;
                 INSERT OR IGNORE INTO schema_version (version) VALUES (24);
                 INSERT OR IGNORE INTO agents (id, name, home_dir) VALUES ('mika', 'Mika', '');",
        )
        .unwrap();

    // Verify v24 state: KG tables should not exist
    let v24_version: i64 = conn2
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        v24_version, 24,
        "DB2 should be at v24 before incremental migration"
    );
    assert!(
        !table_exists(&conn2, "kg_entities"),
        "kg_entities should not exist at v24"
    );

    // Run incremental migrations: v24 -> v25 -> v26 -> v27 -> v28 -> v29
    let mut db2 = Database { conn: conn2 };
    db2.migrate_v24_to_v25().unwrap();
    db2.migrate_v25_to_v26().unwrap();
    db2.migrate_v26_to_v27().unwrap();

    // Insert the v27 coalesce marker so check_v27_coalesce_guard() passes.
    // In production this is written by the #787 coalesce step; in tests we
    // short-circuit it so the convergence comparison can proceed.
    db2.conn
        .execute(
            "INSERT OR IGNORE INTO schema_meta (key, value) VALUES ('v27_coalesce_complete', '1')",
            [],
        )
        .unwrap();

    db2.migrate_v27_to_v28().unwrap();
    db2.migrate_v28_to_v29().unwrap();
    db2.migrate_v29_to_v30().unwrap();
    db2.migrate_v30_to_v31().unwrap();
    db2.migrate_v31_to_v32().unwrap();
    db2.migrate_v32_to_v33().unwrap();
    db2.migrate_v33_to_v34().unwrap();
    db2.migrate_v34_to_v35().unwrap();
    db2.migrate_v35_to_v36().unwrap();
    db2.migrate_v36_to_v37().unwrap();
    db2.migrate_v37_to_v38().unwrap();
    db2.migrate_v38_to_v39().unwrap();
    db2.migrate_v39_to_v40().unwrap();
    db2.migrate_v40_to_v41().unwrap();
    db2.migrate_v41_to_v42().unwrap();
    db2.migrate_v42_to_v43().unwrap();
    db2.migrate_v43_to_v44().unwrap();
    db2.migrate_v44_to_v45().unwrap();
    db2.migrate_v45_to_v46().unwrap();
    db2.migrate_v46_to_v47().unwrap();
    db2.migrate_v47_to_v48().unwrap();
    db2.migrate_v48_to_v49().unwrap();
    db2.migrate_v49_to_v50().unwrap();
    db2.migrate_v50_to_v51().unwrap();
    db2.migrate_v51_to_v52().unwrap();
    db2.migrate_v52_to_v53().unwrap();
    db2.migrate_v53_to_v54().unwrap();

    let final_version: i64 = db2
        .conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        final_version, CURRENT_SCHEMA_VERSION,
        "DB2 should be at CURRENT_SCHEMA_VERSION after incremental migrations"
    );

    // #757: kg_extractions.source_doc_hash must exist after v27 migration.
    assert!(
        db2.column_exists("kg_extractions", "source_doc_hash")
            .unwrap(),
        "kg_extractions.source_doc_hash should exist after v27 migration"
    );

    // #786: kg_chunks.docs_root_hash must exist after v27 migration.
    assert!(
        db2.column_exists("kg_chunks", "docs_root_hash").unwrap(),
        "kg_chunks.docs_root_hash should exist after v27 migration"
    );

    // #798: agent_kg_corpora must exist after v28 migration.
    assert!(
        table_exists(&db2.conn, "agent_kg_corpora"),
        "agent_kg_corpora should exist after v28 migration"
    );

    let snap2 = snapshot_schema(&db2.conn);

    // Compare schemas structurally
    assert_eq!(
        snap1.len(),
        snap2.len(),
        "Table count mismatch: v1 has {} tables, incremental has {}\nv1: {:?}\nincremental: {:?}",
        snap1.len(),
        snap2.len(),
        snap1.iter().map(|t| &t.name).collect::<Vec<_>>(),
        snap2.iter().map(|t| &t.name).collect::<Vec<_>>(),
    );

    for (t1, t2) in snap1.iter().zip(snap2.iter()) {
        assert_eq!(t1.name, t2.name, "Table name mismatch");
        assert_eq!(
            t1.columns, t2.columns,
            "Column mismatch in table '{}'",
            t1.name
        );
        assert_eq!(
            t1.foreign_keys, t2.foreign_keys,
            "Foreign key mismatch in table '{}'",
            t1.name
        );
        assert_eq!(
            t1.indexes, t2.indexes,
            "Index mismatch in table '{}'",
            t1.name
        );
    }
}

#[test]
fn test_v25_kg_chunks_unique_constraint() {
    let db = db();
    db.conn
        .execute(
            "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
                 VALUES ('0000000000000000', '/test', 0, 'docs/test.md', 'hash1')",
            [],
        )
        .unwrap();

    // Duplicate (docs_root_hash, source_doc_path, seq_id) should fail
    let result = db.conn.execute(
        "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
             VALUES ('0000000000000000', '/test', 0, 'docs/test.md', 'hash2')",
        [],
    );
    assert!(
        result.is_err(),
        "duplicate (docs_root_hash, source_doc_path, seq_id) should violate UNIQUE constraint"
    );

    // Different seq_id should succeed
    db.conn
        .execute(
            "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
                 VALUES ('0000000000000000', '/test', 1, 'docs/test.md', 'hash1')",
            [],
        )
        .expect("different seq_id should be allowed");
}

#[test]
fn test_v25_subject_entity_agent_key_unique() {
    let db = db();
    db.conn
        .execute(
            "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
                 VALUES ('0000000000000000', '/test', 'failure_mode:oom', 'failure_mode', 'oom', 0.9)",
            [],
        )
        .unwrap();

    // Duplicate (docs_root_hash, entity_key) should fail
    let result = db.conn.execute(
        "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
             VALUES ('0000000000000000', '/test', 'failure_mode:oom', 'failure_mode', 'oom', 0.8)",
        [],
    );
    assert!(
        result.is_err(),
        "duplicate (docs_root_hash, entity_key) should violate UNIQUE constraint"
    );
}

// ── count_chunks_for_docs_root_hash tests (#778) ──────────────────────

#[test]
fn count_chunks_for_docs_root_hash_returns_zero_for_unknown() {
    let db = db();
    let count = db
        .count_chunks_for_docs_root_hash("unknown_hash_0000")
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn count_chunks_for_docs_root_hash_returns_correct_count() {
    let db = db();
    let hash = "abc1234567890abc";
    // Insert 3 chunks with the same docs_root_hash.
    for seq in 0..3 {
        db.conn
            .execute(
                "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
                     VALUES (?1, '/test', ?2, 'docs/test.md', 'dochash')",
                rusqlite::params![hash, seq],
            )
            .unwrap();
    }
    let count = db.count_chunks_for_docs_root_hash(hash).unwrap();
    assert_eq!(count, 3);

    // Different hash should still return 0.
    let other = db
        .count_chunks_for_docs_root_hash("other_hash_000000")
        .unwrap();
    assert_eq!(other, 0);
}

// --- Transaction RAII tests (mika#636) ---

/// Test 1: A committed RAII transaction persists writes visible to a
/// separate connection.
#[test]
fn test_transaction_commit_persists() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    // Connection A: write inside a committed transaction.
    {
        let mut conn_a = Connection::open(path).unwrap();
        conn_a.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn_a
            .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);")
            .unwrap();

        let tx = conn_a.transaction().unwrap();
        tx.execute("INSERT INTO t (val) VALUES ('hello')", [])
            .unwrap();
        tx.commit().unwrap();
    }

    // Connection B: verify the row is visible.
    let conn_b = Connection::open(path).unwrap();
    let count: i64 = conn_b
        .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

/// Test 2: A transaction dropped without commit() auto-rolls back — the
/// write is invisible to a separate connection.
#[test]
fn test_transaction_drop_without_commit_rolls_back() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    // Connection A: write inside a transaction that is dropped (not committed).
    {
        let mut conn_a = Connection::open(path).unwrap();
        conn_a.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn_a
            .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);")
            .unwrap();

        let tx = conn_a.transaction().unwrap();
        tx.execute("INSERT INTO t (val) VALUES ('dropped')", [])
            .unwrap();
        // Intentionally NOT calling tx.commit() — drop triggers ROLLBACK.
        drop(tx);

        // Even on the same connection, the row should not be visible.
        let count: i64 = conn_a
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "row should be rolled back on same connection");
    }

    // Connection B: also invisible.
    let conn_b = Connection::open(path).unwrap();
    let count: i64 = conn_b
        .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "row should be rolled back for other connections");
}

/// Test 3: RAII Transaction rollback preserves prior state when an error
/// occurs mid-transaction. Uses the same DEFERRED transaction pattern as
/// `replace_with_summary` to verify that `Transaction::drop` rolls back
/// partial writes.
#[test]
fn test_replace_with_summary_rollback_on_error() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    let mut conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    conn.execute_batch(
        "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT NOT NULL);
             INSERT INTO t (val) VALUES ('original');",
    )
    .unwrap();

    // Verify baseline.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);

    // Simulate a transaction that partially succeeds then fails.
    // This mirrors the replace_with_summary pattern: DELETE + INSERT
    // where the INSERT can fail.
    let result: Result<(), rusqlite::Error> = (|| {
        let tx = conn.transaction()?;
        // First operation succeeds — deletes the original row.
        tx.execute("DELETE FROM t WHERE val = 'original'", [])?;
        // Second operation fails — NOT NULL constraint violation.
        tx.execute("INSERT INTO t (val) VALUES (NULL)", [])?;
        tx.commit()?;
        Ok(())
    })();

    assert!(
        result.is_err(),
        "INSERT NULL should violate NOT NULL constraint"
    );

    // The original row should still be present — RAII Transaction::drop
    // rolled back the DELETE when the INSERT failed.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "original row should be preserved after rollback");

    let val: String = conn
        .query_row("SELECT val FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        val, "original",
        "original value should be preserved after rollback"
    );
}

/// Test 6: Cross-connection staleness regression test.
///
/// Opens two connections to the same WAL-mode DB with `cache=private`
/// (disables in-process shared cache to simulate cross-process visibility).
/// Connection A writes a session. Connection B reads sessions. Without
/// the WAL checkpoint fix, B would see stale data if A's transaction was
/// held; with the fix + checkpoint, B sees fresh data.
#[test]
fn test_dashboard_sees_writes_from_separate_connection() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    // Open Connection A with cache=private to simulate cross-process isolation.
    let uri_a = format!("file:{}?cache=private", path);
    let mut conn_a = Connection::open_with_flags(
        &uri_a,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
            | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap();
    conn_a.execute_batch("PRAGMA journal_mode=WAL;").unwrap();

    // Create schema (minimal: agents + sessions tables).
    conn_a
        .execute_batch(
            "CREATE TABLE agents (
                     id TEXT PRIMARY KEY,
                     name TEXT NOT NULL,
                     home_dir TEXT NOT NULL DEFAULT '',
                     active BOOLEAN NOT NULL DEFAULT 1,
                     last_seen TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                 );
                 CREATE TABLE sessions (
                     id TEXT PRIMARY KEY,
                     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                     channel_type TEXT NOT NULL DEFAULT 'cli',
                     started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     ended_at TEXT,
                     metadata TEXT,
                     parent_session_id TEXT,
                     task_id TEXT
                 );
                 INSERT INTO agents (id, name) VALUES ('test', 'test');
                 INSERT INTO sessions (id, agent_id, channel_type) VALUES ('s1', 'test', 'cli');",
        )
        .unwrap();

    // Open Connection B with cache=private (simulates dashboard reader).
    let uri_b = format!("file:{}?cache=private", path);
    let conn_b = Connection::open_with_flags(
        &uri_b,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap();

    // B sees the initial session.
    let count_before: i64 = conn_b
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count_before, 1);

    // A writes a new session (committed transaction, RAII style).
    {
        let tx = conn_a.transaction().unwrap();
        tx.execute(
            "INSERT INTO sessions (id, agent_id, channel_type) VALUES ('s2', 'test', 'cli')",
            [],
        )
        .unwrap();
        tx.commit().unwrap();
    }

    // Without a checkpoint, B may still see stale data due to WAL snapshot.
    // Run PASSIVE checkpoint on A's connection to advance the snapshot.
    conn_a
        .execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
        .unwrap();

    // B should now see the new session after the checkpoint.
    let count_after: i64 = conn_b
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count_after, 2,
        "dashboard connection should see writes after WAL checkpoint"
    );
}
