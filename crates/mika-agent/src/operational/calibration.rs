//! Calibration constants for operational-item priority scoring.
//!
//! These constants are Layer 2's concern (the "What's Next" engine computes
//! composite priority at read time), but they are defined in Layer 1 so the
//! schema stores the correct fields and Layer 2 can consume them without
//! migration. See foundation Decision C.

// -- Commitment weight by owner --

/// Priority weight for user-owned commitments.
pub const COMMITMENT_WEIGHT_USER: f32 = 50.0;

/// Priority weight for Mika-owned commitments.
pub const COMMITMENT_WEIGHT_MIKA: f32 = 35.0;

/// Priority weight for third-party-owned commitments (person/agent).
pub const COMMITMENT_WEIGHT_THIRD_PARTY: f32 = 20.0;

// -- Staleness --

/// Maximum staleness penalty (days overdue cap).
pub const STALE_TIME_CAP: f32 = 30.0;

/// Multiplier per day overdue (up to `STALE_TIME_CAP` days).
pub const STALE_TIME_MULTIPLIER: f32 = 5.0;

// -- Dependency risk --

/// Priority penalty per item blocked by this one.
pub const DEPENDENCY_RISK_PER_BLOCKED: f32 = 10.0;

/// Maximum dependency-risk penalty.
pub const DEPENDENCY_RISK_CAP: f32 = 40.0;

// -- Confidence and urgency --

/// Multiplier for low-confidence penalty (applied as `(1 - confidence) * M`).
pub const CONFIDENCE_PENALTY_MULTIPLIER: f32 = 50.0;

/// Maximum urgency score (from time-based proximity to due date).
pub const URGENCY_MAX: f32 = 100.0;

/// Maximum user-importance score.
pub const USER_IMPORTANCE_MAX: f32 = 50.0;
