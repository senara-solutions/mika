//! Scenario: Required-tools-gate retry produces thin final turn (mika#890 class)
//!
//! Context: a skill declares `[constraints] required_tools = ["gh_read"]`. The agent
//! emits a substantive review on step 0 (EndTurn) without calling `gh_read`. The
//! required-tools gate rejects it and injects a correction message. The agent calls
//! `gh_read`, produces tool-use turns with the real review content, then emits a thin
//! pointer-summary on the final EndTurn ("Disposition stands: ITERATE with the revised
//! findings above"). Only the final EndTurn is persisted to `messages` — the
//! substantive content is lost.
//!
//! ## Hard Assertions
//! - **Regression-reproduction:** final turn does NOT contain `[KEY-FINDING-1]` and
//!   `[KEY-FINDING-2]` markers (proves the assertion has teeth on the pre-fix shape).
//! - **Post-fix happy path:** final turn DOES contain both markers in order (proves
//!   the self-contained-response contract holds).
//! - **Correction message contains self-contained instruction:** the User correction
//!   injected by the required-tools gate includes the new persistence-awareness text.
//!
//! ## Tags
//! - `grounding:transport-contract-thin-final-turn` — pre-fix failure tag
//!   (substantive content lost because final turn is a pointer-summary)
//! - `grounding:transport-contract-self-contained` — post-fix success tag
//!   (final turn restates the full content after retry)
//!
//! ## Frozen Fixture
//! - `fixtures/required_tools_retry_thin_final_turn_pre_fix.json` — hand-crafted
//!   pre-fix trace shape with `[KEY-FINDING-1]` and `[KEY-FINDING-2]` markers in the
//!   rejected step 0 content. Final EndTurn (step 4) is a thin pointer-summary without
//!   the markers.
//!
//! Reference: mika#890, mika#270 (required-tools gate origin)

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_common::llm::mock::*;

use super::*;

/// Create a skill entry with `required_tools` configured,
/// mimicking a mika-arch skill that requires `gh_read`.
fn make_required_tools_skill(name: &str, keywords: &[&str], required_tools: &[&str]) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} test skill"),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
            },
            triggers: Triggers {
                keywords: keywords.iter().map(|s| s.to_string()).collect(),
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: required_tools.iter().map(|s| s.to_string()).collect(),
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from(format!("/skills/{name}")),
        keywords_lower: keywords.iter().map(|s| s.to_lowercase()).collect(),
        prompt_snippet: String::new(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// Regression-reproduction test: simulates the pre-fix mika#890 trace shape.
///
/// The agent emits substantive content with `[KEY-FINDING-1]` and `[KEY-FINDING-2]`
/// markers on step 0 (rejected by required-tools gate), calls `search_memory`
/// (the required tool), then emits a thin pointer-summary on the final EndTurn
/// that does NOT contain the markers.
///
/// This test proves the marker-based assertion catches the failure class.
#[tokio::test]
async fn test_regression_required_tools_retry_thin_final_turn() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_required_tools_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        &["search_memory"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 0: Substantive review with markers — but no required tool call.
            // Required-tools gate rejects this EndTurn.
            text_response(
                "[KEY-FINDING-1] Concern: the key shape in the issue body does not \
                 match the schema defined in architecture.md section 4.\n\n\
                 [KEY-FINDING-2] Concern: record-at-dispatch failure mode — if the \
                 dispatch call fails after task status is set to in_progress, there \
                 is no rollback path.\n\n\
                 Disposition: ITERATE",
            ),
            // Step 1: After correction, agent calls the required tool.
            tool_call_response(
                "search_memory",
                json!({
                    "query": "issue 886 key shape"
                }),
            ),
            // Step 2: Thin pointer-summary — the pre-fix failure shape.
            // This is the final EndTurn that gets persisted.
            text_response(
                "Disposition stands: ITERATE with the revised findings above. \
                 The prior turn's review was issued without ground-truth \
                 verification — required-tools gate correctly forced the \
                 correction. Two structural deviations require resolution \
                 before second pass.",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness
        .run("groom-ticket: Review the implementation plan for mika#886.")
        .await?;

    // Guard fired — more than 1 LLM call
    assert!(
        trace.llm_call_count > 1,
        "Expected required-tools gate to fire (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // The final output is the thin pointer-summary — it does NOT contain the markers.
    // This proves the assertion has teeth: the pre-fix shape fails the marker check.
    let output_text = trace.output.text.as_deref().unwrap_or("");
    assert!(
        !output_text.contains("[KEY-FINDING-1]"),
        "Pre-fix regression: final turn should NOT contain [KEY-FINDING-1], \
         but it does — the thin-final-turn failure is not reproducing"
    );
    assert!(
        !output_text.contains("[KEY-FINDING-2]"),
        "Pre-fix regression: final turn should NOT contain [KEY-FINDING-2], \
         but it does — the thin-final-turn failure is not reproducing"
    );

    Ok(())
}

/// Post-fix happy path: after the engine fix (Unit 1), the correction message
/// instructs the LLM to restate the full content. The final EndTurn contains
/// both `[KEY-FINDING-1]` and `[KEY-FINDING-2]` markers in order.
#[tokio::test]
async fn test_required_tools_retry_self_contained_final_turn() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_required_tools_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        &["search_memory"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 0: Substantive review with markers — no required tool call.
            // Required-tools gate rejects this EndTurn.
            text_response(
                "[KEY-FINDING-1] Concern: the key shape in the issue body does not \
                 match the schema defined in architecture.md section 4.\n\n\
                 [KEY-FINDING-2] Concern: record-at-dispatch failure mode — if the \
                 dispatch call fails after task status is set to in_progress, there \
                 is no rollback path.\n\n\
                 Disposition: ITERATE",
            ),
            // Step 1: After correction, agent calls the required tool.
            tool_call_response(
                "search_memory",
                json!({
                    "query": "issue 886 key shape"
                }),
            ),
            // Step 2: Self-contained final turn — restates both markers.
            // This is the post-fix behavior: the correction message told the LLM
            // to restate the full content.
            text_response(
                "[KEY-FINDING-1] Concern: the key shape in the issue body does not \
                 match the schema defined in architecture.md section 4 — confirmed \
                 by ground-truth verification via search_memory.\n\n\
                 [KEY-FINDING-2] Concern: record-at-dispatch failure mode — if the \
                 dispatch call fails after task status is set to in_progress, there \
                 is no rollback path. Confirmed by issue body review.\n\n\
                 Disposition: ITERATE\n\n\
                 Two structural deviations require resolution before second pass.",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness
        .run("groom-ticket: Review the implementation plan for mika#886.")
        .await?;

    // Guard fired — more than 1 LLM call
    assert!(
        trace.llm_call_count > 1,
        "Expected required-tools gate to fire (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Post-fix: the final turn contains both markers in order
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains_in_order(
        &trace,
        &["[KEY-FINDING-1]", "[KEY-FINDING-2]"],
    );

    // The required tool was called
    grounding_assertions::assert_tool_called_before_response(&trace, "search_memory");

    Ok(())
}

/// Verify that the correction message injected by the required-tools gate
/// contains the new self-contained-response instruction.
#[tokio::test]
async fn test_correction_message_contains_self_contained_instruction() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_required_tools_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        &["search_memory"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 0: No required tool call — gate fires.
            text_response("Review without calling required tools."),
            // Step 1: Calls the required tool.
            tool_call_response("search_memory", json!({ "query": "test" })),
            // Step 2: Final response.
            text_response("Final review after correction."),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("groom-ticket: Review the plan.").await?;

    // The correction message should be in captured_requests.
    // After the gate rejection, the second LLM request should contain the
    // User correction message with the self-contained instruction.
    assert!(
        trace.captured_requests.len() >= 2,
        "Expected at least 2 captured requests (original + post-correction), got {}",
        trace.captured_requests.len()
    );

    // The second request's messages should contain the User correction
    let second_request = &trace.captured_requests[1];
    // mika#1168 reshape (Phase A Step 3) updated the correction text to
    // use `[mika-engine]` trusted-marker framing + dropped the "You MUST"
    // mandate phrasing. The self-contained-response invariant is
    // preserved in different words: "only the final response reaches the
    // conversation log" semantically equals the prior "Only the final
    // response is persisted to the conversation log." Both shapes also
    // include "restate the full content" and "in-memory loop context."
    let has_self_contained_instruction = second_request.messages.iter().any(|msg| {
        let text = match &msg.content {
            mika_common::llm::LlmContent::Text(t) => t.as_str(),
            _ => "",
        };
        let lowered = text.to_lowercase();
        lowered.contains("restate the full content")
            && lowered.contains("only the final response")
            && lowered.contains("conversation log")
            && lowered.contains("in-memory loop context")
    });

    assert!(
        has_self_contained_instruction,
        "Expected the correction message to contain the self-contained-response \
         instruction ('restate the full content' + 'only the final response' + \
         'conversation log' + 'in-memory loop context'), but it was not found in \
         the second request's messages"
    );

    Ok(())
}
