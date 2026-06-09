//! plan-doc invariants, dispatch-readiness predicates, agent-loop policy.
//!
//! Per Foundation §6, this module owns three sub-concerns. At extraction time:
//! - **agent-loop policy** — 12 constants in `policy.rs` (step budgets,
//!   timeouts, byte/char caps, staleness thresholds)
//! - **dispatch-readiness predicates** — currently empty (gate predicates
//!   live in `tool_execution::dispatch_gates`)
//! - **plan-doc invariants** — currently empty (future-accretion target)

pub mod policy;
