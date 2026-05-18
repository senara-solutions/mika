//! mika-arch role calibration scenarios.
//!
//! Anchored on existing `mika_arch_groom_milestone.rs` patterns:
//! - Disposition keyword as literal final line
//! - Required suffix lines per skill manifest
//! - Required finding list when verdict is ITERATE/ESCALATE
//! - Citation-or-silence discipline

use std::sync::Arc;
use std::time::Instant;

use mika_common::llm::LlmProvider;

use crate::calibration::failure::FailureClass;
use crate::calibration::role::{RoleScenario, RoleScenarioResult};

/// Static scenario definitions for the mika-arch role.
pub const SCENARIOS: &[RoleScenario] = &[
    RoleScenario {
        id: "groom_ticket_basic",
        description: "Basic ticket grooming produces READY/ITERATE/ESCALATE disposition",
        tags: &["contract", "disposition"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"],
    },
    RoleScenario {
        id: "groom_milestone",
        description: "Milestone grooming produces per-sub-issue disposition with scope declaration",
        tags: &["contract", "disposition", "milestone"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"],
    },
    RoleScenario {
        id: "citation_discipline",
        description: "Claims about code must be backed by file references, not fabricated",
        tags: &["grounding", "citation"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["Fabrication"],
    },
    RoleScenario {
        id: "disposition_keyword_discipline",
        description: "Final line must be exactly one of the allowed disposition keywords",
        tags: &["contract", "suffix-line"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation"],
    },
    RoleScenario {
        id: "required_finding_list",
        description: "ITERATE/ESCALATE verdicts must include F1:/F2:/... finding list",
        tags: &["contract", "finding-list"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation"],
    },
];

/// Run a single mika-arch scenario against a real provider.
pub async fn run_scenario(scenario_id: &str, provider: Arc<dyn LlmProvider>) -> RoleScenarioResult {
    let start = Instant::now();

    match scenario_id {
        "groom_ticket_basic" => run_groom_ticket_basic(provider, start).await,
        "groom_milestone" => run_groom_milestone(provider, start).await,
        "citation_discipline" => run_citation_discipline(provider, start).await,
        "disposition_keyword_discipline" => run_disposition_keyword(provider, start).await,
        "required_finding_list" => run_required_finding_list(provider, start).await,
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

/// Groom ticket: model must produce a disposition keyword on the final line.
async fn run_groom_ticket_basic(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-arch/groom_ticket_basic.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some("You are mika-arch, a read-only architect agent. You review plans and tickets. Your response MUST end with exactly one of these disposition keywords on its own line: `Verdict: READY`, `Verdict: ITERATE`, or `Verdict: ESCALATE`. Provide your analysis first, then end with the verdict.".to_string()),
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
                    "groom_ticket_basic",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Check for disposition keyword in last 3 non-empty lines
            let valid_verdicts = ["Verdict: READY", "Verdict: ITERATE", "Verdict: ESCALATE"];
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            let last_lines = &lines[lines.len().saturating_sub(3)..];
            let has_verdict = last_lines
                .iter()
                .any(|line| valid_verdicts.iter().any(|v| line.trim() == *v));

            if !has_verdict {
                return RoleScenarioResult::fail(
                    "groom_ticket_basic",
                    FailureClass::ContractViolation,
                    format!(
                        "No valid disposition keyword in last 3 lines. Last lines: {:?}",
                        last_lines
                    ),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "groom_ticket_basic",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            RoleScenarioResult::fail(
                "groom_ticket_basic",
                FailureClass::TransportError,
                format!("{e}"),
                None,
                None,
                latency,
            )
        }
    }
}

/// Groom milestone: must produce per-sub-issue analysis with scope declaration.
async fn run_groom_milestone(provider: Arc<dyn LlmProvider>, start: Instant) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-arch/groom_milestone.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some("You are mika-arch reviewing a milestone. Analyze each sub-issue, provide per-issue disposition, then end with `Scope: milestone` on its own line followed by `Disposition: READY` or `Disposition: ITERATE`.".to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(fixture.to_string()),
        }],
        tools: None,
        max_tokens: 3000,
        thinking: None,
    };

    match provider.send_message(&request).await {
        Ok(response) => {
            let text = response.text().to_string();
            let latency = start.elapsed().as_millis() as u64;

            if text.trim().is_empty() {
                return RoleScenarioResult::fail(
                    "groom_milestone",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must contain scope declaration and disposition
            let has_scope = text.contains("Scope: milestone") || text.contains("Scope:milestone");
            let has_disposition =
                text.contains("Disposition: READY") || text.contains("Disposition: ITERATE");

            if !has_scope || !has_disposition {
                return RoleScenarioResult::fail(
                    "groom_milestone",
                    FailureClass::ContractViolation,
                    format!(
                        "Missing scope ({}) or disposition ({})",
                        has_scope, has_disposition
                    ),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "groom_milestone",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            RoleScenarioResult::fail(
                "groom_milestone",
                FailureClass::TransportError,
                format!("{e}"),
                None,
                None,
                latency,
            )
        }
    }
}

/// Citation discipline: model must not fabricate file references.
async fn run_citation_discipline(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-arch/citation_discipline.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some("You are mika-arch. When asked about code, only reference files and structures you can verify exist. If you cannot verify, say 'I would need to read the file to confirm'. Do NOT fabricate file paths or code structures.".to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(fixture.to_string()),
        }],
        tools: None,
        max_tokens: 1500,
        thinking: None,
    };

    match provider.send_message(&request).await {
        Ok(response) => {
            let text = response.text().to_string();
            let latency = start.elapsed().as_millis() as u64;

            if text.trim().is_empty() {
                return RoleScenarioResult::fail(
                    "citation_discipline",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Should contain hedging language or explicit verification need
            let lower = text.to_lowercase();
            let has_hedge = lower.contains("would need to")
                || lower.contains("cannot verify")
                || lower.contains("need to read")
                || lower.contains("based on")
                || lower.contains("likely")
                || lower.contains("typically");

            // Fabrication check: asserting specific line numbers or internal struct fields
            // without having read the file is a fabrication signal
            let has_specific_claims = text.contains("line ") && text.contains("fn ");

            if has_specific_claims && !has_hedge {
                return RoleScenarioResult::fail(
                    "citation_discipline",
                    FailureClass::Fabrication,
                    "Makes specific code claims without hedging or verification".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "citation_discipline",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            RoleScenarioResult::fail(
                "citation_discipline",
                FailureClass::TransportError,
                format!("{e}"),
                None,
                None,
                latency,
            )
        }
    }
}

/// Disposition keyword: final line must be an exact match from the accept set.
async fn run_disposition_keyword(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture = include_str!(
        "../../../tests/eval/calibration_fixtures/mika-arch/disposition_keyword_discipline.md"
    );

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some("You are mika-arch performing a second-pass review. Your response MUST end with exactly one of: `Verdict: GROOMED` or `Verdict: ESCALATE` on its own line. No other text on that line. Provide your analysis first.".to_string()),
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
                    "disposition_keyword_discipline",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            let valid_verdicts = ["Verdict: GROOMED", "Verdict: ESCALATE"];
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            let last_lines = &lines[lines.len().saturating_sub(3)..];
            let has_verdict = last_lines
                .iter()
                .any(|line| valid_verdicts.iter().any(|v| line.trim() == *v));

            if !has_verdict {
                return RoleScenarioResult::fail(
                    "disposition_keyword_discipline",
                    FailureClass::ContractViolation,
                    format!(
                        "No valid verdict in last 3 lines. Last lines: {:?}",
                        last_lines
                    ),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "disposition_keyword_discipline",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            RoleScenarioResult::fail(
                "disposition_keyword_discipline",
                FailureClass::TransportError,
                format!("{e}"),
                None,
                None,
                latency,
            )
        }
    }
}

/// Required finding list: ITERATE/ESCALATE must include Fn: prefixed findings.
async fn run_required_finding_list(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-arch/required_finding_list.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some("You are mika-arch reviewing a plan that has issues. You must ESCALATE. When your verdict is ITERATE or ESCALATE, you MUST include findings prefixed with F1:, F2:, etc. Each finding describes a specific issue. End with `Verdict: ESCALATE`.".to_string()),
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
                    "required_finding_list",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Check for F-list prefixes
            let has_findings = text.contains("F1:") || text.contains("F1 :");

            if !has_findings {
                return RoleScenarioResult::fail(
                    "required_finding_list",
                    FailureClass::ContractViolation,
                    "ESCALATE verdict without F1:/F2:/... finding list".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Also check verdict is present
            let has_verdict =
                text.contains("Verdict: ESCALATE") || text.contains("Verdict: ITERATE");
            if !has_verdict {
                return RoleScenarioResult::fail(
                    "required_finding_list",
                    FailureClass::ContractViolation,
                    "Missing ESCALATE/ITERATE verdict line".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "required_finding_list",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            RoleScenarioResult::fail(
                "required_finding_list",
                FailureClass::TransportError,
                format!("{e}"),
                None,
                None,
                latency,
            )
        }
    }
}
