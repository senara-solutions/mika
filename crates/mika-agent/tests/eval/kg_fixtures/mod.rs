//! # KG Fixture Helpers — Shared Seeding for KG Eval Scenarios
//!
//! Crate-shared helpers for seeding a known KG state in test databases.
//! Used by `#740` (self-knowledge) and available for `#741` (grounding) scenarios.
//!
//! ## Entity Key Format
//!
//! Entity keys follow the `<type>:<name>` convention from
//! `crates/mika-agent/src/kg/domain_builder.rs` and `docs/architecture/kg-id-convention.md`.
//!
//! ## Schema Pin
//!
//! Fixtures are pinned to the current KG schema version. If the schema advances,
//! the assertion below fails with an actionable message pointing here.
//!
//! v26 (#757) added `kg_extractions.source_doc_hash` — no fixture-helper signature
//! change needed because `seed_*` helpers write via positional SQL that ignores
//! the new column (it stays NULL for fixture rows, which is the correct default).

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{CURRENT_SCHEMA_VERSION, Database};

/// Current schema version this fixture module is pinned to.
/// Bump this when updating fixtures for a new schema version.
/// Type is i64 to match db.rs::CURRENT_SCHEMA_VERSION source of truth.
const PINNED_SCHEMA_VERSION: i64 = 38;

const _: () = assert!(
    CURRENT_SCHEMA_VERSION == PINNED_SCHEMA_VERSION,
    "KG eval fixtures pin out of sync with db.rs CURRENT_SCHEMA_VERSION. \
     Bump PINNED_SCHEMA_VERSION in tests/eval/kg_fixtures/mod.rs and update \
     seed_* helpers. See docs/plans/740-kg-self-knowledge-eval.md D5.",
);

/// Default docs_root_hash for test fixtures. 16 hex chars matching the
/// `hash_docs_root` format. Used by seed_* helpers for shared-layer tables.
pub const TEST_DOCS_ROOT_HASH: &str = "0000000000000000";

/// Default docs_root display string for test fixtures.
pub const TEST_DOCS_ROOT: &str = "/test/docs/solutions";

// ---------------------------------------------------------------------------
// Test DB Factory
// ---------------------------------------------------------------------------

/// Create an in-memory `AsyncDatabase` with a test agent and session.
///
/// Agent ID defaults to `"mika-dev"` — the primary development agent.
/// Session `"eval-kg-session"` is pre-created for FK satisfaction.
pub fn test_db() -> AsyncDatabase {
    test_db_with_agent("mika-dev")
}

/// Create an in-memory `AsyncDatabase` with a specific agent ID.
pub fn test_db_with_agent(agent_id: &str) -> AsyncDatabase {
    let db = Database::open_in_memory().unwrap();
    // Register agent first (sessions FK to agents)
    db.register_agent(agent_id, agent_id, &format!("/tmp/mika-test/{agent_id}"))
        .unwrap();
    // Seed default corpus mapping so query fan-out finds this agent's KG data.
    db.register_agent_corpus(agent_id, TEST_DOCS_ROOT_HASH, TEST_DOCS_ROOT)
        .unwrap();
    db.create_session("eval-kg-session", agent_id, "test")
        .unwrap();
    AsyncDatabase::new_with_agent(db, agent_id)
}

/// Assert the schema version matches the fixture pin.
/// Call from scenario setup to catch schema drift early.
pub async fn assert_schema_version(db: &AsyncDatabase) {
    let version: i64 = db
        .with_db(|db| {
            db.query_scalar::<i64>("SELECT COALESCE(MAX(version), 0) FROM schema_version", &[])
                .map(|v| v.unwrap_or(0))
        })
        .await
        .unwrap();

    assert_eq!(
        version, PINNED_SCHEMA_VERSION,
        "KG eval fixtures pinned to schema v{PINNED_SCHEMA_VERSION}. Schema bumped to v{version}; \
         update seed_* helpers in tests/eval/kg_fixtures/mod.rs and bump this pin. \
         See docs/plans/740-kg-self-knowledge-eval.md D5.",
    );
}

// ---------------------------------------------------------------------------
// Seed Helpers
// ---------------------------------------------------------------------------

/// Seed an agent-corpus mapping into `agent_kg_corpora`. Idempotent.
pub async fn seed_agent_corpus(
    db: &AsyncDatabase,
    agent_id: &str,
    docs_root_hash: &str,
    docs_root_path: &str,
) {
    let agent_id = agent_id.to_string();
    let hash = docs_root_hash.to_string();
    let path = docs_root_path.to_string();

    db.with_db(move |db| db.register_agent_corpus(&agent_id, &hash, &path))
        .await
        .unwrap();
}

/// Specification for a domain entity seed.
pub struct DomainEntitySpec {
    pub entity_type: &'static str,
    pub name: &'static str,
    pub properties_json: Option<&'static str>,
}

/// Seed a domain entity into `kg_entities`. Returns the inserted row ID.
pub async fn seed_domain_entity(db: &AsyncDatabase, spec: &DomainEntitySpec) -> i64 {
    let entity_key = format!("{}:{}", spec.entity_type, spec.name);
    let etype = spec.entity_type.to_string();
    let name = spec.name.to_string();
    let props = spec.properties_json.map(|s| s.to_string());

    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_entities (entity_key, type, name, properties_json) \
             VALUES (?1, ?2, ?3, ?4)",
            &[
                &entity_key as &dyn rusqlite::types::ToSql,
                &etype,
                &name,
                &props,
            ],
        )?;
        Ok(db.last_insert_rowid())
    })
    .await
    .unwrap()
}

/// Seed a domain relationship into `kg_relationships`. Returns the inserted row ID.
pub async fn seed_domain_relationship(
    db: &AsyncDatabase,
    from_entity_id: i64,
    to_entity_id: i64,
    rel_type: &str,
) -> i64 {
    let rt = rel_type.to_string();
    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_relationships (from_entity_id, to_entity_id, type) \
             VALUES (?1, ?2, ?3)",
            &[
                &from_entity_id as &dyn rusqlite::types::ToSql,
                &to_entity_id,
                &rt,
            ],
        )?;
        Ok(db.last_insert_rowid())
    })
    .await
    .unwrap()
}

/// Specification for a subject entity seed.
pub struct SubjectEntitySpec {
    pub entity_type: &'static str,
    pub name: &'static str,
    pub confidence: f64,
    pub properties_json: Option<&'static str>,
}

/// Seed a subject entity into `kg_subject_entities`. Returns the inserted row ID.
///
/// Uses `TEST_DOCS_ROOT_HASH` and `TEST_DOCS_ROOT` for shared-layer scoping (v27).
pub async fn seed_subject_entity(db: &AsyncDatabase, spec: &SubjectEntitySpec) -> i64 {
    let docs_root_hash = TEST_DOCS_ROOT_HASH.to_string();
    let docs_root = TEST_DOCS_ROOT.to_string();
    let entity_key = format!("{}:{}", spec.entity_type, spec.name);
    let etype = spec.entity_type.to_string();
    let name = spec.name.to_string();
    let confidence = spec.confidence;
    let props = spec.properties_json.map(|s| s.to_string());

    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence, properties_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            &[
                &docs_root_hash as &dyn rusqlite::types::ToSql,
                &docs_root,
                &entity_key,
                &etype,
                &name,
                &confidence,
                &props,
            ],
        )?;
        Ok(db.last_insert_rowid())
    })
    .await
    .unwrap()
}

/// Specification for a chunk seed.
pub struct ChunkSpec {
    pub seq_id: i64,
    pub source_doc_path: &'static str,
    pub source_doc_hash: &'static str,
    pub text: &'static str,
}

/// Seed a chunk into `kg_chunks` and index its text via `Database::index_content`.
/// Returns the chunk row ID.
///
/// Mirrors the lexical ingestor's insert pattern (kg_chunks + index_content in the
/// same transaction) so FTS5 (`fts_search`) — and vec_search when embeddings are
/// supplied — stay in parity with `search_content`. Path C semantic retrieval
/// depends on `fts_search` being populated; a raw insert into `search_content`
/// silently breaks Path C tests.
///
/// Uses `TEST_DOCS_ROOT_HASH` and `TEST_DOCS_ROOT` for shared-layer scoping (v27).
/// Agent ID from `db.agent_id` is still used for `index_content` (search_content table).
pub async fn seed_chunk(db: &AsyncDatabase, spec: &ChunkSpec) -> i64 {
    let agent_id = db.agent_id.clone();
    let docs_root_hash = TEST_DOCS_ROOT_HASH.to_string();
    let docs_root = TEST_DOCS_ROOT.to_string();
    let seq_id = spec.seq_id;
    let source_doc_path = spec.source_doc_path.to_string();
    let source_doc_hash = spec.source_doc_hash.to_string();
    let text = spec.text.to_string();

    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            &[
                &docs_root_hash as &dyn rusqlite::types::ToSql,
                &docs_root,
                &seq_id,
                &source_doc_path,
                &source_doc_hash,
            ],
        )?;
        let chunk_id = db.last_insert_rowid();

        // Index via the canonical path: updates search_content + fts_search
        // (and vec_search via index_embedding when embeddings are available).
        // search_content still uses agent_id for its own scoping.
        db.index_content(
            &agent_id,
            mika_agent::db::kg_schema::KG_CHUNK_SOURCE_TYPE,
            Some(chunk_id),
            &text,
        )?;

        Ok(chunk_id)
    })
    .await
    .unwrap()
}

/// Link a chunk to a subject entity via `kg_chunk_subjects`. Returns the row ID.
pub async fn seed_chunk_subject(db: &AsyncDatabase, chunk_id: i64, subject_entity_id: i64) -> i64 {
    let docs_root_hash = TEST_DOCS_ROOT_HASH.to_string();
    let docs_root = TEST_DOCS_ROOT.to_string();

    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_chunk_subjects (docs_root_hash, docs_root, chunk_id, subject_entity_id) \
             VALUES (?1, ?2, ?3, ?4)",
            &[
                &docs_root_hash as &dyn rusqlite::types::ToSql,
                &docs_root,
                &chunk_id,
                &subject_entity_id,
            ],
        )?;
        Ok(db.last_insert_rowid())
    })
    .await
    .unwrap()
}

/// Seed a resolution into both `kg_subject_resolutions` and `kg_resolutions_log`.
pub async fn seed_resolution(
    db: &AsyncDatabase,
    subject_entity_id: i64,
    domain_entity_id: i64,
    confidence: f64,
    outcome: &str,
) {
    let agent_id = db.agent_id.clone();
    let outcome = outcome.to_string();
    let trace_id = uuid::Uuid::new_v4().to_string().replace('-', "");

    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO kg_subject_resolutions (agent_id, subject_entity_id, domain_entity_id, confidence) \
             VALUES (?1, ?2, ?3, ?4)",
            &[
                &agent_id as &dyn rusqlite::types::ToSql,
                &subject_entity_id,
                &domain_entity_id,
                &confidence,
            ],
        )?;
        db.execute_sql(
            "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
             VALUES (?1, ?2, ?3, ?4)",
            &[
                &agent_id as &dyn rusqlite::types::ToSql,
                &subject_entity_id,
                &outcome,
                &trace_id,
            ],
        )?;
        Ok(())
    })
    .await
    .unwrap();
}

/// Disable a skill for a specific agent via `skill_overrides`.
pub async fn disable_skill(db: &AsyncDatabase, agent_id: &str, skill_name: &str) {
    let aid = agent_id.to_string();
    let sn = skill_name.to_string();

    db.with_db(move |db| {
        db.execute_sql(
            "INSERT INTO skill_overrides (agent_id, skill_name, enabled) \
             VALUES (?1, ?2, 0) \
             ON CONFLICT(agent_id, skill_name) DO UPDATE SET enabled = 0",
            &[&aid as &dyn rusqlite::types::ToSql, &sn],
        )?;
        Ok(())
    })
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// Query Helpers (for assertions)
// ---------------------------------------------------------------------------

/// Read a resolution log entry for a subject entity. Returns (outcome, model).
pub async fn get_resolution_log(
    db: &AsyncDatabase,
    subject_entity_id: i64,
) -> Option<(String, Option<String>)> {
    let agent_id = db.agent_id.clone();

    db.with_db(move |db| {
        db.query_row_2::<String, Option<String>>(
            "SELECT outcome, model FROM kg_resolutions_log \
             WHERE agent_id = ?1 AND subject_entity_id = ?2",
            &[&agent_id as &dyn rusqlite::types::ToSql, &subject_entity_id],
        )
    })
    .await
    .unwrap()
}

/// Read a resolution for a subject entity. Returns (domain_entity_id, confidence).
pub async fn get_resolution(db: &AsyncDatabase, subject_entity_id: i64) -> Option<(i64, f64)> {
    let agent_id = db.agent_id.clone();

    db.with_db(move |db| {
        db.query_row_2::<i64, f64>(
            "SELECT domain_entity_id, confidence FROM kg_subject_resolutions \
             WHERE agent_id = ?1 AND subject_entity_id = ?2",
            &[&agent_id as &dyn rusqlite::types::ToSql, &subject_entity_id],
        )
    })
    .await
    .unwrap()
}

// ---------------------------------------------------------------------------
// v26→v27 Migration Test Infrastructure (#787)
// ---------------------------------------------------------------------------
//
// The helpers below create raw `rusqlite::Connection` databases with a frozen
// v26 KG schema, seed synthetic data, then leave the database in the exact
// state that `v27_coalesce_sql` expects: v26 tables renamed to *_v26_backup,
// v27 tables created empty, schema_meta present but no coalesce marker.
//
// These helpers are intentionally separate from the async `test_db()` factory
// above because:
//   1. The coalesce SQL is a single `execute_batch` — no async needed.
//   2. We must NOT call `init_sqlite_vec()` (requires the C extension).
//   3. We must NOT call `Database::open_in_memory()` (runs full migration).

/// v26 KG table DDL — frozen snapshot from before #786. Used by #787 migration tests.
/// Do NOT update when the schema version increments.
pub const V26_KG_DDL: &str = "
    CREATE TABLE IF NOT EXISTS agents (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        home_dir TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS schema_version (
        version INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS kg_entities (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        entity_key TEXT NOT NULL UNIQUE,
        type TEXT NOT NULL,
        name TEXT NOT NULL,
        properties_json TEXT,
        created_at TEXT,
        updated_at TEXT,
        CHECK (entity_key = type || ':' || name)
    );

    CREATE TABLE IF NOT EXISTS kg_relationships (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        from_entity_id INTEGER,
        to_entity_id INTEGER,
        type TEXT,
        properties_json TEXT,
        created_at TEXT
    );

    CREATE TABLE IF NOT EXISTS kg_chunks (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        seq_id INTEGER NOT NULL,
        source_doc_path TEXT NOT NULL,
        source_doc_hash TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        trace_id TEXT,
        UNIQUE (agent_id, source_doc_path, seq_id)
    );

    CREATE TABLE IF NOT EXISTS kg_subject_entities (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        entity_key TEXT NOT NULL,
        type TEXT NOT NULL,
        name TEXT NOT NULL,
        confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
        properties_json TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        trace_id TEXT,
        CHECK (entity_key = type || ':' || name),
        UNIQUE (agent_id, entity_key)
    );

    CREATE TABLE IF NOT EXISTS kg_subject_relationships (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        from_entity_id INTEGER NOT NULL,
        to_entity_id INTEGER NOT NULL,
        type TEXT NOT NULL,
        confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
        properties_json TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        trace_id TEXT,
        UNIQUE (agent_id, from_entity_id, to_entity_id, type)
    );

    CREATE TABLE IF NOT EXISTS kg_chunk_subjects (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        chunk_id INTEGER NOT NULL,
        subject_entity_id INTEGER NOT NULL,
        extraction_trace_id TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        UNIQUE (agent_id, chunk_id, subject_entity_id)
    );

    CREATE TABLE IF NOT EXISTS kg_chunk_subject_relationships (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        chunk_id INTEGER NOT NULL,
        subject_relationship_id INTEGER NOT NULL,
        extraction_trace_id TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        UNIQUE (agent_id, chunk_id, subject_relationship_id)
    );

    CREATE TABLE IF NOT EXISTS kg_extractions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        source_doc_path TEXT NOT NULL,
        source_doc_hash TEXT,
        extraction_model TEXT NOT NULL,
        entities_extracted INTEGER NOT NULL DEFAULT 0,
        relationships_extracted INTEGER NOT NULL DEFAULT 0,
        extraction_trace_id TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        UNIQUE (agent_id, source_doc_path)
    );

    CREATE TABLE IF NOT EXISTS kg_subject_resolutions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        subject_entity_id INTEGER NOT NULL,
        domain_entity_id INTEGER NOT NULL,
        confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        trace_id TEXT,
        UNIQUE (agent_id, subject_entity_id, domain_entity_id)
    );

    CREATE TABLE IF NOT EXISTS kg_resolutions_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        subject_entity_id INTEGER NOT NULL,
        outcome TEXT NOT NULL CHECK (outcome IN (
            'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
        )),
        resolution_trace_id TEXT NOT NULL,
        source_extraction_trace_id TEXT,
        model TEXT,
        duration_ms INTEGER,
        resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        UNIQUE (agent_id, subject_entity_id)
    );

    CREATE TABLE IF NOT EXISTS schema_meta (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
";

/// v27 DDL for KG tables — the target tables created by migrate_v26_to_v27.
/// Matches the exact DDL from db.rs. Used after renaming v26 tables to *_v26_backup.
const V27_KG_DDL: &str = "
    CREATE TABLE kg_chunks (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        docs_root_hash TEXT NOT NULL,
        docs_root TEXT NOT NULL,
        seq_id INTEGER NOT NULL,
        source_doc_path TEXT NOT NULL,
        source_doc_hash TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        trace_id TEXT,
        UNIQUE (docs_root_hash, source_doc_path, seq_id)
    );

    CREATE TABLE kg_subject_entities (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        docs_root_hash TEXT NOT NULL,
        docs_root TEXT NOT NULL,
        entity_key TEXT NOT NULL,
        type TEXT NOT NULL,
        name TEXT NOT NULL,
        confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
        properties_json TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        trace_id TEXT,
        CHECK (entity_key = type || ':' || name),
        UNIQUE (docs_root_hash, entity_key)
    );

    CREATE TABLE kg_subject_relationships (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        docs_root_hash TEXT NOT NULL,
        docs_root TEXT NOT NULL,
        from_entity_id INTEGER NOT NULL,
        to_entity_id INTEGER NOT NULL,
        type TEXT NOT NULL,
        confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
        properties_json TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        trace_id TEXT,
        UNIQUE (docs_root_hash, from_entity_id, to_entity_id, type)
    );

    CREATE TABLE kg_chunk_subjects (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        docs_root_hash TEXT NOT NULL,
        docs_root TEXT NOT NULL,
        chunk_id INTEGER NOT NULL,
        subject_entity_id INTEGER NOT NULL,
        extraction_trace_id TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        UNIQUE (docs_root_hash, chunk_id, subject_entity_id)
    );

    CREATE TABLE kg_chunk_subject_relationships (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        docs_root_hash TEXT NOT NULL,
        docs_root TEXT NOT NULL,
        chunk_id INTEGER NOT NULL,
        subject_relationship_id INTEGER NOT NULL,
        extraction_trace_id TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        UNIQUE (docs_root_hash, chunk_id, subject_relationship_id)
    );

    CREATE TABLE kg_extractions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        docs_root_hash TEXT NOT NULL,
        docs_root TEXT NOT NULL,
        source_doc_path TEXT NOT NULL,
        source_doc_hash TEXT,
        extraction_model TEXT NOT NULL,
        entities_extracted INTEGER NOT NULL DEFAULT 0,
        relationships_extracted INTEGER NOT NULL DEFAULT 0,
        extraction_trace_id TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        UNIQUE (docs_root_hash, source_doc_path)
    );

    CREATE TABLE kg_subject_resolutions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        subject_entity_id INTEGER NOT NULL,
        domain_entity_id INTEGER NOT NULL,
        confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        trace_id TEXT,
        UNIQUE (agent_id, subject_entity_id, domain_entity_id)
    );

    CREATE TABLE kg_resolutions_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        subject_entity_id INTEGER NOT NULL,
        outcome TEXT NOT NULL CHECK (outcome IN (
            'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
        )),
        resolution_trace_id TEXT NOT NULL,
        source_extraction_trace_id TEXT,
        model TEXT,
        duration_ms INTEGER,
        resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        UNIQUE (agent_id, subject_entity_id)
    );
";

/// Drift profiles for synthetic v26 data seeding.
pub enum DriftProfile {
    /// Every agent extracts the same entities with identical confidence.
    NoDrift,
    /// Agents extract overlapping but non-identical entity sets with varied confidence.
    ObservedDrift,
}

/// Create an in-memory connection with v26 KG schema + v27 empty tables.
/// Sets up the exact state after #786's DDL runs but before #787's coalesce:
/// - v26 tables renamed to *_v26_backup
/// - v27 tables created (empty)
/// - schema_meta table exists (no v27_coalesce_complete marker)
///
/// Does NOT call `init_sqlite_vec` — the C extension may not be linked in test context.
pub fn open_v26_for_coalesce() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory SQLite");
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("set pragmas");

    // 1. Create v26 schema tables
    conn.execute_batch(V26_KG_DDL).expect("create v26 tables");

    // 2. Rename v26 KG tables to *_v26_backup (same as DDL part of migrate_v26_to_v27)
    conn.execute_batch(
        "ALTER TABLE kg_chunks RENAME TO kg_chunks_v26_backup;
         ALTER TABLE kg_subject_entities RENAME TO kg_subject_entities_v26_backup;
         ALTER TABLE kg_subject_relationships RENAME TO kg_subject_relationships_v26_backup;
         ALTER TABLE kg_chunk_subjects RENAME TO kg_chunk_subjects_v26_backup;
         ALTER TABLE kg_chunk_subject_relationships RENAME TO kg_chunk_subject_relationships_v26_backup;
         ALTER TABLE kg_extractions RENAME TO kg_extractions_v26_backup;
         ALTER TABLE kg_subject_resolutions RENAME TO kg_subject_resolutions_v26_backup;
         ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v26_backup;",
    )
    .expect("rename v26 tables to backup");

    // 3. Create v27 empty tables
    conn.execute_batch(V27_KG_DDL).expect("create v27 tables");

    // 4. Insert schema_version 27
    conn.execute("INSERT INTO schema_version (version) VALUES (27)", [])
        .expect("insert schema_version");

    // 5. Turn FK back on
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable FK");

    conn
}

/// Seed a v26 synthetic database with realistic extraction drift.
/// Returns a Connection with v26 backup tables populated and v27 tables empty.
///
/// Parameters:
/// - agents: number of agents (each gets its own set of extracted entities)
/// - docs: number of docs in the shared corpus
/// - drift: controls entity extraction variance across agents
pub fn build_v26_synthetic_db(agents: u32, docs: u32, drift: DriftProfile) -> rusqlite::Connection {
    let conn = open_v26_for_coalesce();

    // FK off during bulk inserts to v26 backup tables (which have no FK constraints anyway)
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("disable FK for seeding");

    // --- Seed agents ---
    for a in 0..agents {
        let agent_id = format!("agent-{a}");
        conn.execute(
            "INSERT INTO agents (id, name, home_dir) VALUES (?1, ?2, ?3)",
            rusqlite::params![agent_id, agent_id, format!("/tmp/mika-test/{agent_id}")],
        )
        .expect("insert agent");
    }

    // --- Seed domain entities (for FK from resolutions) ---
    for i in 0..20 {
        let etype = match i % 4 {
            0 => "skill",
            1 => "tool",
            2 => "agent",
            _ => "problem_type",
        };
        let name = format!("domain-{i}");
        let entity_key = format!("{etype}:{name}");
        conn.execute(
            "INSERT INTO kg_entities (entity_key, type, name) VALUES (?1, ?2, ?3)",
            rusqlite::params![entity_key, etype, name],
        )
        .expect("insert domain entity");
    }

    // --- Seed per-agent data ---
    for a in 0..agents {
        let agent_id = format!("agent-{a}");

        for d in 0..docs {
            let doc_path = format!("docs/solutions/test-doc-{d}.md");
            let doc_hash = format!("{:016x}", (d as u64) * 0x1234_5678_9abc_def0 + 1);

            // Insert 5 chunks per doc
            let mut chunk_ids = Vec::new();
            for seq in 0..5 {
                conn.execute(
                    "INSERT INTO kg_chunks_v26_backup (agent_id, seq_id, source_doc_path, source_doc_hash) \
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![agent_id, seq, doc_path, doc_hash],
                )
                .expect("insert chunk");
                let chunk_id = conn.last_insert_rowid();
                chunk_ids.push(chunk_id);
            }

            // Generate entities based on drift profile
            let entities = generate_entities(a, d, &drift);

            let mut entity_ids = Vec::new();
            for ent in &entities {
                conn.execute(
                    "INSERT OR IGNORE INTO kg_subject_entities_v26_backup \
                     (agent_id, entity_key, type, name, confidence) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        agent_id,
                        ent.entity_key,
                        ent.etype,
                        ent.name,
                        ent.confidence
                    ],
                )
                .expect("insert subject entity");
                // Get the ID (may have been ignored if duplicate agent+entity_key)
                let eid: i64 = conn
                    .query_row(
                        "SELECT id FROM kg_subject_entities_v26_backup \
                         WHERE agent_id = ?1 AND entity_key = ?2",
                        rusqlite::params![agent_id, ent.entity_key],
                        |row| row.get(0),
                    )
                    .expect("get entity id");
                entity_ids.push(eid);
            }

            // Insert 3 relationships per doc (between first 3 entities, if enough exist)
            let mut rel_ids = Vec::new();
            if entity_ids.len() >= 3 {
                for r in 0..3 {
                    let from_id = entity_ids[r];
                    let to_id = entity_ids[(r + 1) % 3];
                    let rel_type = match r {
                        0 => "SOLVED_BY",
                        1 => "USES",
                        _ => "CALLS",
                    };
                    let confidence = 0.80 + (r as f64) * 0.05;
                    conn.execute(
                        "INSERT OR IGNORE INTO kg_subject_relationships_v26_backup \
                         (agent_id, from_entity_id, to_entity_id, type, confidence) \
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![agent_id, from_id, to_id, rel_type, confidence],
                    )
                    .expect("insert subject relationship");
                    let rid: i64 = conn
                        .query_row(
                            "SELECT id FROM kg_subject_relationships_v26_backup \
                             WHERE agent_id = ?1 AND from_entity_id = ?2 AND to_entity_id = ?3 AND type = ?4",
                            rusqlite::params![agent_id, from_id, to_id, rel_type],
                            |row| row.get(0),
                        )
                        .expect("get relationship id");
                    rel_ids.push(rid);
                }
            }

            // Link chunks to entities via kg_chunk_subjects
            for (i, &chunk_id) in chunk_ids.iter().enumerate() {
                // Each chunk links to 2-3 entities
                let link_count = std::cmp::min(entity_ids.len(), if i < 3 { 3 } else { 2 });
                for j in 0..link_count {
                    let eid = entity_ids[j % entity_ids.len()];
                    conn.execute(
                        "INSERT OR IGNORE INTO kg_chunk_subjects_v26_backup \
                         (agent_id, chunk_id, subject_entity_id, extraction_trace_id) \
                         VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![
                            agent_id,
                            chunk_id,
                            eid,
                            format!("trace-{a}-{d}-{i}-{j}")
                        ],
                    )
                    .expect("insert chunk_subject");
                }
            }

            // Link chunks to relationships via kg_chunk_subject_relationships
            if !rel_ids.is_empty() {
                for (i, &chunk_id) in chunk_ids.iter().take(3).enumerate() {
                    let rid = rel_ids[i % rel_ids.len()];
                    conn.execute(
                        "INSERT OR IGNORE INTO kg_chunk_subject_relationships_v26_backup \
                         (agent_id, chunk_id, subject_relationship_id, extraction_trace_id) \
                         VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![
                            agent_id,
                            chunk_id,
                            rid,
                            format!("trace-rel-{a}-{d}-{i}")
                        ],
                    )
                    .expect("insert chunk_subject_relationship");
                }
            }

            // Insert 1 extraction record per (agent, doc)
            conn.execute(
                "INSERT OR IGNORE INTO kg_extractions_v26_backup \
                 (agent_id, source_doc_path, source_doc_hash, extraction_model, \
                  entities_extracted, relationships_extracted, extraction_trace_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    agent_id,
                    doc_path,
                    doc_hash,
                    "openrouter/deepseek/deepseek-v3",
                    entities.len() as i64,
                    std::cmp::min(rel_ids.len(), 3) as i64,
                    format!("extraction-trace-{a}-{d}")
                ],
            )
            .expect("insert extraction");
        }
    }

    // --- Seed resolutions (for ~70% of each agent's entities) ---
    // Only create resolutions for the FIRST entity per agent per normalized key
    // to avoid collision when case variants both resolve to the same domain entity.
    for a in 0..agents {
        let agent_id = format!("agent-{a}");

        // Collect all entity IDs and their normalized keys for this agent
        let mut stmt = conn
            .prepare(
                "SELECT id, entity_key FROM kg_subject_entities_v26_backup \
                 WHERE agent_id = ?1 ORDER BY id",
            )
            .expect("prepare entity query");
        let entities: Vec<(i64, String)> = stmt
            .query_map(rusqlite::params![agent_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .expect("query entities")
            .filter_map(|r| r.ok())
            .collect();

        // Track which normalized keys we've already resolved for this agent
        let mut resolved_norm_keys: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (i, (entity_id, entity_key)) in entities.iter().enumerate() {
            // ~70% resolution rate
            if (i * 7) % 10 >= 7 {
                continue;
            }

            let norm_key = entity_key.trim().to_lowercase();
            if resolved_norm_keys.contains(&norm_key) {
                continue; // Skip case variants — avoid collision on INSERT OR IGNORE
            }
            resolved_norm_keys.insert(norm_key);

            let domain_entity_id = (i as i64 % 20) + 1; // Map to one of the 20 domain entities
            let confidence = 0.85;
            let trace_id = format!("res-trace-{a}-{i}");

            conn.execute(
                "INSERT OR IGNORE INTO kg_subject_resolutions_v26_backup \
                 (agent_id, subject_entity_id, domain_entity_id, confidence, trace_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![agent_id, entity_id, domain_entity_id, confidence, trace_id],
            )
            .expect("insert resolution");

            conn.execute(
                "INSERT OR IGNORE INTO kg_resolutions_log_v26_backup \
                 (agent_id, subject_entity_id, outcome, resolution_trace_id) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![agent_id, entity_id, "matched_exact", trace_id],
            )
            .expect("insert resolution log");
        }
    }

    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("re-enable FK");

    conn
}

/// Internal entity spec for seeding.
struct EntitySpec {
    entity_key: String,
    etype: String,
    name: String,
    confidence: f64,
}

/// Generate entities for a given agent and doc based on drift profile.
fn generate_entities(agent_idx: u32, doc_idx: u32, drift: &DriftProfile) -> Vec<EntitySpec> {
    match drift {
        DriftProfile::NoDrift => {
            // Every agent extracts the same 10 entities with identical confidence
            (0..10)
                .map(|i| {
                    let name = format!("entity-{doc_idx}-{i}");
                    EntitySpec {
                        entity_key: format!("skill:{name}"),
                        etype: "skill".to_string(),
                        name,
                        confidence: 0.85,
                    }
                })
                .collect()
        }
        DriftProfile::ObservedDrift => {
            let mut entities = Vec::new();
            let seed = (agent_idx as usize) * 1000 + (doc_idx as usize);

            // True set: 30 entities per doc
            // Each agent draws 8-12 from the true set (deterministic selection)
            let draw_count = 8 + (seed % 5); // 8..12
            for i in 0..30 {
                // Deterministic selection: use a simple hash-like function
                let selected = ((seed * 31 + i * 17) % 30) < draw_count;
                if !selected {
                    continue;
                }
                let name = format!("entity-{doc_idx}-{i}");
                let confidence = 0.70 + ((i % 3) as f64) * 0.10;
                // Apply per-agent confidence jitter: ±0.10
                let jitter = ((agent_idx as f64) - 1.0) * 0.05;
                let confidence = (confidence + jitter).clamp(0.01, 1.0);
                entities.push(EntitySpec {
                    entity_key: format!("skill:{name}"),
                    etype: "skill".to_string(),
                    name,
                    confidence,
                });
            }

            // Agent-unique entities: 2-4 per (agent, doc)
            let unique_count = 2 + (seed % 3); // 2..4
            for i in 0..unique_count {
                let name = format!("unique-{agent_idx}-{doc_idx}-{i}");
                entities.push(EntitySpec {
                    entity_key: format!("skill:{name}"),
                    etype: "skill".to_string(),
                    name,
                    confidence: 0.75,
                });
            }

            // Case variants: 1-2 per (agent, doc)
            let variant_count = 1 + (seed % 2); // 1..2
            for i in 0..variant_count {
                // Create case variant of an entity from the true set
                let target_idx = (seed + i) % 30;
                let name = format!("Entity-{doc_idx}-{target_idx}"); // uppercase E
                entities.push(EntitySpec {
                    entity_key: format!("skill:{name}"),
                    etype: "skill".to_string(),
                    name,
                    confidence: 0.72,
                });
            }

            entities
        }
    }
}
