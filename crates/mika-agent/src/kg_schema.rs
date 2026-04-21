//! Knowledge Graph schema constants, type enums, and helpers.
//!
//! This module defines the typed constants and utility functions for Mika's
//! Knowledge Graph (KG) schema. The actual DDL lives in [`crate::db`] (both
//! `create_tables_v1()` and `migrate_v24_to_v25()`); this module provides the
//! Rust-side type system that tools and extraction pipelines build on.
//!
//! # Entity Key Convention
//!
//! Every domain-layer entity has a composite text key `entity_key = type:name`
//! (e.g. `"person:Alice"`, `"org:Acme Corp"`). The key is enforced by a SQLite
//! CHECK constraint and is the deduplication identity for entities.
//!
//! - **INTEGER PK** (`id`) is used for joins and FK references — fast, compact.
//! - **TEXT `entity_key`** is the UNIQUE external identity — human-readable,
//!   stable across migrations, used by extraction and query tools.
//!
//! # Write Contract
//!
//! All writes to KG tables must:
//! 1. Use [`format_entity_key`] to produce `entity_key` values.
//! 2. Validate `entity_type` against [`VALID_ENTITY_TYPES`] before insert.
//! 3. Ensure `confidence` values are in `[0.0, 1.0]` (enforced by CHECK).
//! 4. Ensure `source_doc_hash` is non-empty for chunk dedup/idempotency.

/// Entity type: a person (contact, colleague, family member).
pub const ENTITY_TYPE_PERSON: &str = "person";

/// Entity type: an organization (company, team, group).
pub const ENTITY_TYPE_ORG: &str = "org";

/// Entity type: a project (software project, initiative, campaign).
pub const ENTITY_TYPE_PROJECT: &str = "project";

/// Entity type: a place (city, office, venue).
pub const ENTITY_TYPE_PLACE: &str = "place";

/// Entity type: a concept (technology, methodology, domain term).
pub const ENTITY_TYPE_CONCEPT: &str = "concept";

/// Entity type: an event (meeting, conference, milestone).
pub const ENTITY_TYPE_EVENT: &str = "event";

/// All valid entity types. The DB CHECK constraint enforces the same set.
pub const VALID_ENTITY_TYPES: &[&str] = &[
    ENTITY_TYPE_PERSON,
    ENTITY_TYPE_ORG,
    ENTITY_TYPE_PROJECT,
    ENTITY_TYPE_PLACE,
    ENTITY_TYPE_CONCEPT,
    ENTITY_TYPE_EVENT,
];

// --- Relationship types ---

/// Relationship: source works at / belongs to target org.
pub const REL_WORKS_AT: &str = "works_at";

/// Relationship: source manages / leads target.
pub const REL_MANAGES: &str = "manages";

/// Relationship: source is located in target place.
pub const REL_LOCATED_IN: &str = "located_in";

/// Relationship: source is related to target (generic).
pub const REL_RELATED_TO: &str = "related_to";

/// Relationship: source is part of target.
pub const REL_PART_OF: &str = "part_of";

/// All valid relationship types.
pub const VALID_RELATIONSHIP_TYPES: &[&str] = &[
    REL_WORKS_AT,
    REL_MANAGES,
    REL_LOCATED_IN,
    REL_RELATED_TO,
    REL_PART_OF,
];

// --- Resolution types ---

/// Resolution: exact name match.
pub const RESOLUTION_EXACT: &str = "exact";

/// Resolution: alias or alternative name.
pub const RESOLUTION_ALIAS: &str = "alias";

/// Resolution: coreference (pronoun or indirect reference).
pub const RESOLUTION_COREFERENCE: &str = "coreference";

/// All valid resolution types. The DB CHECK constraint enforces the same set.
pub const VALID_RESOLUTION_TYPES: &[&str] =
    &[RESOLUTION_EXACT, RESOLUTION_ALIAS, RESOLUTION_COREFERENCE];

// --- Extraction status ---

/// Extraction status: currently running.
pub const EXTRACTION_STATUS_RUNNING: &str = "running";

/// Extraction status: completed successfully.
pub const EXTRACTION_STATUS_COMPLETED: &str = "completed";

/// Extraction status: failed.
pub const EXTRACTION_STATUS_FAILED: &str = "failed";

/// All valid extraction statuses.
pub const VALID_EXTRACTION_STATUSES: &[&str] = &[
    EXTRACTION_STATUS_RUNNING,
    EXTRACTION_STATUS_COMPLETED,
    EXTRACTION_STATUS_FAILED,
];

// --- Resolution log outcomes ---

/// Resolution outcome: successfully resolved to a domain entity.
pub const RESOLUTION_OUTCOME_RESOLVED: &str = "resolved";

/// Resolution outcome: no matching domain entity found.
pub const RESOLUTION_OUTCOME_UNRESOLVED: &str = "unresolved";

/// Resolution outcome: ambiguous — multiple candidates.
pub const RESOLUTION_OUTCOME_AMBIGUOUS: &str = "ambiguous";

/// All valid resolution outcomes.
pub const VALID_RESOLUTION_OUTCOMES: &[&str] = &[
    RESOLUTION_OUTCOME_RESOLVED,
    RESOLUTION_OUTCOME_UNRESOLVED,
    RESOLUTION_OUTCOME_AMBIGUOUS,
];

/// The separator used in `entity_key` between type and name.
const ENTITY_KEY_SEPARATOR: char = ':';

/// Format an entity key from its type and name.
///
/// Returns `"type:name"` (e.g. `"person:Alice"`).
///
/// # Panics
///
/// Does not panic. Empty `entity_type` or `name` produce a technically valid
/// key (e.g. `":Alice"` or `"person:"`), but the DB CHECK constraint will
/// reject empty components if enforced at the application layer.
pub fn format_entity_key(entity_type: &str, name: &str) -> String {
    format!("{entity_type}{ENTITY_KEY_SEPARATOR}{name}")
}

/// Parse an entity key into `(entity_type, name)`.
///
/// Returns `None` if the key does not contain the separator.
/// Only splits on the first `:`, so names containing `:` are preserved.
pub fn parse_entity_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(ENTITY_KEY_SEPARATOR)
}

/// Source type value for KG chunks in the `search_content` pipeline.
/// Used when inserting KG chunk content into the shared FTS5/vec search index.
pub const SEARCH_CONTENT_SOURCE_TYPE_KG_CHUNK: &str = "kg_chunk";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_entity_key() {
        assert_eq!(format_entity_key("person", "Alice"), "person:Alice");
        assert_eq!(format_entity_key("org", "Acme Corp"), "org:Acme Corp");
    }

    #[test]
    fn test_format_entity_key_empty_name() {
        // Empty name still formats — validation is caller's responsibility
        assert_eq!(format_entity_key("person", ""), "person:");
    }

    #[test]
    fn test_format_entity_key_name_with_colon() {
        // Names can contain colons — only the first colon is the separator
        assert_eq!(
            format_entity_key("concept", "key:value pairs"),
            "concept:key:value pairs"
        );
    }

    #[test]
    fn test_parse_entity_key() {
        assert_eq!(parse_entity_key("person:Alice"), Some(("person", "Alice")));
        assert_eq!(
            parse_entity_key("concept:key:value pairs"),
            Some(("concept", "key:value pairs"))
        );
    }

    #[test]
    fn test_parse_entity_key_no_separator() {
        assert_eq!(parse_entity_key("noseparator"), None);
    }

    #[test]
    fn test_valid_entity_types_contains_all() {
        assert!(VALID_ENTITY_TYPES.contains(&ENTITY_TYPE_PERSON));
        assert!(VALID_ENTITY_TYPES.contains(&ENTITY_TYPE_ORG));
        assert!(VALID_ENTITY_TYPES.contains(&ENTITY_TYPE_PROJECT));
        assert!(VALID_ENTITY_TYPES.contains(&ENTITY_TYPE_PLACE));
        assert!(VALID_ENTITY_TYPES.contains(&ENTITY_TYPE_CONCEPT));
        assert!(VALID_ENTITY_TYPES.contains(&ENTITY_TYPE_EVENT));
    }

    #[test]
    fn test_valid_resolution_types_contains_all() {
        assert!(VALID_RESOLUTION_TYPES.contains(&RESOLUTION_EXACT));
        assert!(VALID_RESOLUTION_TYPES.contains(&RESOLUTION_ALIAS));
        assert!(VALID_RESOLUTION_TYPES.contains(&RESOLUTION_COREFERENCE));
    }
}
