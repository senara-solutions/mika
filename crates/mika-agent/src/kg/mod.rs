//! # Knowledge Graph Layer
//!
//! The KG layer provides a graph-structured view of Mika's domain knowledge,
//! split into three tiers plus a cross-layer bridge:
//!
//! - **Domain graph** (this module, [`domain_builder`]): Deterministic, container-wide
//!   projection of structural facts from skill manifests, tool registry, MCP connections,
//!   and agent configs. Populated at startup, idempotent, no LLM calls. See
//!   `docs/architecture/kg-implementation-conventions.md` for cross-cutting conventions
//!   and `crates/mika-agent/src/db/kg_schema.rs` for schema constants and ID format.
//!
//! - **Lexical graph** ([`lexical_ingestor`], #689): Per-agent chunk ingestion of
//!   `docs/solutions/**/*.md` into `kg_chunks` + `search_content`. Content-hash
//!   idempotent, runs at startup after domain rebuild.
//!
//! - **Subject graph** ([`subject_extractor`], #690): Per-agent LLM-extracted
//!   entities and fact triples from compound docs. Uses constrained NER with
//!   approved entity/relationship types.
//!
//! - **Entity resolution** ([`entity_resolver`], #691): Bridges subject graph to
//!   domain graph by resolving extracted entity mentions to canonical domain nodes.
//!   Two-stage pipeline: exact match + LLM disambiguation. Writes `SAME_AS` edges
//!   in `kg_subject_resolutions` with confidence scores.

pub mod chunker;
pub mod config;
pub mod domain_builder;
pub mod entity_resolver;
pub mod ingestion_orchestrator;
pub mod lexical_ingestor;
pub mod query;
pub mod subject_extractor;
