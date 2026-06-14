//! Scenario: structured guard telemetry (#953)
//!
//! Verifies that fabrication-class guards fire correctly and that the agent
//! loop accepts the corrected response on the subsequent turn. The structured
//! tracing events (`guard.*`, `guard.correction_accepted`) are emitted by the
//! instrumented `warn!`/`info!` macros — this scenario exercises the guard
//! firing + correction acceptance path end-to-end via the eval harness.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count > 1 (re-prompt occurred).
//! - Corrected response is accepted: trace has output text.
//!
//! ## Tags
//! - `grounding:guard-telemetry-detection` — guard detection event fires
//! - `grounding:guard-telemetry-correction` — correction event fires on accept
//!
//! Reference: mika#953

use super::*;

/// Fabricated action claim guard fires (position 5), then corrected response
/// is accepted on the next turn. Exercises the GuardCorrelation detection →
/// correction_accepted event pair.
#[tokio::test]
async fn test_guard_telemetry_fabricated_action_claim() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent fabricates an action claim with a GitHub URL
            // but made zero tool calls. Guard 5 should fire.
            text_response(
                "I've posted a comment on https://github.com/senara-solutions/mika/issues/953 \
                 explaining the telemetry changes needed for the citation fabrication guards.",
            ),
            // Turn 2: After corrective re-prompt, agent drops the fabricated
            // claim. Guard correlation captures this as the corrected content.
            text_response(
                "I need to call run_gh to post a comment on the issue. \
                 Let me check the current issue state first.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("Post a comment on mika#953 about the telemetry changes")
        .await?;

    // Hard: guard fired — more than 1 LLM call (detection event emitted)
    assert!(
        trace.llm_call_count > 1,
        "Expected fabricated action claim guard to fire and re-prompt \
         (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: corrected response accepted (correction event emitted)
    assert_has_output(&trace);

    // Hard: final output does NOT repeat the fabricated action
    grounding_assertions::assert_response_forbids(&trace, &["posted a comment"]);

    Ok(())
}

/// Assert-grounded guard fires (position 6d), then corrected response is
/// accepted. Exercises GuardCorrelation with a different guard label.
#[tokio::test]
async fn test_guard_telemetry_assert_grounded() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent claims PR #100 is merged without calling run_gh.
            // Guard 6d should fire.
            text_response(
                "I've verified that PR #100 has been merged successfully \
                 and the changes are now on main.",
            ),
            // Turn 2: After corrective re-prompt, agent acknowledges it
            // needs to verify state.
            text_response(
                "I need to call run_gh to check the actual state of PR #100 \
                 before reporting its status.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("Check if PR #100 has been merged").await?;

    // Hard: guard fired — more than 1 LLM call (detection event emitted)
    assert!(
        trace.llm_call_count > 1,
        "Expected assert-grounded guard to fire and re-prompt \
         (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: corrected response accepted (correction event emitted)
    assert_has_output(&trace);

    Ok(())
}
