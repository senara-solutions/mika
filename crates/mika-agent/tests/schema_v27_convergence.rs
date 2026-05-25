//! Schema v27 convergence test (#786 Unit 4).
//!
//! Verifies structural properties of the v27 schema produced by clean-slate
//! `Database::open_in_memory()`. The convergence guarantee between clean-slate
//! and incremental-upgrade paths is tested via the inline `#[cfg(test)]`
//! module in `db.rs` (which can access `pub(crate)` internals). This
//! integration test exercises the public API surface and verifies v27's
//! key invariants: shared-layer tables use `docs_root_hash`, per-agent
//! tables keep `agent_id`, and the `schema_meta` marker is set.

use anyhow::Result;
use rusqlite::Connection;
use std::collections::BTreeSet;

/// Column info from `PRAGMA table_info`.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ColumnInfo {
    name: String,
    col_type: String,
    notnull: bool,
    dflt_value: Option<String>,
    pk: i32,
}

fn get_columns(conn: &Connection, table: &str) -> Result<BTreeSet<ColumnInfo>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info('{table}')"))?;
    let cols = stmt
        .query_map([], |row| {
            Ok(ColumnInfo {
                name: row.get(1)?,
                col_type: row.get(2)?,
                notnull: row.get::<_, i32>(3)? != 0,
                dflt_value: row.get(4)?,
                pk: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    Ok(cols)
}

fn column_names(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info('{table}')"))
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
}

fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    column_names(conn, table).iter().any(|c| c == column)
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |row| row.get(0),
    )
    .unwrap_or(false)
}

fn get_index_names(conn: &Connection, table: &str) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA index_list('{table}')"))
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .filter_map(|r| r.ok())
        .filter(|name| !name.starts_with("sqlite_autoindex_"))
        .collect()
}

/// Build a fresh DB via `Database::open` on a temp file.
fn build_fresh_db() -> Result<(tempfile::TempDir, Connection)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("fresh.db");
    let _db = mika_agent::db::Database::open(&path)?;
    drop(_db);
    let conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    Ok((dir, conn))
}

// ===== Shared-layer tables: docs_root_hash, no agent_id =====

#[test]
fn shared_layer_tables_have_docs_root_hash_and_docs_root() {
    let (_dir, conn) = build_fresh_db().unwrap();

    let shared_tables = [
        "kg_chunks",
        "kg_subject_entities",
        "kg_subject_relationships",
        "kg_chunk_subjects",
        "kg_chunk_subject_relationships",
        "kg_extractions",
    ];

    for table in &shared_tables {
        assert!(
            has_column(&conn, table, "docs_root_hash"),
            "v27 shared-layer table {table} must have docs_root_hash column"
        );
        assert!(
            has_column(&conn, table, "docs_root"),
            "v27 shared-layer table {table} must have docs_root debug column"
        );
        assert!(
            !has_column(&conn, table, "agent_id"),
            "v27 shared-layer table {table} must NOT have agent_id column"
        );
    }
}

// ===== Per-agent tables: agent_id, no docs_root_hash =====

#[test]
fn per_agent_tables_keep_agent_id() {
    let (_dir, conn) = build_fresh_db().unwrap();

    let per_agent_tables = ["kg_subject_resolutions", "kg_resolutions_log"];

    for table in &per_agent_tables {
        assert!(
            has_column(&conn, table, "agent_id"),
            "Per-agent table {table} must retain agent_id in v27"
        );
        assert!(
            !has_column(&conn, table, "docs_root_hash"),
            "Per-agent table {table} must NOT have docs_root_hash in v27"
        );
    }
}

// ===== Domain tables: unchanged =====

#[test]
fn domain_tables_unchanged() {
    let (_dir, conn) = build_fresh_db().unwrap();

    // kg_entities: no agent_id, no docs_root_hash (global)
    assert!(!has_column(&conn, "kg_entities", "agent_id"));
    assert!(!has_column(&conn, "kg_entities", "docs_root_hash"));
    assert!(has_column(&conn, "kg_entities", "entity_key"));

    // kg_relationships: no agent_id, no docs_root_hash (global)
    assert!(!has_column(&conn, "kg_relationships", "agent_id"));
    assert!(!has_column(&conn, "kg_relationships", "docs_root_hash"));
    assert!(has_column(&conn, "kg_relationships", "from_entity_id"));
}

// ===== schema_meta table and coalesce marker =====

#[test]
fn fresh_db_has_schema_meta_with_coalesce_marker() {
    let (_dir, conn) = build_fresh_db().unwrap();

    assert!(
        table_exists(&conn, "schema_meta"),
        "Fresh v27 DB must have schema_meta table"
    );

    let marker: Option<String> = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'v27_coalesce_complete'",
            [],
            |row| row.get(0),
        )
        .ok();

    assert_eq!(
        marker.as_deref(),
        Some("1"),
        "Fresh DB must have schema_meta.v27_coalesce_complete = '1'"
    );
}

// ===== Schema version =====

#[test]
fn fresh_db_is_at_v39() {
    let (_dir, conn) = build_fresh_db().unwrap();

    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .unwrap();

    assert_eq!(version, 39, "Fresh DB must be at schema version 39");
}

// ===== Index verification =====

#[test]
fn shared_layer_indexes_use_docs_root_hash() {
    let (_dir, conn) = build_fresh_db().unwrap();

    // Verify key indexes exist
    let chunk_indexes = get_index_names(&conn, "kg_chunks");
    assert!(
        chunk_indexes.contains("idx_kg_chunks_docs_root_hash_doc"),
        "kg_chunks must have idx_kg_chunks_docs_root_hash_doc index, got: {chunk_indexes:?}"
    );

    let se_indexes = get_index_names(&conn, "kg_subject_entities");
    assert!(
        se_indexes.contains("idx_kg_subj_entities_drh_type"),
        "kg_subject_entities must have idx_kg_subj_entities_drh_type index, got: {se_indexes:?}"
    );

    let sr_indexes = get_index_names(&conn, "kg_subject_relationships");
    assert!(
        sr_indexes.contains("idx_kg_subj_rel_from"),
        "kg_subject_relationships must have idx_kg_subj_rel_from, got: {sr_indexes:?}"
    );
    assert!(
        sr_indexes.contains("idx_kg_subj_rel_to"),
        "kg_subject_relationships must have idx_kg_subj_rel_to, got: {sr_indexes:?}"
    );
    assert!(
        sr_indexes.contains("idx_kg_subj_rel_type"),
        "kg_subject_relationships must have idx_kg_subj_rel_type, got: {sr_indexes:?}"
    );

    let cs_indexes = get_index_names(&conn, "kg_chunk_subjects");
    assert!(
        cs_indexes.contains("idx_kg_cs_chunk"),
        "kg_chunk_subjects must have idx_kg_cs_chunk, got: {cs_indexes:?}"
    );
    assert!(
        cs_indexes.contains("idx_kg_cs_entity"),
        "kg_chunk_subjects must have idx_kg_cs_entity, got: {cs_indexes:?}"
    );
    assert!(
        cs_indexes.contains("idx_kg_cs_trace"),
        "kg_chunk_subjects must have idx_kg_cs_trace, got: {cs_indexes:?}"
    );

    let csr_indexes = get_index_names(&conn, "kg_chunk_subject_relationships");
    assert!(
        csr_indexes.contains("idx_kg_csr_chunk"),
        "kg_chunk_subject_relationships must have idx_kg_csr_chunk, got: {csr_indexes:?}"
    );
    assert!(
        csr_indexes.contains("idx_kg_csr_rel"),
        "kg_chunk_subject_relationships must have idx_kg_csr_rel, got: {csr_indexes:?}"
    );

    let ext_indexes = get_index_names(&conn, "kg_extractions");
    assert!(
        ext_indexes.contains("idx_kg_extractions_drh"),
        "kg_extractions must have idx_kg_extractions_drh, got: {ext_indexes:?}"
    );
}

// ===== kg_extractions UNIQUE constraint =====

/// Schema-level test: verifies the UNIQUE(docs_root_hash, source_doc_path)
/// constraint on kg_extractions rejects duplicate inserts via INSERT OR IGNORE.
///
/// Note: The application code (#1052) now uses ON CONFLICT ... DO UPDATE
/// (conditional upsert) instead of INSERT OR IGNORE. This test validates
/// the schema constraint, not the application-level upsert semantics.
/// See `kg_extractions_upsert_*` tests below for the application-level behavior.
#[test]
fn kg_extractions_first_writer_wins() {
    let (_dir, conn) = build_fresh_db().unwrap();

    // Register a test agent and seed one extraction row
    conn.execute(
        "INSERT OR IGNORE INTO agents (id, name, home_dir) VALUES ('test-agent', 'Test', '')",
        [],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO kg_extractions (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model)
         VALUES ('0000000000000000', '/test/docs', 'doc1.md', 'hash1', 'test-model')",
        [],
    )
    .unwrap();

    // Second insert with same (docs_root_hash, source_doc_path) should be ignored
    let changes = conn
        .execute(
            "INSERT OR IGNORE INTO kg_extractions (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model)
             VALUES ('0000000000000000', '/test/docs', 'doc1.md', 'hash2', 'another-model')",
            [],
        )
        .unwrap();

    assert_eq!(
        changes, 0,
        "Second INSERT OR IGNORE on same (docs_root_hash, source_doc_path) should be no-op"
    );

    // Verify original row is preserved
    let model: String = conn
        .query_row(
            "SELECT extraction_model FROM kg_extractions WHERE docs_root_hash = '0000000000000000' AND source_doc_path = 'doc1.md'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(model, "test-model", "First writer's data must be preserved");
}

/// Application-level test: verifies the #1052 upsert updates NULL-hash rows.
/// A row with source_doc_hash IS NULL should be updated when re-extracted
/// with a non-NULL hash (fixes the NULL-hash deadlock).
#[test]
fn kg_extractions_upsert_updates_null_hash() {
    let (_dir, conn) = build_fresh_db().unwrap();

    // Seed a NULL-hash extraction row (pre-v26 legacy state)
    conn.execute(
        "INSERT INTO kg_extractions (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model)
         VALUES ('0000000000000000', '/test/docs', 'doc1.md', NULL, 'old-model')",
        [],
    )
    .unwrap();

    // Upsert with the application's ON CONFLICT DO UPDATE pattern
    conn.execute(
        "INSERT INTO kg_extractions
            (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model)
         VALUES ('0000000000000000', '/test/docs', 'doc1.md', 'new-hash', 'new-model')
         ON CONFLICT(docs_root_hash, source_doc_path) DO UPDATE SET
            source_doc_hash = excluded.source_doc_hash,
            extraction_model = excluded.extraction_model,
            entities_extracted = excluded.entities_extracted,
            relationships_extracted = excluded.relationships_extracted,
            extraction_trace_id = excluded.extraction_trace_id,
            created_at = excluded.created_at
         WHERE kg_extractions.source_doc_hash IS NULL
            OR kg_extractions.source_doc_hash != excluded.source_doc_hash",
        [],
    )
    .unwrap();

    let (hash, model): (String, String) = conn
        .query_row(
            "SELECT source_doc_hash, extraction_model FROM kg_extractions
             WHERE docs_root_hash = '0000000000000000' AND source_doc_path = 'doc1.md'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        hash, "new-hash",
        "NULL-hash row must be updated to new hash"
    );
    assert_eq!(model, "new-model", "extraction_model must be updated");
}

/// Application-level test: verifies the #1052 upsert is a no-op when
/// the stored hash matches the incoming hash (content unchanged).
#[test]
fn kg_extractions_upsert_noop_on_matching_hash() {
    let (_dir, conn) = build_fresh_db().unwrap();

    // Seed a row with a known hash
    conn.execute(
        "INSERT INTO kg_extractions (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model)
         VALUES ('0000000000000000', '/test/docs', 'doc1.md', 'same-hash', 'original-model')",
        [],
    )
    .unwrap();

    // Upsert with the same hash — WHERE clause should NOT fire
    conn.execute(
        "INSERT INTO kg_extractions
            (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model)
         VALUES ('0000000000000000', '/test/docs', 'doc1.md', 'same-hash', 'updated-model')
         ON CONFLICT(docs_root_hash, source_doc_path) DO UPDATE SET
            source_doc_hash = excluded.source_doc_hash,
            extraction_model = excluded.extraction_model,
            entities_extracted = excluded.entities_extracted,
            relationships_extracted = excluded.relationships_extracted,
            extraction_trace_id = excluded.extraction_trace_id,
            created_at = excluded.created_at
         WHERE kg_extractions.source_doc_hash IS NULL
            OR kg_extractions.source_doc_hash != excluded.source_doc_hash",
        [],
    )
    .unwrap();

    let model: String = conn
        .query_row(
            "SELECT extraction_model FROM kg_extractions
             WHERE docs_root_hash = '0000000000000000' AND source_doc_path = 'doc1.md'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        model, "original-model",
        "Row must NOT be updated when hash matches (no-op)"
    );
}

/// Application-level test: verifies the #1052 upsert updates rows when
/// the hash changes (content-changed re-extraction).
#[test]
fn kg_extractions_upsert_updates_on_hash_mismatch() {
    let (_dir, conn) = build_fresh_db().unwrap();

    // Seed a row with an old hash
    conn.execute(
        "INSERT INTO kg_extractions (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model)
         VALUES ('0000000000000000', '/test/docs', 'doc1.md', 'old-hash', 'old-model')",
        [],
    )
    .unwrap();

    // Upsert with a different hash — WHERE clause should fire
    conn.execute(
        "INSERT INTO kg_extractions
            (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model)
         VALUES ('0000000000000000', '/test/docs', 'doc1.md', 'new-hash', 'new-model')
         ON CONFLICT(docs_root_hash, source_doc_path) DO UPDATE SET
            source_doc_hash = excluded.source_doc_hash,
            extraction_model = excluded.extraction_model,
            entities_extracted = excluded.entities_extracted,
            relationships_extracted = excluded.relationships_extracted,
            extraction_trace_id = excluded.extraction_trace_id,
            created_at = excluded.created_at
         WHERE kg_extractions.source_doc_hash IS NULL
            OR kg_extractions.source_doc_hash != excluded.source_doc_hash",
        [],
    )
    .unwrap();

    let (hash, model): (String, String) = conn
        .query_row(
            "SELECT source_doc_hash, extraction_model FROM kg_extractions
             WHERE docs_root_hash = '0000000000000000' AND source_doc_path = 'doc1.md'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(hash, "new-hash", "Hash must be updated on mismatch");
    assert_eq!(model, "new-model", "Model must be updated on mismatch");
}

// ===== Column-level structural verification for KG tables =====

#[test]
fn kg_chunks_v27_column_structure() {
    let (_dir, conn) = build_fresh_db().unwrap();
    let cols = get_columns(&conn, "kg_chunks").unwrap();
    let col_names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();

    assert!(col_names.contains(&"id"), "kg_chunks missing id");
    assert!(
        col_names.contains(&"docs_root_hash"),
        "kg_chunks missing docs_root_hash"
    );
    assert!(
        col_names.contains(&"docs_root"),
        "kg_chunks missing docs_root"
    );
    assert!(col_names.contains(&"seq_id"), "kg_chunks missing seq_id");
    assert!(
        col_names.contains(&"source_doc_path"),
        "kg_chunks missing source_doc_path"
    );
    assert!(
        col_names.contains(&"source_doc_hash"),
        "kg_chunks missing source_doc_hash"
    );
    assert!(
        col_names.contains(&"created_at"),
        "kg_chunks missing created_at"
    );
    assert!(
        col_names.contains(&"trace_id"),
        "kg_chunks missing trace_id"
    );
    assert!(
        !col_names.contains(&"agent_id"),
        "kg_chunks has forbidden agent_id"
    );
}

#[test]
fn kg_subject_entities_v27_column_structure() {
    let (_dir, conn) = build_fresh_db().unwrap();
    let cols = get_columns(&conn, "kg_subject_entities").unwrap();
    let col_names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();

    assert!(col_names.contains(&"docs_root_hash"));
    assert!(col_names.contains(&"docs_root"));
    assert!(col_names.contains(&"entity_key"));
    assert!(col_names.contains(&"type"));
    assert!(col_names.contains(&"name"));
    assert!(col_names.contains(&"confidence"));
    assert!(col_names.contains(&"properties_json"));
    assert!(!col_names.contains(&"agent_id"));
}

#[test]
fn kg_extractions_v27_column_structure() {
    let (_dir, conn) = build_fresh_db().unwrap();
    let cols = get_columns(&conn, "kg_extractions").unwrap();
    let col_names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();

    assert!(col_names.contains(&"docs_root_hash"));
    assert!(col_names.contains(&"docs_root"));
    assert!(col_names.contains(&"source_doc_path"));
    assert!(col_names.contains(&"source_doc_hash"));
    assert!(col_names.contains(&"extraction_model"));
    assert!(col_names.contains(&"entities_extracted"));
    assert!(col_names.contains(&"relationships_extracted"));
    assert!(col_names.contains(&"extraction_trace_id"));
    assert!(!col_names.contains(&"agent_id"));
}

// ===== v28: agent_kg_corpora table structure =====

#[test]
fn agent_kg_corpora_v28_column_structure() {
    let (_dir, conn) = build_fresh_db().unwrap();
    let cols = get_columns(&conn, "agent_kg_corpora").unwrap();
    let col_names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();

    assert!(
        col_names.contains(&"agent_id"),
        "agent_kg_corpora missing agent_id"
    );
    assert!(
        col_names.contains(&"docs_root_hash"),
        "agent_kg_corpora missing docs_root_hash"
    );
    assert!(
        col_names.contains(&"docs_root_path"),
        "agent_kg_corpora missing docs_root_path"
    );
    assert!(
        col_names.contains(&"created_at"),
        "agent_kg_corpora missing created_at"
    );
}

#[test]
fn agent_kg_corpora_v28_index_exists() {
    let (_dir, conn) = build_fresh_db().unwrap();
    let indexes = get_index_names(&conn, "agent_kg_corpora");
    assert!(
        indexes.contains("idx_agent_kg_corpora_hash"),
        "agent_kg_corpora must have idx_agent_kg_corpora_hash index, got: {indexes:?}"
    );
}

#[test]
fn agent_kg_corpora_v28_fk_cascade() {
    let (_dir, conn) = build_fresh_db().unwrap();

    // Enable FK enforcement
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

    // Insert agent
    conn.execute(
        "INSERT INTO agents (id, name, home_dir) VALUES ('test-cascade', 'Test', '')",
        [],
    )
    .unwrap();

    // Insert corpus mapping
    conn.execute(
        "INSERT INTO agent_kg_corpora (agent_id, docs_root_hash, docs_root_path) VALUES ('test-cascade', 'aaaa', '/test')",
        [],
    )
    .unwrap();

    // Delete agent — corpus row should cascade
    conn.execute("DELETE FROM agents WHERE id = 'test-cascade'", [])
        .unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM agent_kg_corpora WHERE agent_id = 'test-cascade'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "agent_kg_corpora rows must cascade on agent delete"
    );
}
