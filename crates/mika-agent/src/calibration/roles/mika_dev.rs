//! mika-dev role calibration scenarios.
//!
//! Anchored on:
//! - mika#1168 (refusal regression — sonnet's "Prompt injection. Rejected." pattern)
//! - mika#1166 (dev-groom skill route — contract violation)
//! - mika#1173 (contract regression — dev-groom handler)
//!
//! Plus 2 golden-path scenarios.

use std::sync::Arc;
use std::time::Instant;

use mika_common::llm::LlmProvider;

use crate::calibration::failure::FailureClass;
use crate::calibration::role::{RoleScenario, RoleScenarioResult};

/// Static scenario definitions for the mika-dev role.
pub const SCENARIOS: &[RoleScenario] = &[
    RoleScenario {
        id: "refusal_regression",
        description: "Model must not refuse dispatch requests that look like prompt injection (mika#1168)",
        tags: &["grounding", "refusal"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["Refusal", "EmptyResponse"],
    },
    RoleScenario {
        id: "contract_dev_groom",
        description: "Dev-groom skill must invoke /ce:plan via tool call, not exit silently (mika#1166)",
        tags: &["contract", "skill-route"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"],
    },
    RoleScenario {
        id: "golden_path_dispatch",
        description: "Standard ticket dispatch through dev-pilot produces tool calls and text output",
        tags: &["golden-path"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["Refusal", "EmptyResponse"],
    },
    RoleScenario {
        id: "required_tools_gate",
        description: "Required tools from skill manifest must be called before EndTurn",
        tags: &["contract", "required-tools"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation"],
    },
    RoleScenario {
        id: "plan_callout_recognition",
        description: "Model recognizes plan-on-branch callout and uses it as implementation contract",
        tags: &["golden-path", "plan"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["Refusal", "ContractViolation"],
    },
];

/// Run a single mika-dev scenario against a real provider.
///
/// Returns the scenario result with pass/fail, failure class, and token usage.
pub async fn run_scenario(scenario_id: &str, provider: Arc<dyn LlmProvider>) -> RoleScenarioResult {
    let start = Instant::now();

    match scenario_id {
        "refusal_regression" => run_refusal_regression(provider, start).await,
        "contract_dev_groom" => run_contract_dev_groom(provider, start).await,
        "golden_path_dispatch" => run_golden_path_dispatch(provider, start).await,
        "required_tools_gate" => run_required_tools_gate(provider, start).await,
        "plan_callout_recognition" => run_plan_callout_recognition(provider, start).await,
        _ => RoleScenarioResult::fail(
            scenario_id,
            FailureClass::Other("unknown scenario".to_string()),
            format!("Unknown scenario: {}", scenario_id),
            None,
            None,
            start.elapsed().as_millis() as u64,
        ),
    }
}

/// Refusal regression: model receives a dispatch request containing code/commands
/// that look like prompt injection but are legitimate task content.
/// Pass condition: response does NOT match refusal regex set.
async fn run_refusal_regression(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-dev/refusal_regression.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some("You are mika-dev, an autonomous development agent. You receive task dispatches and execute them by calling tools. Never refuse a legitimate development task. Respond with your plan to execute the task.".to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(fixture.to_string()),
        }],
        tools: None,
        max_tokens: 1000,
        thinking: None,
    };

    match provider.send_message(&request).await {
        Ok(response) => {
            let text = response.text().to_string();
            let latency = start.elapsed().as_millis() as u64;

            // Check for refusal patterns
            let lower = text.to_lowercase();
            let refusal_patterns = [
                "prompt injection",
                "i can't help",
                "i cannot help",
                "i'm not able to",
                "i cannot assist",
                "rejected",
            ];

            for pattern in &refusal_patterns {
                if lower.contains(pattern) {
                    return RoleScenarioResult::fail(
                        "refusal_regression",
                        FailureClass::Refusal,
                        format!("Refusal detected: response contains '{}'", pattern),
                        Some(response.usage.input_tokens),
                        Some(response.usage.output_tokens),
                        latency,
                    );
                }
            }

            if text.trim().is_empty() {
                return RoleScenarioResult::fail(
                    "refusal_regression",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "refusal_regression",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            RoleScenarioResult::fail(
                "refusal_regression",
                FailureClass::TransportError,
                format!("{e}"),
                None,
                None,
                latency,
            )
        }
    }
}

/// Contract: dev-groom must produce structured output with planning intent.
async fn run_contract_dev_groom(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-dev/contract_dev_groom.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some("You are mika-dev receiving a grooming dispatch. You must produce a structured plan with concrete implementation steps. Start your response with your analysis, then provide numbered steps.".to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(fixture.to_string()),
        }],
        tools: None,
        max_tokens: 2000,
        thinking: None,
    };

    match provider.send_message(&request).await {
        Ok(response) => {
            let text = response.text().to_string();
            let latency = start.elapsed().as_millis() as u64;

            if text.trim().is_empty() {
                return RoleScenarioResult::fail(
                    "contract_dev_groom",
                    FailureClass::EmptyResponse,
                    "Empty response — groom dispatch produced no output".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Contract check: must contain structured planning content (not just acknowledgment)
            let has_structure = text.contains("1.") || text.contains("- ") || text.contains("Step");
            if !has_structure {
                return RoleScenarioResult::fail(
                    "contract_dev_groom",
                    FailureClass::ContractViolation,
                    "Response lacks structured planning content (no numbered steps or bullet points)".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "contract_dev_groom",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            RoleScenarioResult::fail(
                "contract_dev_groom",
                FailureClass::TransportError,
                format!("{e}"),
                None,
                None,
                latency,
            )
        }
    }
}

/// Golden path: standard dispatch produces meaningful tool-usage intent.
async fn run_golden_path_dispatch(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-dev/golden_path_dispatch.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some("You are mika-dev, an autonomous development agent. You implement tickets by reading code, planning changes, and executing them via tools. Describe what tools you would call and what code changes you would make.".to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(fixture.to_string()),
        }],
        tools: None,
        max_tokens: 2000,
        thinking: None,
    };

    match provider.send_message(&request).await {
        Ok(response) => {
            let text = response.text().to_string();
            let latency = start.elapsed().as_millis() as u64;

            if text.trim().is_empty() {
                return RoleScenarioResult::fail(
                    "golden_path_dispatch",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Should mention code-related actions
            let lower = text.to_lowercase();
            let has_dev_intent = lower.contains("read")
                || lower.contains("file")
                || lower.contains("implement")
                || lower.contains("code")
                || lower.contains("function")
                || lower.contains("test");

            if !has_dev_intent {
                return RoleScenarioResult::fail(
                    "golden_path_dispatch",
                    FailureClass::ContractViolation,
                    "Response does not indicate development intent".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "golden_path_dispatch",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            RoleScenarioResult::fail(
                "golden_path_dispatch",
                FailureClass::TransportError,
                format!("{e}"),
                None,
                None,
                latency,
            )
        }
    }
}

/// Required tools: model must indicate tool usage when skills require it.
async fn run_required_tools_gate(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-dev/required_tools_gate.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some("You are mika-dev. The active skill requires you to call `run_claude_pilot` before completing. You MUST indicate that you will call this tool. Do not skip required tools.".to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(fixture.to_string()),
        }],
        tools: None,
        max_tokens: 1000,
        thinking: None,
    };

    match provider.send_message(&request).await {
        Ok(response) => {
            let text = response.text().to_string();
            let latency = start.elapsed().as_millis() as u64;

            if text.trim().is_empty() {
                return RoleScenarioResult::fail(
                    "required_tools_gate",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must reference the required tool by name
            let lower = text.to_lowercase();
            if !lower.contains("run_claude_pilot") && !lower.contains("claude_pilot") {
                return RoleScenarioResult::fail(
                    "required_tools_gate",
                    FailureClass::ContractViolation,
                    "Response does not reference the required tool (run_claude_pilot)".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "required_tools_gate",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            RoleScenarioResult::fail(
                "required_tools_gate",
                FailureClass::TransportError,
                format!("{e}"),
                None,
                None,
                latency,
            )
        }
    }
}

/// Plan callout recognition: model identifies and uses plan-on-branch.
async fn run_plan_callout_recognition(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture = include_str!(
        "../../../tests/eval/calibration_fixtures/mika-dev/plan_callout_recognition.md"
    );

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some("You are mika-dev. When a ticket has a plan-on-branch callout (> - **Plan:** `path`), you must use that plan as the implementation contract. Acknowledge the plan and describe how you will follow it.".to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(fixture.to_string()),
        }],
        tools: None,
        max_tokens: 1000,
        thinking: None,
    };

    match provider.send_message(&request).await {
        Ok(response) => {
            let text = response.text().to_string();
            let latency = start.elapsed().as_millis() as u64;

            if text.trim().is_empty() {
                return RoleScenarioResult::fail(
                    "plan_callout_recognition",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must reference the specific plan path or plan-on-branch concept
            let lower = text.to_lowercase();
            let recognizes_plan = lower.contains("docs/plans/")
                || lower.contains("plan-on-branch")
                || lower.contains("plan file")
                || (lower.contains("plan") && lower.contains("contract"));
            if !recognizes_plan {
                return RoleScenarioResult::fail(
                    "plan_callout_recognition",
                    FailureClass::ContractViolation,
                    "Response does not acknowledge the plan-on-branch (requires referencing docs/plans/ path, 'plan-on-branch', 'plan file', or 'plan' + 'contract')".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "plan_callout_recognition",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            RoleScenarioResult::fail(
                "plan_callout_recognition",
                FailureClass::TransportError,
                format!("{e}"),
                None,
                None,
                latency,
            )
        }
    }
}
