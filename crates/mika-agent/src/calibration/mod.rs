//! Model calibration framework (#1190).
//!
//! Provides:
//! - Calibration artifact schema and diff tool
//! - Scenario outcome types
//! - Provider construction helpers
//! - Role-scoped scenario abstractions
//! - Failure classification taxonomy

pub mod artifact;
pub mod disambiguator;
pub mod failure;
pub mod providers;
pub mod role;
pub mod roles;
pub mod scenario;
