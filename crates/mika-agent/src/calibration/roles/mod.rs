//! Role implementations for model calibration.
//!
//! Each submodule defines a role's scenario suite with synthetic skills
//! and structural assertions.

use mika_common::llm::LlmError;
use mika_common::llm::types::{LlmResponse, LlmStopReason};

use crate::calibration::failure::{FailureClass, classify_failure};
use crate::calibration::role::RoleScenarioResult;

pub mod mika_arch;
pub mod mika_dev;
pub mod mika_qa;

/// Shared helper: classify an LLM error into a `RoleScenarioResult` failure
/// using `classify_failure` instead of hardcoding `TransportError`.
pub fn llm_error_result(scenario_id: &str, error: LlmError, latency_ms: u64) -> RoleScenarioResult {
    let error_str = error.to_string();
    let failure_class = classify_failure(Some(&error_str), None, None, false);
    RoleScenarioResult::fail(
        scenario_id,
        failure_class,
        error_str,
        None,
        None,
        latency_ms,
    )
}

/// Shared helper: build a failure result for an empty visible response,
/// distinguishing a genuine empty/refusal from reasoning-budget exhaustion
/// (the model consumed its entire output budget on internal reasoning tokens
/// before emitting any visible content). The latter is remediated by raising
/// `max_tokens`, not by reverting a model swap (mika#1665).
pub fn empty_response_result(
    scenario_id: &str,
    response: &LlmResponse,
    latency_ms: u64,
) -> RoleScenarioResult {
    let finish_reason_is_length = response.stop_reason == LlmStopReason::MaxTokens;
    let class = classify_failure(
        None,
        Some(""),
        Some(response.usage.output_tokens),
        finish_reason_is_length,
    );
    let detail = match class {
        FailureClass::ReasoningBudgetExhausted => format!(
            "Reasoning budget exhausted: {} output tokens consumed by internal \
             reasoning before any visible content (finish_reason=length). \
             Raise max_tokens for this scenario.",
            response.usage.output_tokens
        ),
        _ => "Empty response".to_string(),
    };
    RoleScenarioResult::fail(
        scenario_id,
        class,
        detail,
        Some(response.usage.input_tokens),
        Some(response.usage.output_tokens),
        latency_ms,
    )
}
