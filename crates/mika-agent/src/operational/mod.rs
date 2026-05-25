//! Operational ledger — canonical schema for tracking goals, tasks, commitments,
//! decisions, blockers, evidence, and next-actions across the Mika agent platform.
//!
//! See `docs/architecture/operational-partner-frame.md` for the design foundation
//! and `docs/plans/2026-05-24-001-feat-task-ledger-operational-item-plan.md` for
//! the Layer 1 implementation plan (mika#1262).

pub mod calibration;
pub mod query;
pub mod types;
pub mod write;
