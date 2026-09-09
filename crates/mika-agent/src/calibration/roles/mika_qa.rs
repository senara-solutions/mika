//! mika-qa role calibration scenarios.
//!
//! Anchored on the verdict contract parsed by `server::verdict_handler`:
//! - `(?mi)^VERDICT:\s*(.+)$` regex routes on exact substrings
//! - Per-AC enumeration (`[✅]`/`[❌]`) consumed by `parse_verdict_unsatisfied_acs`
//! - Rescue-class PR detection (wip prefix + rescue-pipeline-verified marker)
//!
//! 5 scenarios, all structural assertions (no LLM-as-judge).

use std::sync::Arc;
use std::time::Instant;

use mika_common::llm::LlmProvider;

use crate::calibration::failure::FailureClass;
use crate::calibration::role::{RoleScenario, RoleScenarioResult};
use crate::calibration::roles::llm_error_result;

/// Static scenario definitions for the mika-qa role.
pub const SCENARIOS: &[RoleScenario] = &[
    RoleScenario {
        id: "verdict_format_precision",
        description: "All ACs satisfied — must emit exactly `VERDICT: pass` with correct casing and required sections",
        tags: &["verdict", "format", "contract"],
        flaky: false,
        weight: 1.0,
        expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"],
    },
    RoleScenario {
        id: "per_ac_enumeration",
        description: "3/5 ACs satisfied — must enumerate each AC with checkmark/cross markers and emit block[ac]",
        tags: &["verdict", "enumeration", "contract"],
        flaky: false,
        weight: 1.5,
        expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"],
    },
    RoleScenario {
        id: "absence_claim_grounding",
        description: "AC asserts absence — must cite evidence or emit block[ac], never claim absence without grounding",
        tags: &["grounding", "absence", "evidence"],
        flaky: false,
        weight: 2.0,
        expected_failure_classes_absent: &["Fabrication", "EmptyResponse"],
    },
    RoleScenario {
        id: "wip_rescue_skip",
        description: "Auto-rescued draft PR with wip() commit — must NOT emit VERDICT: pass",
        tags: &["rescue", "wip", "draft"],
        flaky: false,
        weight: 2.0,
        expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"],
    },
    RoleScenario {
        id: "no_fabricated_fix",
        description: "Replay consistency — same fixture twice must produce same verdict and unsatisfied AC",
        tags: &["consistency", "replay", "fabrication"],
        flaky: false,
        weight: 2.0,
        expected_failure_classes_absent: &["Fabrication", "ContractViolation", "EmptyResponse"],
    },
    RoleScenario {
        id: "duplicate_claim_grounded",
        description: "Cross-artifact equivalence — must cite compared file sets or hedge, never bare 'content identical' (mika#1645)",
        tags: &["grounding", "equivalence", "cross-artifact"],
        flaky: false,
        weight: 2.0,
        expected_failure_classes_absent: &["Fabrication", "EmptyResponse"],
    },
    RoleScenario {
        id: "negative_test_invariant_gate",
        description: "Positive-only tests on an in-perimeter PR must NOT pass (2.5.4b); the same diff out of perimeter must not be blocked on 2.5.4b grounds (mika#2264)",
        tags: &["invariant", "negative-test", "perimeter", "gating"],
        flaky: false,
        weight: 2.0,
        expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"],
    },
    RoleScenario {
        id: "verdict_format_canonical_shape",
        description: "VERDICT line must be canonical (no `**` wrapper, no alias tokens) — drift-prevention for mika#1828 parser tolerance",
        tags: &["verdict", "format", "canonical", "drift"],
        flaky: false,
        weight: 1.5,
        expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"],
    },
];

/// Run a single mika-qa scenario against a real provider.
pub async fn run_scenario(scenario_id: &str, provider: Arc<dyn LlmProvider>) -> RoleScenarioResult {
    let start = Instant::now();

    match scenario_id {
        "verdict_format_precision" => run_verdict_format_precision(provider, start).await,
        "per_ac_enumeration" => run_per_ac_enumeration(provider, start).await,
        "absence_claim_grounding" => run_absence_claim_grounding(provider, start).await,
        "wip_rescue_skip" => run_wip_rescue_skip(provider, start).await,
        "no_fabricated_fix" => run_no_fabricated_fix(provider, start).await,
        "duplicate_claim_grounded" => run_duplicate_claim_grounded(provider, start).await,
        "verdict_format_canonical_shape" => {
            run_verdict_format_canonical_shape(provider, start).await
        }
        "negative_test_invariant_gate" => run_negative_test_invariant_gate(provider, start).await,
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

/// Verdict format precision: all ACs satisfied, model must emit exactly `VERDICT: pass`
/// with correct casing and required sections.
async fn run_verdict_format_precision(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture = include_str!(
        "../../../tests/eval/calibration_fixtures/mika-qa/verdict_format_precision.md"
    );

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(
            "You are mika-qa, a specialist PR reviewer. Review the PR below and produce a \
             structured verdict. All ACs are satisfied. Your response MUST contain exactly \
             `VERDICT: pass` (uppercase VERDICT, colon, space, lowercase pass). Include DEPTH, \
             REASON, DIFF ANALYSIS, and PLAN-AC VERIFICATION sections."
                .to_string(),
        ),
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
                    "verdict_format_precision",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must contain exact `VERDICT: pass` (case-sensitive)
            if !text.contains("VERDICT: pass") {
                return RoleScenarioResult::fail(
                    "verdict_format_precision",
                    FailureClass::ContractViolation,
                    "Response does not contain exact 'VERDICT: pass' (case-sensitive)".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Reject wrong casing: `Verdict: pass`
            if text.contains("Verdict: pass") && !text.contains("VERDICT: pass") {
                return RoleScenarioResult::fail(
                    "verdict_format_precision",
                    FailureClass::ContractViolation,
                    "Wrong casing: 'Verdict: pass' instead of 'VERDICT: pass'".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Reject missing space: `VERDICT:pass`
            if text.contains("VERDICT:pass") {
                return RoleScenarioResult::fail(
                    "verdict_format_precision",
                    FailureClass::ContractViolation,
                    "Missing space after colon: 'VERDICT:pass'".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must contain DEPTH section
            if !text.contains("DEPTH:") && !text.contains("DEPTH") {
                return RoleScenarioResult::fail(
                    "verdict_format_precision",
                    FailureClass::ContractViolation,
                    "Response missing DEPTH section".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must contain DIFF ANALYSIS section
            if !text.contains("DIFF ANALYSIS") {
                return RoleScenarioResult::fail(
                    "verdict_format_precision",
                    FailureClass::ContractViolation,
                    "Response missing DIFF ANALYSIS section".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "verdict_format_precision",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => llm_error_result(
            "verdict_format_precision",
            e,
            start.elapsed().as_millis() as u64,
        ),
    }
}

/// Per-AC enumeration: 3/5 ACs satisfied, model must enumerate each with markers.
async fn run_per_ac_enumeration(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-qa/per_ac_enumeration.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(
            "You are mika-qa. Review the PR below. 3 of 5 ACs are satisfied, 2 are unsatisfied. \
             For EACH AC, emit a line starting with `[✅]` (satisfied) or `[❌]` (unsatisfied) \
             followed by the AC's short label. Include PLAN-AC VERIFICATION section. Emit \
             `VERDICT: block[ac]` since ACs are unsatisfied."
                .to_string(),
        ),
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
                    "per_ac_enumeration",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must contain VERDICT: block[ac]
            if !text.contains("VERDICT: block[ac]") {
                return RoleScenarioResult::fail(
                    "per_ac_enumeration",
                    FailureClass::ContractViolation,
                    "Response does not contain 'VERDICT: block[ac]'".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must contain at least 3 checkmarks
            let checkmark_count = text.matches("[✅]").count();
            if checkmark_count < 3 {
                return RoleScenarioResult::fail(
                    "per_ac_enumeration",
                    FailureClass::ContractViolation,
                    format!(
                        "Expected at least 3 [✅] markers, found {}",
                        checkmark_count
                    ),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must contain at least 2 cross marks
            let cross_count = text.matches("[❌]").count();
            if cross_count < 2 {
                return RoleScenarioResult::fail(
                    "per_ac_enumeration",
                    FailureClass::ContractViolation,
                    format!("Expected at least 2 [❌] markers, found {}", cross_count),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must contain PLAN-AC VERIFICATION section
            if !text.contains("PLAN-AC VERIFICATION") {
                return RoleScenarioResult::fail(
                    "per_ac_enumeration",
                    FailureClass::ContractViolation,
                    "Response missing PLAN-AC VERIFICATION section".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "per_ac_enumeration",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => llm_error_result("per_ac_enumeration", e, start.elapsed().as_millis() as u64),
    }
}

/// Absence claim grounding: model must cite evidence or block, never claim absence
/// without grounding.
async fn run_absence_claim_grounding(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-qa/absence_claim_grounding.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(
            "You are mika-qa. The PR has an AC asserting 'no null values in JSON response fields'. \
             You must either cite specific code evidence (line number, grep result, or quoted code) \
             proving the absence, or emit `block[ac]` with a structured reason explaining what you \
             could not verify. Never claim absence without evidence."
                .to_string(),
        ),
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
                    "absence_claim_grounding",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            let lower = text.to_lowercase();

            // Evidence markers that indicate grounding
            let evidence_markers = [
                "line",
                "grep",
                "search",
                "found",
                "verified",
                "confirmed",
                "`",
            ];
            let has_evidence = evidence_markers.iter().any(|m| lower.contains(m));
            let has_block = text.contains("VERDICT: block[ac]");

            // Must have either evidence or block[ac]
            if !has_evidence && !has_block {
                return RoleScenarioResult::fail(
                    "absence_claim_grounding",
                    FailureClass::ContractViolation,
                    "Response has neither evidence markers nor VERDICT: block[ac]".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // If response claims pass, it MUST have evidence (not just block)
            if lower.contains("verdict: pass") && !has_evidence {
                return RoleScenarioResult::fail(
                    "absence_claim_grounding",
                    FailureClass::Fabrication,
                    "Claimed VERDICT: pass without evidence markers — fabricated absence claim"
                        .to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "absence_claim_grounding",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => llm_error_result(
            "absence_claim_grounding",
            e,
            start.elapsed().as_millis() as u64,
        ),
    }
}

/// WIP rescue skip: auto-rescued draft PR must NOT get VERDICT: pass.
async fn run_wip_rescue_skip(provider: Arc<dyn LlmProvider>, start: Instant) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-qa/wip_rescue_skip.md");

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(
            "You are mika-qa. The PR below is an auto-rescued draft PR with a `wip(` commit \
             prefix and `<!-- rescue-pipeline-verified: no -->` marker. You MUST NOT approve this \
             PR. Emit `hold[review]` or an appropriate `block[…]` verdict."
                .to_string(),
        ),
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
                    "wip_rescue_skip",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must NOT contain VERDICT: pass (case-insensitive)
            let lower = text.to_lowercase();
            if lower.contains("verdict: pass") {
                return RoleScenarioResult::fail(
                    "wip_rescue_skip",
                    FailureClass::ContractViolation,
                    "Response contains 'VERDICT: pass' for a wip-rescue draft PR".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Must emit SOME verdict
            if !lower.contains("verdict:") {
                return RoleScenarioResult::fail(
                    "wip_rescue_skip",
                    FailureClass::ContractViolation,
                    "Response does not contain any VERDICT: line".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "wip_rescue_skip",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => llm_error_result("wip_rescue_skip", e, start.elapsed().as_millis() as u64),
    }
}

/// No fabricated fix: send the same fixture twice and assert both responses agree.
async fn run_no_fabricated_fix(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture =
        include_str!("../../../tests/eval/calibration_fixtures/mika-qa/no_fabricated_fix.md");

    let system = "You are mika-qa. The PR below has 4 ACs. 3 are satisfied. AC4 (test file at \
                  `tests/eval/calibration_diff_test.rs`) is unsatisfied — the diff shows no such \
                  file was added. Emit `VERDICT: block[ac]` and mark AC4 with `[❌]`."
        .to_string();

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(system.clone()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(fixture.to_string()),
        }],
        tools: None,
        max_tokens: 2000,
        thinking: None,
    };

    // First call
    let response1 = match provider.send_message(&request).await {
        Ok(r) => r,
        Err(e) => {
            return llm_error_result("no_fabricated_fix", e, start.elapsed().as_millis() as u64);
        }
    };

    let text1 = response1.text().to_string();
    let tokens1_in = response1.usage.input_tokens;
    let tokens1_out = response1.usage.output_tokens;

    if text1.trim().is_empty() {
        return RoleScenarioResult::fail(
            "no_fabricated_fix",
            FailureClass::EmptyResponse,
            "First call returned empty response".to_string(),
            Some(tokens1_in),
            Some(tokens1_out),
            start.elapsed().as_millis() as u64,
        );
    }

    // Second call — same request
    let request2 = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(system),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(fixture.to_string()),
        }],
        tools: None,
        max_tokens: 2000,
        thinking: None,
    };

    let response2 = match provider.send_message(&request2).await {
        Ok(r) => r,
        Err(e) => {
            return llm_error_result("no_fabricated_fix", e, start.elapsed().as_millis() as u64);
        }
    };

    let text2 = response2.text().to_string();
    let total_in = tokens1_in + response2.usage.input_tokens;
    let total_out = tokens1_out + response2.usage.output_tokens;
    let latency = start.elapsed().as_millis() as u64;

    if text2.trim().is_empty() {
        return RoleScenarioResult::fail(
            "no_fabricated_fix",
            FailureClass::EmptyResponse,
            "Second call returned empty response".to_string(),
            Some(total_in),
            Some(total_out),
            latency,
        );
    }

    // Both must contain VERDICT: block[ac]
    if !text1.contains("VERDICT: block[ac]") {
        return RoleScenarioResult::fail(
            "no_fabricated_fix",
            FailureClass::ContractViolation,
            "First response missing 'VERDICT: block[ac]'".to_string(),
            Some(total_in),
            Some(total_out),
            latency,
        );
    }
    if !text2.contains("VERDICT: block[ac]") {
        return RoleScenarioResult::fail(
            "no_fabricated_fix",
            FailureClass::ContractViolation,
            "Second response missing 'VERDICT: block[ac]'".to_string(),
            Some(total_in),
            Some(total_out),
            latency,
        );
    }

    // Both must contain [❌]
    if !text1.contains("[❌]") {
        return RoleScenarioResult::fail(
            "no_fabricated_fix",
            FailureClass::ContractViolation,
            "First response missing [❌] marker".to_string(),
            Some(total_in),
            Some(total_out),
            latency,
        );
    }
    if !text2.contains("[❌]") {
        return RoleScenarioResult::fail(
            "no_fabricated_fix",
            FailureClass::ContractViolation,
            "Second response missing [❌] marker".to_string(),
            Some(total_in),
            Some(total_out),
            latency,
        );
    }

    // Both must reference the unsatisfied AC (AC4 or test file)
    let lower1 = text1.to_lowercase();
    let lower2 = text2.to_lowercase();
    let ac4_ref1 =
        lower1.contains("ac4") || lower1.contains("test") || lower1.contains("calibration_diff");
    let ac4_ref2 =
        lower2.contains("ac4") || lower2.contains("test") || lower2.contains("calibration_diff");

    if !ac4_ref1 {
        return RoleScenarioResult::fail(
            "no_fabricated_fix",
            FailureClass::ContractViolation,
            "First response does not reference unsatisfied AC4/test file".to_string(),
            Some(total_in),
            Some(total_out),
            latency,
        );
    }
    if !ac4_ref2 {
        return RoleScenarioResult::fail(
            "no_fabricated_fix",
            FailureClass::ContractViolation,
            "Second response does not reference unsatisfied AC4/test file".to_string(),
            Some(total_in),
            Some(total_out),
            latency,
        );
    }

    // Neither may contain VERDICT: pass (fabricated fix)
    if lower1.contains("verdict: pass") {
        return RoleScenarioResult::fail(
            "no_fabricated_fix",
            FailureClass::Fabrication,
            "First response fabricated VERDICT: pass despite unsatisfied AC".to_string(),
            Some(total_in),
            Some(total_out),
            latency,
        );
    }
    if lower2.contains("verdict: pass") {
        return RoleScenarioResult::fail(
            "no_fabricated_fix",
            FailureClass::Fabrication,
            "Second response fabricated VERDICT: pass despite unsatisfied AC".to_string(),
            Some(total_in),
            Some(total_out),
            latency,
        );
    }

    RoleScenarioResult::pass("no_fabricated_fix", total_in, total_out, latency)
}

/// Cross-artifact equivalence grounding (mika#1645): the PR resembles a
/// previously-merged PR (recovery-class header + shared keyword) but has a
/// different file diff. The model must either cite the compared file sets/diff
/// OR hedge ("possible duplicate"). It must NEVER assert bare equivalence
/// ("content identical", "duplicate of", "same as") without a citation.
async fn run_duplicate_claim_grounded(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture = include_str!(
        "../../../tests/eval/calibration_fixtures/mika-qa/duplicate_claim_grounded.md"
    );

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(
            "You are mika-qa. The PR below resembles a previously-merged PR (same recovery-class \
             header, shared keyword), but its file diff differs. Cross-artifact equivalence \
             assertions (identical / duplicate of / same as / content identical) REQUIRE a cited \
             comparison of the two file sets. If you have not compared the file lists, downgrade \
             to hedged language: \"possible duplicate — operator should verify file diffs\". Never \
             emit \"content identical\" without citing the compared file sets. Co-occurring surface \
             signals (recovery headers, title keywords, core memory) are NOT grounding."
                .to_string(),
        ),
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
                    "duplicate_claim_grounded",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            let lower = text.to_lowercase();

            // Does the response assert cross-artifact equivalence (identity)?
            let asserts_equivalence = lower.contains("content identical")
                || lower.contains("duplicate of")
                || lower.contains("identical to")
                || lower.contains("same as")
                || lower.contains("equivalent to");

            // Hedged / non-asserting language (acceptable without a diff).
            let hedged = lower.contains("possible duplicate")
                || lower.contains("operator should verify")
                || lower.contains("could not verify")
                || lower.contains("not a duplicate")
                || lower.contains("distinct")
                || lower.contains("different file");

            // Citation of a file-set comparison (grounds an equivalence claim).
            let cites_comparison = (lower.contains("file")
                && (lower.contains("diff")
                    || lower.contains("compar")
                    || lower.contains("intersection")
                    || lower.contains("overlap")
                    || lower.contains("list")))
                || lower.contains("pr diff")
                || lower.contains("1638");

            // Fabrication: asserts identity without citing the comparison and
            // without hedging — the exact mika#1644 failure mode.
            if asserts_equivalence && !cites_comparison && !hedged {
                return RoleScenarioResult::fail(
                    "duplicate_claim_grounded",
                    FailureClass::Fabrication,
                    "Asserted cross-artifact equivalence without citing the compared file sets \
                     and without hedged language (mika#1645 failure mode)"
                        .to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "duplicate_claim_grounded",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => llm_error_result(
            "duplicate_claim_grounded",
            e,
            start.elapsed().as_millis() as u64,
        ),
    }
}

/// Verdict format canonical shape (mika#1828): the VERDICT line must be
/// canonical (no `**` markdown wrapper, no alias tokens like `REQUEST CHANGES`
/// or `APPROVE`). Drift-prevention for the parser tolerance added in mika#1828
/// — the parser now accepts drift, but the calibration guarantees the model
/// still emits canonical shape at model-swap time.
async fn run_verdict_format_canonical_shape(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    let fixture = include_str!(
        "../../../tests/eval/calibration_fixtures/mika-qa/verdict_format_canonical_shape.md"
    );

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(
            "You are mika-qa, a specialist PR reviewer. Review the PR below and produce a \
             structured verdict. The canonical VERDICT line format is EXACTLY one of: \
             `VERDICT: pass`, `VERDICT: block[ac]`, `VERDICT: block[ci]`, `VERDICT: block[security]`, \
             `VERDICT: block[pipeline]`, `VERDICT: hold[review]`. Do NOT wrap the VERDICT line in \
             markdown emphasis (`**...**`, `__...__`, `*...*`). Do NOT use GitHub-review-state \
             tokens like `REQUEST CHANGES`, `REQUEST_CHANGES`, `CHANGES_REQUESTED`, `APPROVE`, \
             `APPROVED` — those are aliases the parser tolerates but the emitted shape must be \
             canonical. Include DEPTH, REASON, DIFF ANALYSIS sections."
                .to_string(),
        ),
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
                    "verdict_format_canonical_shape",
                    FailureClass::EmptyResponse,
                    "Empty response".to_string(),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Locate the first line that (after trim) begins with `VERDICT:` OR
            // any emphasis wrapper thereof — accept the widest possible drift
            // as detection input, then classify.
            let mut verdict_line: Option<&str> = None;
            for line in text.lines() {
                let trimmed = line.trim();
                let stripped_leading = trimmed.trim_start_matches(['*', '_']);
                if stripped_leading
                    .to_ascii_uppercase()
                    .starts_with("VERDICT:")
                {
                    verdict_line = Some(trimmed);
                    break;
                }
            }

            let line = match verdict_line {
                Some(l) => l,
                None => {
                    return RoleScenarioResult::fail(
                        "verdict_format_canonical_shape",
                        FailureClass::ContractViolation,
                        "No VERDICT line found in response".to_string(),
                        Some(response.usage.input_tokens),
                        Some(response.usage.output_tokens),
                        latency,
                    );
                }
            };

            // Reject markdown-wrapped verdict line — must START with literal `VERDICT:`
            if !line.starts_with("VERDICT:") {
                return RoleScenarioResult::fail(
                    "verdict_format_canonical_shape",
                    FailureClass::ContractViolation,
                    format!(
                        "VERDICT line has markdown-emphasis wrapper (must start with literal 'VERDICT:'): {}",
                        line
                    ),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            // Extract the value after `VERDICT:` — must match exactly one of
            // the six canonical class-detail combinations.
            let value = line.trim_start_matches("VERDICT:").trim();
            let canonical_forms = [
                "pass",
                "block[ac]",
                "block[ci]",
                "block[security]",
                "block[pipeline]",
                "hold[review]",
            ];
            if !canonical_forms.contains(&value) {
                return RoleScenarioResult::fail(
                    "verdict_format_canonical_shape",
                    FailureClass::ContractViolation,
                    format!(
                        "VERDICT value not canonical ('{}'; expected one of: {})",
                        value,
                        canonical_forms.join(", ")
                    ),
                    Some(response.usage.input_tokens),
                    Some(response.usage.output_tokens),
                    latency,
                );
            }

            RoleScenarioResult::pass(
                "verdict_format_canonical_shape",
                response.usage.input_tokens,
                response.usage.output_tokens,
                latency,
            )
        }
        Err(e) => llm_error_result(
            "verdict_format_canonical_shape",
            e,
            start.elapsed().as_millis() as u64,
        ),
    }
}

/// Negative-test invariant gate (mika#2264): the reviewer must refuse `pass` on an
/// in-perimeter PR whose diff adds only positive assertions — and must NOT refuse the
/// same diff out of perimeter.
///
/// The two fixtures are deliberately isomorphic — same AC set (including the
/// "no test regressions" AC that Step 2.5.3 used to defer to CI), same three
/// positive-only tests, same counts. The only variable is the path. Without the
/// out-of-perimeter control this scenario would measure "the reviewer always
/// blocks", which is not the property under test.
async fn run_negative_test_invariant_gate(
    provider: Arc<dyn LlmProvider>,
    start: Instant,
) -> RoleScenarioResult {
    use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

    const ID: &str = "negative_test_invariant_gate";

    let in_perimeter = include_str!(
        "../../../tests/eval/calibration_fixtures/mika-qa/negative_test_invariant_gate.md"
    );
    let out_of_perimeter = include_str!(
        "../../../tests/eval/calibration_fixtures/mika-qa/negative_test_invariant_gate_out_of_perimeter.md"
    );

    // The scenario consumes the PRODUCTION prompt verbatim — `include_str!` of the
    // very file this PR edits — not a paraphrase of it. That coupling is what makes
    // the red-before/green-after calibration meaningful: revert
    // `skills/bundled/qa-review/system_prompt.md` to its pre-PR content and this
    // scenario fails, because the rule under test is no longer in the prompt. A
    // scenario that restated 2.5.4b inline would pass with or without the fix and
    // would measure nothing (mika#2264 AC5).
    const QA_REVIEW_PROMPT: &str =
        include_str!("../../../../../skills/bundled/qa-review/system_prompt.md");

    // Harness preamble — identical in both the red and green runs, so the ONLY
    // variable between them is the prompt body above. It neutralises the steps that
    // are out of scope here (tool calls, pipeline artifacts, GitHub posting) without
    // touching the AC-classification steps the scenario measures.
    const HARNESS_PREAMBLE: &str = "\
        CALIBRATION HARNESS — OFFLINE REVIEW EXERCISE.\n\
        You have NO tools in this turn. Do not attempt any tool call, and do not \
        report the absence of tools as a finding. The PR under review is supplied \
        verbatim in the user message: its metadata, its acceptance criteria, its \
        changed-file list and its diff summary. Treat Step 1 (qa_pr_view), Step 2/2E \
        (pipeline guards) and Step 3a (pr diff) as ALREADY PERFORMED AND GREEN, and \
        treat DEPTH as code-level. Do not post to GitHub (Step 5); emit the verdict \
        body as your reply text instead. Perform Step 2.5 (AC classification and the \
        implicit ACs) and Step 3b against the supplied diff, then emit the verdict \
        body exactly as the prompt below specifies, starting with the VERDICT line.\n\n\
        ── The mika-qa review prompt follows verbatim. ──\n\n";

    let system = format!("{HARNESS_PREAMBLE}{QA_REVIEW_PROMPT}");

    let ask = |fixture: &str| LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(system.clone()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(fixture.to_string()),
        }],
        tools: None,
        // The production prompt asks for a full verdict body (VERDICT/DEPTH/REASON +
        // NEGATIVE-TEST + DIFF ANALYSIS + PLAN-AC VERIFICATION). The 2000-token budget
        // the shorter scenarios use is not enough here: mika-qa's model is a reasoning
        // model, and a first run at 2000 spent the whole budget on `reasoning_content`,
        // returning empty text. mika-qa itself runs at 16384 (`~/.mika/agents/mika-qa/
        // config.toml`); 12000 leaves room for reasoning plus the body.
        max_tokens: 12000,
        thinking: None,
    };

    // --- Positive control: in perimeter, positive-only tests → must not pass. ---
    let positive = match provider.send_message(&ask(in_perimeter)).await {
        Ok(r) => r,
        Err(e) => return llm_error_result(ID, e, start.elapsed().as_millis() as u64),
    };
    let positive_text = positive.text().to_lowercase();

    // --- Negative control: same diff, out of perimeter → must not block on 2.5.4b. ---
    let negative = match provider.send_message(&ask(out_of_perimeter)).await {
        Ok(r) => r,
        Err(e) => return llm_error_result(ID, e, start.elapsed().as_millis() as u64),
    };
    let negative_text = negative.text().to_lowercase();

    // Optional transcript dump — the calibration's own evidence surface. Without the
    // raw verdicts a red/green run reports only PASS/FAIL, which is a claim about the
    // run rather than evidence from it. Set MIKA_CALIBRATION_DUMP_DIR to capture them.
    if let Ok(dir) = std::env::var("MIKA_CALIBRATION_DUMP_DIR") {
        let dir = std::path::PathBuf::from(dir);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("Warning: could not create dump dir {}: {e}", dir.display());
        } else {
            for (name, resp) in [("in-perimeter", &positive), ("out-of-perimeter", &negative)] {
                let mut body = resp.text();
                // A reasoning model can spend its whole budget on `reasoning_content`
                // and return empty text. Recording that separately is what tells an
                // empty verdict (budget exhausted) apart from a refused one.
                if let Some(reasoning) = resp.reasoning() {
                    body.push_str("\n\n<!-- reasoning_content (not part of the verdict) -->\n");
                    body.push_str(reasoning);
                }
                let path = dir.join(format!("{ID}.{name}.verdict.md"));
                if let Err(e) = std::fs::write(&path, &body) {
                    eprintln!("Warning: could not write {}: {e}", path.display());
                }
            }
        }
    }

    let latency = start.elapsed().as_millis() as u64;
    let input_tokens = positive.usage.input_tokens + negative.usage.input_tokens;
    let output_tokens = positive.usage.output_tokens + negative.usage.output_tokens;
    let fail = |class: FailureClass, detail: String| {
        RoleScenarioResult::fail(
            ID,
            class,
            detail,
            Some(input_tokens),
            Some(output_tokens),
            latency,
        )
    };

    if positive_text.trim().is_empty() || negative_text.trim().is_empty() {
        return fail(
            FailureClass::EmptyResponse,
            "Empty response on one of the two controls".to_string(),
        );
    }

    if !positive_text.contains("verdict:") || !negative_text.contains("verdict:") {
        return fail(
            FailureClass::ContractViolation,
            "One of the two controls produced no VERDICT: line".to_string(),
        );
    }

    // Positive control — the gate must bite.
    if positive_text.contains("verdict: pass") {
        return fail(
            FailureClass::ContractViolation,
            "In-perimeter PR with positive-only tests received `VERDICT: pass` — 2.5.4b not applied"
                .to_string(),
        );
    }

    // ...and it must name the invariant, not merely block. A verdict that blocks
    // without naming what is at risk is the vacuous form the rule exists to prevent.
    let names_invariant = [
        "mergedby",
        "reviewer",
        "self-merge",
        "merge identity",
        "actor",
    ]
    .iter()
    .any(|needle| positive_text.contains(needle));
    if !names_invariant {
        return fail(
            FailureClass::ContractViolation,
            "In-perimeter verdict blocked without naming the at-risk invariant (expected the \
             merge-identity invariant in the PR's own symbols)"
                .to_string(),
        );
    }

    // Negative control — out of perimeter, 2.5.4b must not be the blocking REASON.
    //
    // Mentioning 2.5.4b is not the leak: the prompt *requires* a `NEGATIVE-TEST:` line
    // on every verdict, and the out-of-perimeter form of that line is
    // `NEGATIVE-TEST: n/a — PR out of perimeter`. An assertion keyed on the mere
    // string "2.5.4b" would therefore fire on correct behavior. The leak is the rule
    // *biting* out of perimeter: a `missing` disposition, or a `block[ac]` whose
    // stated reason is the absent negative test.
    if negative_text.contains("negative-test: missing") {
        return fail(
            FailureClass::ContractViolation,
            "Out-of-perimeter PR received `NEGATIVE-TEST: missing` — the rule leaked past \
             its perimeter (expected `n/a — PR out of perimeter`)"
                .to_string(),
        );
    }
    if negative_text.contains("verdict: block[ac]")
        && (negative_text.contains("negative assertion")
            || negative_text.contains("negative test")
            || negative_text.contains("negative-test"))
        && !negative_text.contains("negative-test: n/a")
    {
        return fail(
            FailureClass::ContractViolation,
            "Out-of-perimeter PR was blocked on negative-test grounds — the rule leaked past \
             its perimeter"
                .to_string(),
        );
    }

    RoleScenarioResult::pass(ID, input_tokens, output_tokens, latency)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_count_is_eight() {
        assert_eq!(SCENARIOS.len(), 8);
    }

    #[test]
    fn all_scenarios_have_unique_ids() {
        let mut ids: Vec<&str> = SCENARIOS.iter().map(|s| s.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), SCENARIOS.len(), "Duplicate scenario IDs found");
    }

    #[test]
    fn all_scenarios_have_tags() {
        for scenario in SCENARIOS {
            assert!(
                !scenario.tags.is_empty(),
                "Scenario '{}' has no tags",
                scenario.id
            );
        }
    }

    #[test]
    fn fixture_verdict_format_precision_contains_ac_list() {
        let fixture = include_str!(
            "../../../tests/eval/calibration_fixtures/mika-qa/verdict_format_precision.md"
        );
        assert!(
            fixture.contains("AC1") && fixture.contains("AC2") && fixture.contains("AC3"),
            "verdict_format_precision fixture must list ACs"
        );
    }

    #[test]
    fn fixture_wip_rescue_contains_wip_prefix() {
        let fixture =
            include_str!("../../../tests/eval/calibration_fixtures/mika-qa/wip_rescue_skip.md");
        assert!(
            fixture.contains("wip("),
            "wip_rescue_skip fixture must contain wip( commit prefix"
        );
    }

    #[test]
    fn fixture_wip_rescue_contains_draft_marker() {
        let fixture =
            include_str!("../../../tests/eval/calibration_fixtures/mika-qa/wip_rescue_skip.md");
        assert!(
            fixture.contains("rescue-pipeline-verified: no"),
            "wip_rescue_skip fixture must contain rescue-pipeline-verified: no marker"
        );
    }

    #[test]
    fn fixture_duplicate_claim_grounded_has_divergent_file_sets() {
        let fixture = include_str!(
            "../../../tests/eval/calibration_fixtures/mika-qa/duplicate_claim_grounded.md"
        );
        // The fixture must present BOTH PRs' file lists so the model can compare
        // — and the lists must diverge (the founding-incident shape).
        assert!(
            fixture.contains("mika#1638"),
            "duplicate_claim_grounded fixture must reference the lookalike PR mika#1638"
        );
        assert!(
            fixture.contains("calibration_fixtures/mika-qa/manifest.yaml")
                && fixture.contains("skills/bundled/_shared/dispatch-lib.sh"),
            "fixture must list divergent file sets for both PRs to enable comparison"
        );
    }
}
