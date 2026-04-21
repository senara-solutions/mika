//! # KG Schema Constants
//!
//! Centralizes Knowledge Graph table names, column lists, type enums, and ID
//! convention constants. All migration code and row mappers import from here
//! instead of using inline strings or `SELECT *`.
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
//!   allows the ingestor to skip re-ingestion when the doc hasn't changed.
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

/// Column list for `kg_entities` queries. No `SELECT *`.
pub const KG_ENTITY_COLUMNS: &str =
    "id, entity_key, type, name, properties_json, created_at, updated_at";

/// Column list for `kg_relationships` queries. No `SELECT *`.
pub const KG_RELATIONSHIP_COLUMNS: &str =
    "id, from_entity_id, to_entity_id, type, properties_json, created_at";

/// Column list for `kg_chunks` queries. No `SELECT *`.
pub const KG_CHUNK_COLUMNS: &str =
    "id, agent_id, seq_id, source_doc_path, source_doc_hash, created_at, trace_id";

/// Column list for `kg_subject_entities` queries. No `SELECT *`.
pub const KG_SUBJECT_ENTITY_COLUMNS: &str =
    "id, agent_id, entity_key, type, name, confidence, properties_json, created_at, trace_id";

/// Column list for `kg_subject_resolutions` queries. No `SELECT *`.
pub const KG_SUBJECT_RESOLUTION_COLUMNS: &str =
    "id, agent_id, subject_entity_id, domain_entity_id, confidence, created_at, trace_id";

/// Column list for `kg_subject_relationships` queries. No `SELECT *`.
pub const KG_SUBJECT_RELATIONSHIP_COLUMNS: &str = "id, agent_id, from_entity_id, to_entity_id, type, confidence, properties_json, created_at, trace_id";

/// Column list for `kg_chunk_subjects` queries. No `SELECT *`.
pub const KG_CHUNK_SUBJECT_COLUMNS: &str =
    "id, agent_id, chunk_id, subject_entity_id, extraction_trace_id, created_at";

/// Column list for `kg_chunk_subject_relationships` queries. No `SELECT *`.
pub const KG_CHUNK_SUBJECT_RELATIONSHIP_COLUMNS: &str =
    "id, agent_id, chunk_id, subject_relationship_id, extraction_trace_id, created_at";

/// Column list for `kg_extractions` queries. No `SELECT *`.
pub const KG_EXTRACTION_COLUMNS: &str = "id, agent_id, source_doc_path, extraction_model, entities_extracted, relationships_extracted, extraction_trace_id, created_at";

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
pub const KG_DOMAIN_ENTITY_TYPES: &[&str] = &["skill", "tool", "agent", "problem_type"];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_entity_key_happy_path() {
        assert_eq!(format_entity_key("skill", "self-dev"), "skill:self-dev");
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
    }

    #[test]
    fn kg_chunk_source_type_value() {
        assert_eq!(KG_CHUNK_SOURCE_TYPE, "kg_chunk");
    }
}
