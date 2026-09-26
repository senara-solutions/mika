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
//! - `doctrine:false-local-hosting-claimed` — pre-fix failure tag (mika#2290:
//!   the agent asserted it runs locally, or that the user's data never leaves
//!   their machine, on a deployment that is not declared local).
//! - `doctrine:false-local-hosting-suppressed` — post-fix success tag (guard
//!   5d caught the claim and the corrected turn no longer makes it).
//! - `doctrine:hosting-ground-truth-honored` — post-fix success tag (the
//!   corrected turn says what the `## Runtime` hosting line supports, rather
//!   than falling silent).
//! - `doctrine:doctrine-not-found` — pre-fix failure tag (mika#2292: asked what
//!   the Mika doctrine is, the agent answered that it found nothing of that
//!   name — **and then gave the philosophy anyway**, in the same response. Not
//!   a knowledge gap: a name gap).
//! - `doctrine:material-doctrine-answerable` — post-fix success tag (the
//!   material register is in the served prompt under its aliases, so the
//!   question resolves to a substantial answer carrying each stance's *why*).
//! - `register:em-dash-emitted` — pre-fix failure tag (mika#2247: the
//!   general-public tenant's delivered text carried U+2014, one occurrence of
//!   which was `FAMILY_SOUL` copied verbatim).
//! - `register:em-dash-normalised` — post-fix success tag (the three output
//!   sites compose the tag strip with the typographic normaliser, and the
//!   operator register is deliberately untouched).
//! - `register:language-drifted` — pre-fix failure tag (EN↔FR flip inside one
//!   thread on a tenant whose persona prescribes French).
//! - `register:language-held` — post-fix success tag (guard 5f caught the
//!   drift on a tenant that declared its language, and left an undeclared one
//!   alone).
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

// --- mika#2290: hosting is a posed fact, never inferred ---
//
// Same module rather than a new one: the failure class is identical in shape to
// the public-promo one — the agent's own text violates a load-bearing product
// invariant with no fabrication of *evidence* involved. Distinct tag namespace
// entries (`doctrine:false-local-hosting-*`) keep the two populations countable
// apart.
pub mod false_local_hosting_claim_caught;

// --- mika#2292: the tenant held the answer and had no name for it ---
//
// Same module again, and for the reason mika#2290 already wrote here: the
// failure class is the shape of the agent's own text against a load-bearing
// product invariant, with no fabrication of *evidence* involved. Distinct tag
// entries (`doctrine:doctrine-not-found`, `doctrine:material-doctrine-answerable`)
// keep the populations countable apart.
//
// Two files, one axis each — the split is the plan's Fire-Disposition decision,
// not a filing convenience. The first is deterministic and gates CI; the second
// is the behavioural half, which no deterministic test can establish and which
// therefore ships disarmed with its reasoning at the site.
pub mod doctrine_mika_answer_replayed;
pub mod doctrine_mika_section_rendered;

// --- mika#2247: the general-public tenant holds its register ---
//
// Same module, third time, and the criterion is the one mika#2290 wrote here:
// the failure class is the **shape of the agent's own text** against a
// load-bearing product invariant, with no fabrication of evidence involved. A
// register is such an invariant on the family tier — its persona is a product
// decision Vincent approved, not a style preference.
//
// Distinct tag entries (`register:*`) keep the populations countable apart, and
// the file carries both axes that have a production path: the typographic
// normalisation (AC1) and the language-drift guard (AC2). AC3's production half
// is the `## Current Time` section, pinned in `prompt::tests::mika2247_*`.
pub mod tenant_register_held;
