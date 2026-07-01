//! Model calibration framework (#1190).
//!
//! Provides:
//! - Calibration artifact schema and diff tool
//! - Scenario outcome types
//! - Provider construction helpers
//! - Role-scoped scenario abstractions
//! - Failure classification taxonomy
//! - Swap-gate exit-code decision logic

pub mod artifact;
pub mod failure;
pub mod gate;
pub mod providers;
pub mod role;
pub mod roles;
pub mod scenario;
