//! # Doctrine Regression Scenarios (mika#1814)
//!
//! Regression scenarios for Mika's Distribution Doctrine — the invitation-only
//! / hermetic distribution invariant. Founding incident: Al B (family-tier
//! testeur, 2026-07-20) — his Mika proposed proactively to draft a Show HN
//! post to "promote Mika", violating the ratified invitation-only doctrine.
//!
//! Each scenario tests a concrete public-promo failure class against the
//! `guard.doctrine_public_promo` EndTurn guard (position 5c, mika#1814). Hard
//! assertions on LLM call count (guard fires ⇒ retry occurred) and response
//! substring (drafting language suppressed, invitation redirect present).
//!
//! ## Tag Vocabulary (`doctrine:*`)
//!
//! - `doctrine:invitation-only-honored` — post-fix success tag (guard caught
//!   the fabrication and the corrected turn redirects to invitation chain).
//! - `doctrine:public-promo-suppressed` — post-fix success tag (guard caught
//!   the drafting proposal and the corrected turn contains no drafting
//!   language).
//! - `doctrine:public-promo-proposed` — pre-fix failure tag (agent drafted /
//!   proposed a public launch surface without the guard catching it).
//!
//! Namespace convention per `docs/architecture/kg-implementation-conventions.md`
//! § C3 — parallel to `#741 grounding:*` and `#740 self-knowledge:*`.
//!
//! ## Scope Boundary
//!
//! - `grounding:*` (mika#741) — response-to-evidence paths (fabricated
//!   citations, unattempted tools).
//! - `self-knowledge:*` (mika#740) — query-invocation code paths.
//! - `doctrine:*` (this module) — content-shape paths where the agent's
//!   response violates a load-bearing product doctrine even without any
//!   fabrication of evidence.
//!
//! ## Reference
//!
//! - Issue: mika#1814
//! - Plan:  `docs/plans/2026-08-22-005-fix-1814-agent-doctrine-invitation-only-plan.md`
//! - Bearing: `project_mika_invitation_only_no_public_launch`
//! - Related: mika#1798 (umbrella non-transit doctrine bake),
//!   mika#1783 (leak "Salut Vincent" — l'être n'appelle jamais la maison).

// Re-export common test dependencies for scenario files.
pub use mika_common::llm::mock::*;

pub use super::assertions::*;
pub use super::grounding_assertions;
pub use super::harness::EvalHarness;

// --- Scenario modules (one per public-promo class) ---
pub mod doctrine_public_promo_educational_answer_no_op;
pub mod doctrine_public_promo_product_hunt_caught;
pub mod doctrine_public_promo_show_hn_caught;

// --- Prompt-shape contract for AC1 / AC9 ---
pub mod doctrine_prompt_section_rendered;
