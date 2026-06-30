//! grounding-rule enforcement, fabrication-guard predicates, tool-call audit trail.
//!
//! Per Foundation §6: this module owns the predicates that the agent loop's
//! guard-dispatch logic consults (assert_grounded, asserted_unavailability,
//! fabrication detection), plus the audit_events ledger that persists
//! tool-call provenance for the rewind path and dashboard surfaces.
//!
//! Guard *enforcement timing* (the reject-and-reprompt machinery at EndTurn)
//! lives in `crate::agent` (post-#1452 agent_loop/). This module exposes the
//! pure predicates that drive that machinery.

pub mod audit;
pub mod guards;

pub use audit::AuditEvent;
pub use guards::{
    ASSERT_GROUNDED_LABEL, AffirmativeStateClaim, EQUIVALENCE_CLAIM_LABEL, EquivalenceClaim,
    GROUNDING_TOOLS,
};
