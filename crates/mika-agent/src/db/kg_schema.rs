//! # KG Schema Constants
//!
//! Centralizes Knowledge Graph table names, column lists, type enums, and ID
//! convention constants. All migration code and row mappers import from here
//! instead of using inline strings or `SELECT *`.
//!
//! ## Schema migration pre-ship checklist
//!
//! Before shipping any migration that changes what "pending work" queries return,
//! consult `docs/solutions/best-practices/schema-migrations-as-integration-events-2026-04-23.md`
//! for the operational checklist (mika#767).
//!
//! ## Write Contract (for `kg_chunks` consumers)
//!
//! The `kg_chunks` table stores chunk structural metadata (entity link, seq_id,
//! source doc, agent_id). The **indexable text and embedding** flow through the
//! existing `search_content` + `fts_search` + `vec_search` pipeline via
//! [`index_content()`](super::Database::index_content):
//!
//! ```text
//! ingest_kg_chunk(agent_id, seq, doc_path, doc_hash, text):
//!     BEGIN TRANSACTION
//!         INSERT INTO kg_chunks(agent_id, seq_id, source_doc_path, source_doc_hash, trace_id)
//!             -> rowid = R
//!         index_content(
//!             agent_id,
//!             source_type = "kg_chunk",  // KG_CHUNK_SOURCE_TYPE
//!             source_id   = R,
//!             content     = text,
//!         )
//!             -> INSERT INTO search_content + INSERT INTO fts_search
//!             -> embedding queued for backfill (existing path)
//!     COMMIT
//! ```
//!
//! ### Invariants
//!
//! - **Transactional double-write:** `kg_chunks` INSERT and `index_content()` call
//!   MUST happen within the same transaction. If indexing fails, the chunk row
//!   rolls back. No orphan rows, no orphan indexes.
//! - **Idempotency:** The UNIQUE constraint on `(agent_id, source_doc_path, seq_id)`
//!   (see D7 in the plan) means re-ingesting the same chunk position uses
//!   `INSERT OR REPLACE` or explicit UPSERT.
//! - **Content-hash check:** `source_doc_hash` (SHA-256 of normalized content, D10)
//!   allows the ingestor to skip re-ingestion when the doc hasn't changed. The
//!   ingestor writes the **same hash on every chunk row of a given doc**, so a
//!   consumer can read the per-doc hash with `SELECT DISTINCT source_doc_hash
//!   FROM kg_chunks WHERE agent_id=? AND source_doc_path=?` (exactly one row).
//! - **Embedding backfill:** Embeddings are generated asynchronously by the existing
//!   startup backfill path (`embedding_json IS NULL` in `search_content`). The
//!   `kg_chunk` source_type is automatically included.
//!
//! ### Ownership (dual-write prevention)
//!
//! Each entity kind has a single persistence owner:
//! - Domain entities (`kg_entities`, `kg_relationships`): domain ingestor (#687)
//! - Chunks (`kg_chunks`): lexical ingestor (#689)
//! - Subject entities/relationships: subject extractor (#690)
//! - Resolution edges (`kg_subject_resolutions`): entity resolver (#691)
//!
//! ## Idempotency key (extraction, #757 + #786)
//!
//! Extraction is **whole-doc**: one LLM call per source doc, with `[CHUNK N]`
//! markers in the prompt (see `subject_extractor::extract_document`). The unit
//! of idempotency is therefore the doc, not the chunk. `kg_chunk_subjects`
//! is provenance data (which chunks mentioned which subject entities), not an
//! idempotency marker — per-chunk markers would be structurally redundant
//! given whole-doc extraction. See the #757 plan for the alternatives
//! considered.
//!
//! The idempotency contract is:
//!
//! - **Primary key (v27):** `UNIQUE(docs_root_hash, source_doc_path)` on
//!   `kg_extractions`. First-writer-wins via `INSERT OR IGNORE` — the first
//!   agent to extract a doc for a given corpus writes the marker; subsequent
//!   agents see "already extracted" and skip. Cost N× → 1× for identical
//!   corpora.
//! - **Staleness check (content hash):** `kg_extractions.source_doc_hash`
//!   stores the per-doc hash the extractor consumed on its last successful
//!   run. A doc is "pending" when it either has no `kg_extractions` row OR
//!   the row's hash disagrees with the current `kg_chunks.source_doc_hash`:
//!
//!   ```sql
//!   SELECT DISTINCT c.source_doc_path
//!   FROM kg_chunks c
//!   WHERE c.docs_root_hash = ?1
//!     AND NOT EXISTS (
//!       SELECT 1 FROM kg_extractions e
//!       WHERE e.docs_root_hash = c.docs_root_hash
//!         AND e.source_doc_path = c.source_doc_path
//!         AND e.source_doc_hash = c.source_doc_hash
//!     )
//!   ```
//!
//!   The direct hash equality works because the lexical ingestor writes one
//!   identical per-doc hash across every chunk row (#689). No aggregation
//!   needed.
//! - **NULL-hash first boot:** Pre-v26 rows (if any, from the v25 landing)
//!   have `source_doc_hash IS NULL`. The equality fails, so the doc is treated
//!   as pending exactly once. After that run, the hash is populated and
//!   subsequent runs are no-ops. The first-run cost is bounded by
//!   `MIKA_KG_BATCH_BUDGET` (default 500).
//!
//! ## Shared-corpus model (v27, #786)
//!
//! Six shared-layer tables (`kg_chunks`, `kg_subject_entities`,
//! `kg_subject_relationships`, `kg_chunk_subjects`,
//! `kg_chunk_subject_relationships`, `kg_extractions`) are keyed by
//! `docs_root_hash` — a 16-hex-char SHA-256 prefix of
//! `fs::canonicalize(docs_root)`. Agents with the same `docs_root` share
//! a single corpus; extraction runs once, not N× per agent.
//!
//! Per-agent tables (`kg_subject_resolutions`, `kg_resolutions_log`) keep
//! `agent_id` — resolution bridges subject entities to the agent's domain
//! graph, which is inherently per-agent.
//!
//! Domain tables (`kg_entities`, `kg_relationships`) are unchanged — no
//! agent_id, no docs_root_hash.
//!
//! The `docs_root_hash` is computed by [`crate::kg::config::hash_docs_root`].
//!
//! ## Cost model (post-v27)
//!
//! - **Extraction:** 1× per distinct `docs_root_hash`, regardless of agent
//!   count. First-writer-wins on `kg_extractions`.
//! - **Resolution:** N× per agent (per-agent resolution bridges).
//! - **Budget guard (`MIKA_KG_BATCH_BUDGET`, default 500).** Hard cap on LLM
//!   calls per extraction batch and per resolution batch. Worst-case cost per
//!   startup is `N_agents × budget` (resolution only; extraction is 1×).
//! - **Provider choice matters.** Anthropic is typically ~10× more expensive
//!   than OpenRouter equivalents for bulk NER. The server emits a
//!   `kg_anthropic_provider` WARN at startup when KG resolves to Anthropic so
//!   the cost is visible before it accrues.

/// Column list for `kg_entities` queries. No `SELECT *`.
pub const KG_ENTITY_COLUMNS: &str =
    "id, entity_key, type, name, properties_json, created_at, updated_at";

/// Column list for `kg_relationships` queries. No `SELECT *`.
pub const KG_RELATIONSHIP_COLUMNS: &str =
    "id, from_entity_id, to_entity_id, type, properties_json, created_at";

/// Column list for `kg_chunks` queries. No `SELECT *`.
/// v27: `agent_id` replaced by `docs_root_hash, docs_root`.
pub const KG_CHUNK_COLUMNS: &str =
    "id, docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash, created_at, trace_id";

/// Column list for `kg_subject_entities` queries. No `SELECT *`.
/// v27: `agent_id` replaced by `docs_root_hash, docs_root`.
pub const KG_SUBJECT_ENTITY_COLUMNS: &str = "id, docs_root_hash, docs_root, entity_key, type, name, confidence, properties_json, created_at, trace_id";

/// Column list for `kg_subject_resolutions` queries. No `SELECT *`.
/// Per-agent — keeps `agent_id` in v27.
pub const KG_SUBJECT_RESOLUTION_COLUMNS: &str =
    "id, agent_id, subject_entity_id, domain_entity_id, confidence, created_at, trace_id";

/// Column list for `kg_subject_relationships` queries. No `SELECT *`.
/// v27: `agent_id` replaced by `docs_root_hash, docs_root`.
pub const KG_SUBJECT_RELATIONSHIP_COLUMNS: &str = "id, docs_root_hash, docs_root, from_entity_id, to_entity_id, type, confidence, properties_json, created_at, trace_id";

/// Column list for `kg_chunk_subjects` queries. No `SELECT *`.
/// v27: `agent_id` replaced by `docs_root_hash, docs_root`.
pub const KG_CHUNK_SUBJECT_COLUMNS: &str =
    "id, docs_root_hash, docs_root, chunk_id, subject_entity_id, extraction_trace_id, created_at";

/// Column list for `kg_chunk_subject_relationships` queries. No `SELECT *`.
/// v27: `agent_id` replaced by `docs_root_hash, docs_root`.
pub const KG_CHUNK_SUBJECT_RELATIONSHIP_COLUMNS: &str = "id, docs_root_hash, docs_root, chunk_id, subject_relationship_id, extraction_trace_id, created_at";

/// Column list for `kg_extractions` queries. No `SELECT *`.
/// v27: `agent_id` replaced by `docs_root_hash, docs_root`. First-writer-wins
/// via `INSERT OR IGNORE` against `UNIQUE(docs_root_hash, source_doc_path)`.
pub const KG_EXTRACTION_COLUMNS: &str = "id, docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model, entities_extracted, relationships_extracted, extraction_trace_id, created_at";

/// Column list for `kg_resolutions_log` queries. No `SELECT *`.
pub const KG_RESOLUTION_LOG_COLUMNS: &str = "id, agent_id, subject_entity_id, outcome, resolution_trace_id, source_extraction_trace_id, model, duration_ms, resolved_at";

/// The `source_type` discriminator used when writing KG chunks into
/// `search_content` via [`index_content()`](super::Database::index_content).
pub const KG_CHUNK_SOURCE_TYPE: &str = "kg_chunk";

/// Reserved domain entity types for the KG domain layer.
///
/// This constant is the **single source of truth** for the reserved type list.
/// The documentation at `docs/architecture/kg-id-convention.md` is derived from
/// it. When adding a new domain entity type, update this array first, then
/// update the doc.
///
/// Subject-layer entities (per-agent, LLM-extracted) may use these types when
/// the mention resolves to a domain entity, or use subject-only types like
/// `failure_mode`, `solution_path`, etc.
pub const KG_DOMAIN_ENTITY_TYPES: &[&str] = &["skill", "tool", "agent", "problem_type", "concept"];

/// Format an entity key from type and name.
///
/// Returns `"{type}:{name}"` — the same format enforced by the CHECK constraint
/// on `kg_entities.entity_key` and `kg_subject_entities.entity_key`.
///
/// This helper does **no validation** of the inputs. Callers are responsible for
/// ensuring `kind` is a valid entity type and `name` is non-empty, lowercase,
/// and matches `[a-z0-9_-]+`. Semantic validation belongs at the tool/ingestor
/// boundary, not here.
pub fn format_entity_key(kind: &str, name: &str) -> String {
    format!("{kind}:{name}")
}

// ---------------------------------------------------------------------------
// Invalidation marker helpers (#961)
// ---------------------------------------------------------------------------

/// Record subject entity IDs whose `no_match` resolution log rows were
/// deleted by domain-graph rebuild invalidation (#960). Uses `INSERT OR
/// IGNORE` so duplicate markers from crash-restart-crash-restart are safe.
///
/// Called by `domain_builder::rebuild()` inside its transaction, after
/// deleting the `kg_resolutions_log` rows.
pub fn record_invalidated_no_match(
    conn: &rusqlite::Connection,
    agent_id: &str,
    subject_entity_ids: &[i64],
) -> rusqlite::Result<usize> {
    let mut total = 0usize;
    let mut stmt = conn.prepare_cached(
        "INSERT OR IGNORE INTO kg_invalidated_no_match (subject_entity_id, agent_id) VALUES (?1, ?2)",
    )?;
    for &id in subject_entity_ids {
        total += stmt.execute(rusqlite::params![id, agent_id])?;
    }
    Ok(total)
}

/// Rolling-window outcome breakdown from `kg_resolutions_log`.
///
/// Used by `mika kg status` (AC1/AC2 of mika#1077) and the resolver tick
/// alert (AC3). The `attempted` denominator excludes structural skips
/// (`skipped_no_llm`, `skipped_discovered_type`, `skipped_discovered_subject`)
/// and `error` — only outcomes that represent a genuine resolution attempt
/// are counted. See plan design decision D6 for rationale.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ResolutionOutcomeStats {
    /// All rows in window (informational — includes skips and errors).
    pub total: u64,
    /// Attempted resolution outcomes only (denominator for rate calculation).
    /// = matched_exact + matched_llm + matched_llm_db_fallback + no_match + no_candidate_of_type
    pub attempted: u64,
    pub no_match: u64,
    pub no_candidate_of_type: u64,
    pub matched_exact: u64,
    pub matched_llm: u64,
    pub matched_llm_db_fallback: u64,
    /// Structural skips (excluded from rate denominator).
    pub skipped: u64,
    /// Infrastructure failures (excluded from rate denominator).
    pub errors: u64,
}

impl ResolutionOutcomeStats {
    /// `no_match / attempted`. Returns 0.0 when no attempted resolutions exist.
    pub fn no_match_rate(&self) -> f64 {
        if self.attempted == 0 {
            0.0
        } else {
            self.no_match as f64 / self.attempted as f64
        }
    }
}

/// Remove a single invalidation marker after the entity has been re-resolved.
/// No-op if the marker does not exist (idempotent).
///
/// Called by `entity_resolver::write_log()` after writing the new resolution
/// log row, ensuring cleanup from all code paths (startup, compound-hook, tick).
pub fn clear_invalidated_no_match(
    conn: &rusqlite::Connection,
    agent_id: &str,
    subject_entity_id: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM kg_invalidated_no_match WHERE subject_entity_id = ?1 AND agent_id = ?2",
        rusqlite::params![subject_entity_id, agent_id],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- ResolutionOutcomeStats unit tests (#1077) ---

    #[test]
    fn no_match_rate_zero_attempted_returns_zero() {
        let stats = ResolutionOutcomeStats::default();
        assert_eq!(stats.no_match_rate(), 0.0);
    }

    #[test]
    fn no_match_rate_normal_calculation() {
        let stats = ResolutionOutcomeStats {
            total: 120,
            attempted: 100,
            no_match: 82,
            no_candidate_of_type: 0,
            matched_exact: 5,
            matched_llm: 13,
            matched_llm_db_fallback: 0,
            skipped: 15,
            errors: 5,
        };
        let rate = stats.no_match_rate();
        assert!((rate - 0.82).abs() < 1e-10, "Expected 0.82, got {rate}");
    }

    #[test]
    fn no_match_rate_all_matched() {
        let stats = ResolutionOutcomeStats {
            total: 50,
            attempted: 50,
            no_match: 0,
            no_candidate_of_type: 0,
            matched_exact: 30,
            matched_llm: 20,
            matched_llm_db_fallback: 0,
            skipped: 0,
            errors: 0,
        };
        assert_eq!(stats.no_match_rate(), 0.0);
    }

    #[test]
    fn no_match_rate_all_no_match() {
        let stats = ResolutionOutcomeStats {
            total: 100,
            attempted: 80,
            no_match: 80,
            no_candidate_of_type: 0,
            matched_exact: 0,
            matched_llm: 0,
            matched_llm_db_fallback: 0,
            skipped: 15,
            errors: 5,
        };
        assert_eq!(stats.no_match_rate(), 1.0);
    }

    #[test]
    fn format_entity_key_happy_path() {
        assert_eq!(format_entity_key("skill", "self-dev"), "skill:self-dev");
        assert_eq!(format_entity_key("skill", "dev-groom"), "skill:dev-groom");
        assert_eq!(
            format_entity_key("tool", "run_claude_pilot"),
            "tool:run_claude_pilot"
        );
        assert_eq!(format_entity_key("agent", "mika-dev"), "agent:mika-dev");
        assert_eq!(
            format_entity_key("problem_type", "fabrication"),
            "problem_type:fabrication"
        );
    }

    #[test]
    fn format_entity_key_empty_name() {
        // Helper does no validation — empty name produces "skill:" which
        // the DB CHECK constraint will reject at insert time.
        assert_eq!(format_entity_key("skill", ""), "skill:");
    }

    #[test]
    fn domain_entity_types_contains_expected() {
        assert!(KG_DOMAIN_ENTITY_TYPES.contains(&"skill"));
        assert!(KG_DOMAIN_ENTITY_TYPES.contains(&"tool"));
        assert!(KG_DOMAIN_ENTITY_TYPES.contains(&"agent"));
        assert!(KG_DOMAIN_ENTITY_TYPES.contains(&"problem_type"));
        assert!(KG_DOMAIN_ENTITY_TYPES.contains(&"concept"));
    }

    #[test]
    fn kg_chunk_source_type_value() {
        assert_eq!(KG_CHUNK_SOURCE_TYPE, "kg_chunk");
    }

    // --- #961: invalidation marker helpers ---

    /// Helper: open an in-memory DB for marker tests.
    fn marker_test_db() -> rusqlite::Connection {
        let db = crate::db::Database::open_in_memory().unwrap();
        db.register_agent("test-agent", "test-agent", "/tmp/test")
            .unwrap();
        db.conn
    }

    #[test]
    fn record_invalidated_inserts_markers() {
        let conn = marker_test_db();
        let inserted = record_invalidated_no_match(&conn, "test-agent", &[1, 2, 3]).unwrap();
        assert_eq!(inserted, 3);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kg_invalidated_no_match WHERE agent_id = 'test-agent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn record_invalidated_insert_or_ignore_idempotent() {
        let conn = marker_test_db();
        let first = record_invalidated_no_match(&conn, "test-agent", &[42]).unwrap();
        assert_eq!(first, 1);

        // Second call with same (subject_entity_id, agent_id) is a no-op.
        let second = record_invalidated_no_match(&conn, "test-agent", &[42]).unwrap();
        assert_eq!(second, 0, "INSERT OR IGNORE should return 0 on duplicate");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kg_invalidated_no_match WHERE agent_id = 'test-agent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "still only one row");
    }

    #[test]
    fn record_invalidated_empty_slice() {
        let conn = marker_test_db();
        let inserted = record_invalidated_no_match(&conn, "test-agent", &[]).unwrap();
        assert_eq!(inserted, 0);
    }

    #[test]
    fn clear_invalidated_removes_marker() {
        let conn = marker_test_db();
        record_invalidated_no_match(&conn, "test-agent", &[10]).unwrap();

        let removed = clear_invalidated_no_match(&conn, "test-agent", 10).unwrap();
        assert_eq!(removed, 1);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kg_invalidated_no_match WHERE agent_id = 'test-agent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn clear_invalidated_noop_on_missing() {
        let conn = marker_test_db();
        // No marker exists — DELETE is a no-op.
        let removed = clear_invalidated_no_match(&conn, "test-agent", 999).unwrap();
        assert_eq!(removed, 0, "should be no-op when marker does not exist");
    }

    #[test]
    fn markers_scoped_by_agent() {
        let conn = marker_test_db();
        conn.execute(
            "INSERT OR IGNORE INTO agents (id, name, home_dir) VALUES ('other-agent', 'other', '/tmp/other')",
            [],
        )
        .unwrap();

        record_invalidated_no_match(&conn, "test-agent", &[1, 2]).unwrap();
        record_invalidated_no_match(&conn, "other-agent", &[1, 3]).unwrap();

        // Each agent sees only their markers.
        let test_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kg_invalidated_no_match WHERE agent_id = 'test-agent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let other_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kg_invalidated_no_match WHERE agent_id = 'other-agent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(test_count, 2);
        assert_eq!(other_count, 2);

        // Clear for one agent doesn't affect the other.
        clear_invalidated_no_match(&conn, "test-agent", 1).unwrap();
        let test_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kg_invalidated_no_match WHERE agent_id = 'test-agent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let other_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kg_invalidated_no_match WHERE agent_id = 'other-agent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(test_after, 1, "test-agent lost one marker");
        assert_eq!(other_after, 2, "other-agent unchanged");
    }
}
