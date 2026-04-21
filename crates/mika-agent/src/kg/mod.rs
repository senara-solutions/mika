//! Knowledge Graph builders and query helpers.
//!
//! The KG has three layers (see `docs/architecture/kg-implementation-conventions.md`):
//!
//! - **Domain** — deterministic, container-wide. Populated by [`DomainGraphBuilder`]
//!   from skill manifests, tool registry, MCP connections, and agent configs.
//!   Entity types: `skill`, `tool`, `agent`, `problem_type`.
//!   Relationship types: `DEPENDS_ON`, `PROVIDES`.
//!
//! - **Lexical** — per-agent text chunks from ingested documents (#689).
//!
//! - **Subject** — per-agent LLM-extracted entities and relationships (#690, #691).
//!
//! Schema constants live in [`crate::kg_schema`]. The DDL lives in [`crate::db`].

pub mod domain_builder;

pub use domain_builder::{DomainGraphBuilder, PendingRebuild};
