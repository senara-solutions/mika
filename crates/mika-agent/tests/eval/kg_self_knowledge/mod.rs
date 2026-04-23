//! # KG Self-Knowledge Eval Scenarios (#740)
//!
//! Seven scenarios verifying that agents correctly query the knowledge graph
//! before claiming capability or state. Covers the three KG entry paths
//! (domain match, subject match, semantic via chunks), the two-stage resolver
//! (exact match, LLM disambiguation), and agent context annotation.
//!
//! ## Tag Vocabulary (`self-knowledge:*`)
//!
//! See `README.md` in this directory for the full vocabulary and
//! capability × status matrix.
//!
//! ## Fixture Helpers
//!
//! Scenarios share the `kg_fixtures` module for seeding KG state.
//! Each scenario owns its fixture *composition*; helpers are shared.
//!
//! ## Reference
//!
//! - Issue: mika#740
//! - Plan: `docs/plans/740-kg-self-knowledge-eval.md`

// Re-export common test dependencies for scenario files
pub use mika_common::llm::mock::*;
pub use serde_json::json;

pub use super::assertions::*;
pub use super::harness::EvalHarness;
pub use super::kg_fixtures;

// --- Scenario modules (one per scenario, D2) ---
pub mod tool_selection_query_knowledge_graph;
pub mod path_a_direct_domain_match;
pub mod path_b_subject_match_agent_scoped;
pub mod path_c_semantic_via_chunks;
pub mod stage_1_exact_match;
pub mod stage_2_llm_disambiguation;
pub mod agent_context_annotation_disabled;
