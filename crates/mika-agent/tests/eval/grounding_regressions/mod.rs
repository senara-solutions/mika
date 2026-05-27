//! # Grounding + Fabrication Regression Scenarios (#741)
//!
//! Thirty scenarios, each testing a concrete fabrication class from the KG milestone
//! retrospective and subsequent guards. Each scenario has hard assertions on tool calls
//! and response text. All gated behind `#[ignore]` + `MIKA_EVAL_GROUNDING=1` for
//! integration tier.
//!
//! ## Tag Vocabulary (`grounding:*`)
//!
//! See `README.md` in this directory for the full vocabulary, scope boundary
//! against `#740` `self-knowledge:*`, and capability x status matrix.
//!
//! ## Fixture Helpers
//!
//! Scenarios use `grounding_assertions` for hard negative/positive checks and
//! `kg_fixtures` (from #740) for scenario 5's KG state seeding.
//!
//! ## Reference
//!
//! - Issue: mika#741
//! - Plan: `docs/plans/741-grounding-regressions.md`

// Re-export common test dependencies for scenario files
pub use mika_common::llm::mock::*;
pub use serde_json::json;

pub use super::assertions::*;
pub use super::grounding_assertions;
pub use super::harness::EvalHarness;
pub use super::kg_fixtures;

// --- Scenario modules (one per fabrication class, D2) ---
pub mod asserted_unavailability_caught;
pub mod asserted_unavailability_elided_copula;
pub mod asserted_unavailability_genuine;
pub mod auto_merge_vs_merged;
pub mod current_priorities_drift;
pub mod dev_groom_dispatched_no_verdict;
pub mod dev_groom_fabricated_verdict_caught;
pub mod engine_correction_rejection;
pub mod fabricated_shell_errors;
pub mod graphql_field_fabrication;
pub mod kg_result_ignored;
pub mod merge_gate_blocked_no_fallback;
pub mod merge_gate_errored_no_fallback;
pub mod milestone_close;
pub mod qa_review_absence_claim_grounded;
pub mod qa_review_per_element_enumeration;
pub mod quoted_resource_pre_fetch;
pub mod required_finding_list;
pub mod required_suffix_line_caught;
pub mod required_suffix_line_unconstrained;
pub mod required_tools_retry_thin_final_turn;
pub mod summary_conversational_recall;
