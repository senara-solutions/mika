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
use mika_agent::db::Database;

/// Current schema version this fixture module is pinned to.
/// Bump this when updating fixtures for a new schema version.
const PINNED_SCHEMA_VERSION: i32 = 27;

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
    db.create_session("eval-kg-session", agent_id, "test")
        .unwrap();
    AsyncDatabase::new_with_agent(db, agent_id)
}

/// Assert the schema version matches the fixture pin.
/// Call from scenario setup to catch schema drift early.
pub async fn assert_schema_version(db: &AsyncDatabase) {
    let version: i32 = db
        .with_db(|db| {
            db.query_scalar::<i32>("SELECT COALESCE(MAX(version), 0) FROM schema_version", &[])
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
