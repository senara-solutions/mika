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
use crate::calibration::roles::llm_error_result;

/// Static scenario definitions for the mika-arch role.
pub const SCENARIOS: &[RoleScenario] = &[
    RoleScenario {
        id: "groom_ticket_basic",
        description: "Basic ticket grooming produces READY/ITERATE/ESCALATE disposition",
        tags: &["contract", "disposition"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"],
        diagnostic: false,
    },
    RoleScenario {
        id: "groom_milestone",
        description: "Milestone grooming produces per-sub-issue disposition with scope declaration",
        tags: &["contract", "disposition", "milestone"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"],
        diagnostic: false,
    },
    RoleScenario {
        id: "citation_discipline",
        description: "Claims about code must be backed by file references, not fabricated",
        tags: &["grounding", "citation"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["Fabrication"],
        diagnostic: false,
    },
    RoleScenario {
        id: "disposition_keyword_discipline",
        description: "Final line must be exactly one of the allowed disposition keywords",
        tags: &["contract", "suffix-line"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation"],
        diagnostic: false,
    },
    RoleScenario {
        id: "required_finding_list",
        description: "ITERATE/ESCALATE verdicts must include F1:/F2:/... finding list",
        tags: &["contract", "finding-list"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation"],
        diagnostic: false,
    },
    RoleScenario {
        id: "groomed_no_tbds_passes",
        description: "Clean plan with no TBDs must produce READY (TBD gate does not fire)",
        tags: &["contract", "tbd-gate"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation"],
        diagnostic: false,
    },
    RoleScenario {
        id: "groomed_with_tbd_rejected",
        description: "Plan with TBD tokens must produce ITERATE with findings naming the TBDs",
        tags: &["contract", "tbd-gate"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation"],
        diagnostic: false,
    },
    RoleScenario {
        id: "groomed_with_placeholder_path_rejected",
        description: "Plan with <path> placeholders must produce ITERATE with findings",
        tags: &["contract", "tbd-gate"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation"],
        diagnostic: false,
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
        "groomed_no_tbds_passes" => run_groomed_no_tbds_passes(provider, start).await,
        "groomed_with_tbd_rejected" => run_groomed_with_tbd_rejected(provider, start).await,
        "groomed_with_placeholder_path_rejected" => {
            run_groomed_with_placeholder_path_rejected(provider, start).await
        }
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
        Err(e) => llm_error_result("groom_ticket_basic", e, start.elapsed().as_millis() as u64),
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
        Err(e) => llm_error_result("groom_milestone", e, start.elapsed().as_millis() as u64),
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
        Err(e) => llm_error_result("citation_discipline", e, start.elapsed().as_millis() as u64),
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
        Err(e) => llm_error_result(
            "disposition_keyword_discipline",
            e,
            start.elapsed().as_millis() as u64,
        ),
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
        Err(e) => llm_error_result(
            "required_finding_list",
            e,
            start.elapsed().as_millis() as u64,
        ),
    }
}

/// TBD gate — clean plan: model must produce READY (no TBDs to reject).
async fn run_groomed_no_tbds_passes(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture = include_str!(
        "../../../tests/eval/calibration_fixtures/mika-arch/groomed_no_tbds_passes.md"
    );

    let system =
        include_str!("../../../../../skills/bundled/mika-arch-groom-ticket/system_prompt.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(system.to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(format!(
                "Review this plan for ticket mika#1244-test:\n\n{}",
                fixture
            )),
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
                    "groomed_no_tbds_passes",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must produce READY — the plan has no TBDs
            let valid_verdicts = ["Disposition: READY", "Verdict: READY"];
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            let last_lines = &lines[lines.len().saturating_sub(3)..];
            let has_ready = last_lines
                .iter()
                .any(|line| valid_verdicts.iter().any(|v| line.trim() == *v));

            if !has_ready {
                return RoleScenarioResult::fail(
                    "groomed_no_tbds_passes",
                    FailureClass::ContractViolation,
                    format!(
                        "Expected READY for clean plan, got last lines: {:?}",
                        last_lines
                    ),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "groomed_no_tbds_passes",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => llm_error_result(
            "groomed_no_tbds_passes",
            e,
            start.elapsed().as_millis() as u64,
        ),
    }
}

/// TBD gate — plan with TBD tokens: model must produce ITERATE with findings.
async fn run_groomed_with_tbd_rejected(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture = include_str!(
        "../../../tests/eval/calibration_fixtures/mika-arch/groomed_with_tbd_rejected.md"
    );

    let system =
        include_str!("../../../../../skills/bundled/mika-arch-groom-ticket/system_prompt.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(system.to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(format!(
                "Review this plan for ticket mika#1244-test:\n\n{}",
                fixture
            )),
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
                    "groomed_with_tbd_rejected",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must NOT produce READY — the plan has TBDs
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            let last_lines = &lines[lines.len().saturating_sub(3)..];
            let has_ready = last_lines
                .iter()
                .any(|line| line.trim() == "Disposition: READY" || line.trim() == "Verdict: READY");

            if has_ready {
                return RoleScenarioResult::fail(
                    "groomed_with_tbd_rejected",
                    FailureClass::ContractViolation,
                    "Plan with TBDs was approved as READY — TBD gate failed".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must produce ITERATE or ESCALATE
            let has_iterate_or_escalate = last_lines.iter().any(|line| {
                let trimmed = line.trim();
                trimmed == "Disposition: ITERATE"
                    || trimmed == "Verdict: ITERATE"
                    || trimmed == "Disposition: ESCALATE"
                    || trimmed == "Verdict: ESCALATE"
            });

            if !has_iterate_or_escalate {
                return RoleScenarioResult::fail(
                    "groomed_with_tbd_rejected",
                    FailureClass::ContractViolation,
                    format!(
                        "Expected ITERATE or ESCALATE for TBD plan, got last lines: {:?}",
                        last_lines
                    ),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must have at least one finding (F1:)
            let has_findings = text.contains("F1:");
            if !has_findings {
                return RoleScenarioResult::fail(
                    "groomed_with_tbd_rejected",
                    FailureClass::ContractViolation,
                    "ITERATE/ESCALATE without F-list findings naming the TBDs".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "groomed_with_tbd_rejected",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => llm_error_result(
            "groomed_with_tbd_rejected",
            e,
            start.elapsed().as_millis() as u64,
        ),
    }
}

/// TBD gate — plan with placeholder paths: model must produce ITERATE with findings.
async fn run_groomed_with_placeholder_path_rejected(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture = include_str!(
        "../../../tests/eval/calibration_fixtures/mika-arch/groomed_with_placeholder_path_rejected.md"
    );

    let system =
        include_str!("../../../../../skills/bundled/mika-arch-groom-ticket/system_prompt.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(system.to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(format!(
                "Review this plan for ticket mika#1244-test:\n\n{}",
                fixture
            )),
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
                    "groomed_with_placeholder_path_rejected",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must NOT produce READY — the plan has placeholder paths
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            let last_lines = &lines[lines.len().saturating_sub(3)..];
            let has_ready = last_lines
                .iter()
                .any(|line| line.trim() == "Disposition: READY" || line.trim() == "Verdict: READY");

            if has_ready {
                return RoleScenarioResult::fail(
                    "groomed_with_placeholder_path_rejected",
                    FailureClass::ContractViolation,
                    "Plan with <path> placeholders was approved as READY — TBD gate failed"
                        .to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must produce ITERATE or ESCALATE
            let has_iterate_or_escalate = last_lines.iter().any(|line| {
                let trimmed = line.trim();
                trimmed == "Disposition: ITERATE"
                    || trimmed == "Verdict: ITERATE"
                    || trimmed == "Disposition: ESCALATE"
                    || trimmed == "Verdict: ESCALATE"
            });

            if !has_iterate_or_escalate {
                return RoleScenarioResult::fail(
                    "groomed_with_placeholder_path_rejected",
                    FailureClass::ContractViolation,
                    format!(
                        "Expected ITERATE or ESCALATE for placeholder plan, got last lines: {:?}",
                        last_lines
                    ),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must have at least one finding
            let has_findings = text.contains("F1:");
            if !has_findings {
                return RoleScenarioResult::fail(
                    "groomed_with_placeholder_path_rejected",
                    FailureClass::ContractViolation,
                    "ITERATE/ESCALATE without F-list findings naming the placeholders".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "groomed_with_placeholder_path_rejected",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => llm_error_result(
            "groomed_with_placeholder_path_rejected",
            e,
            start.elapsed().as_millis() as u64,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Check if the last N non-empty lines contain any of the given verdicts.
    fn last_lines_contain_verdict(text: &str, verdicts: &[&str], window: usize) -> bool {
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let last_lines = &lines[lines.len().saturating_sub(window)..];
        last_lines
            .iter()
            .any(|line| verdicts.iter().any(|v| line.trim() == *v))
    }

    // --- Fixture invariants ---

    #[test]
    fn fixture_tbd_rejected_contains_tbd_tokens() {
        let fixture = include_str!(
            "../../../tests/eval/calibration_fixtures/mika-arch/groomed_with_tbd_rejected.md"
        );
        assert!(
            fixture.contains("TBD"),
            "TBD-rejection fixture must contain literal TBD tokens"
        );
    }

    #[test]
    fn fixture_placeholder_rejected_contains_angle_bracket_placeholders() {
        let fixture = include_str!(
            "../../../tests/eval/calibration_fixtures/mika-arch/groomed_with_placeholder_path_rejected.md"
        );
        assert!(
            fixture.contains("<path>"),
            "Placeholder-rejection fixture must contain <path> placeholders"
        );
    }

    #[test]
    fn fixture_clean_plan_has_no_tbd_tokens_in_body() {
        let fixture = include_str!(
            "../../../tests/eval/calibration_fixtures/mika-arch/groomed_no_tbds_passes.md"
        );
        // Skip heading lines (metadata) — check only plan body content
        let body: String = fixture
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
        assert!(
            !body.contains("tbd"),
            "Clean-plan fixture body must not contain TBD tokens"
        );
        assert!(
            !fixture.contains("<path>") && !fixture.contains("<version>"),
            "Clean-plan fixture must not contain angle-bracket placeholders"
        );
    }

    // --- System prompt invariants ---

    #[test]
    fn groom_ticket_prompt_contains_unresolved_decision_gate() {
        let prompt =
            include_str!("../../../../../skills/bundled/mika-arch-groom-ticket/system_prompt.md");
        assert!(
            prompt.contains("Unresolved-Decision Gate"),
            "groom-ticket system prompt must contain the Unresolved-Decision Gate section"
        );
        assert!(
            prompt.contains("TBD"),
            "gate must enumerate TBD as an unresolved-decision pattern"
        );
        assert!(
            prompt.contains("Acceptance-Criteria Gate (mika#1559)"),
            "groom-ticket system prompt must contain the Acceptance-Criteria Gate section (mika#1559)"
        );
    }

    #[test]
    fn second_review_prompt_contains_unresolved_decision_gate() {
        let prompt =
            include_str!("../../../../../skills/bundled/mika-arch-second-review/system_prompt.md");
        assert!(
            prompt.contains("Unresolved-Decision Gate"),
            "second-review system prompt must contain the Unresolved-Decision Gate section"
        );
        assert!(
            prompt.contains("Acceptance-Criteria Gate (mika#1559)"),
            "second-review system prompt must contain the Acceptance-Criteria Gate section (mika#1559)"
        );
    }

    // --- Assertion logic regression tests ---

    #[test]
    fn verdict_ready_detected_in_last_three_lines() {
        let text = "Analysis complete.\n\nVerdict: READY\n";
        let ready_verdicts = ["Disposition: READY", "Verdict: READY"];
        assert!(last_lines_contain_verdict(text, &ready_verdicts, 3));
    }

    #[test]
    fn verdict_iterate_rejects_ready_for_tbd_plan() {
        // Simulate a correct rejection: ITERATE with findings
        let text = "The plan has unresolved decisions.\n\n\
                     F1: (BLOCKING) Auth method is TBD — pick HMAC-SHA256 or API key\n\
                     F2: (BLOCKING) Port number is TBD\n\n\
                     Verdict: ITERATE\n";

        let ready_verdicts = ["Disposition: READY", "Verdict: READY"];
        assert!(
            !last_lines_contain_verdict(text, &ready_verdicts, 3),
            "TBD plan must NOT get READY"
        );

        let iterate_verdicts = [
            "Disposition: ITERATE",
            "Verdict: ITERATE",
            "Disposition: ESCALATE",
            "Verdict: ESCALATE",
        ];
        assert!(
            last_lines_contain_verdict(text, &iterate_verdicts, 3),
            "TBD plan must get ITERATE or ESCALATE"
        );

        assert!(text.contains("F1:"), "Must include F-list findings");
    }

    #[test]
    fn false_ready_on_tbd_plan_caught_by_assertion() {
        // Simulate the failure mode: LLM incorrectly returns READY for a TBD plan
        let text = "Plan looks good.\n\nVerdict: READY\n";
        let ready_verdicts = ["Disposition: READY", "Verdict: READY"];
        assert!(
            last_lines_contain_verdict(text, &ready_verdicts, 3),
            "Should detect READY so the scenario can flag it as a contract violation"
        );
    }

    // --- Scenario metadata ---

    #[test]
    fn scenario_count_is_eight() {
        assert_eq!(
            SCENARIOS.len(),
            8,
            "mika-arch should have 8 calibration scenarios (5 original + 3 TBD-gate)"
        );
    }

    #[test]
    fn tbd_gate_scenarios_present_with_correct_tags() {
        let tbd_scenarios: Vec<_> = SCENARIOS
            .iter()
            .filter(|s| s.tags.contains(&"tbd-gate"))
            .collect();
        assert_eq!(
            tbd_scenarios.len(),
            3,
            "Expected 3 tbd-gate tagged scenarios"
        );

        let ids: Vec<&str> = tbd_scenarios.iter().map(|s| s.id).collect();
        assert!(ids.contains(&"groomed_no_tbds_passes"));
        assert!(ids.contains(&"groomed_with_tbd_rejected"));
        assert!(ids.contains(&"groomed_with_placeholder_path_rejected"));
    }

    #[test]
    fn tbd_rejection_scenarios_are_not_flaky() {
        for scenario in SCENARIOS.iter().filter(|s| s.tags.contains(&"tbd-gate")) {
            assert!(
                !scenario.flaky,
                "TBD-gate scenario '{}' must not be marked flaky — it's a hard contract",
                scenario.id
            );
        }
    }
}
