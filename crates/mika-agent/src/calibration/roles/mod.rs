//! Role implementations for model calibration.
//!
//! Each submodule defines a role's scenario suite with synthetic skills
//! and structural assertions.

use mika_common::llm::LlmError;

use crate::calibration::failure::classify_failure;
use crate::calibration::role::RoleScenarioResult;

pub mod mika_arch;
pub mod mika_dev;

/// Shared helper: classify an LLM error into a `RoleScenarioResult` failure
/// using `classify_failure` instead of hardcoding `TransportError`.
pub fn llm_error_result(scenario_id: &str, error: LlmError, latency_ms: u64) -> RoleScenarioResult {
    let error_str = error.to_string();
    let failure_class = classify_failure(Some(&error_str), None);
    RoleScenarioResult::fail(
        scenario_id,
        failure_class,
        error_str,
        None,
        None,
        latency_ms,
    )
}
