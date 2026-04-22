//! # Knowledge Graph Layer
//!
//! The KG layer provides a graph-structured view of Mika's domain knowledge,
//! split into three tiers:
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
//! - **Subject graph** (future, #690/#691): Per-agent LLM-extracted entities, fact
//!   triples, and resolution edges linking subject mentions to domain nodes.

pub mod chunker;
pub mod domain_builder;
pub mod lexical_ingestor;
