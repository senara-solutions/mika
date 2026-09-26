//! Scenario: dev-groom dispatched without verdict (happy path, mika#1133)
//!
//! Context: mika-dev receives a "groom mika#N" message. The agent emits
//! clean dispatch-acknowledgement text WITHOUT a `Verdict:` line. The
//! dev-groom fabrication guard should NOT fire because the response does
//! not claim a verdict.
//!
//! This is the expected behavior after the mika#1133 fix: the dispatcher
//! acknowledges the dispatch and the verdict arrives later via callback.
//!
//! ## Note on skill registration
//! These tests do NOT register the dev-groom skill. With the skill loaded,
//! the `required_tools` gate (#3) would fire first (before the fabrication
//! guard at position 5b), consuming mock responses. These tests verify the
//! false-positive avoidance path: clean text without Verdict passes the
//! agent loop without intervention from any guard.
//!
//! ## Hard Assertions
//! - Guard does NOT fire: LLM call count is exactly 1 (no re-prompt).
//! - Agent output contains dispatch acknowledgement text.
//! - Agent output does NOT contain `Verdict: GROOMED` or `Verdict: ESCALATE`.
//!
//! ## Tags
//! - `grounding:dev-groom-clean-dispatch` — success tag
//!   (dispatcher emits acknowledgement without fabricated verdict)
//!
//! Reference: mika#1133

use super::*;

/// Happy path: agent emits dispatch acknowledgement without Verdict line.
/// Guard should not fire — exactly 1 LLM call.
#[tokio::test]
async fn test_dev_groom_dispatched_no_verdict() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Clean dispatch response — no Verdict line.
            text_response(
                "Dispatched grooming for mika#1133. Awaiting architect verdict \
                 via callback (task: a1b2c3d4-e5f6-7890-abcd-ef1234567890).",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("groom mika issue#1133").await?;

    // Hard: guard did NOT fire — exactly 1 LLM call
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry (no Verdict in response), got {} LLM calls",
        trace.llm_call_count
    );

    // Hard: agent produced output with dispatch acknowledgement
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Dispatched");

    // Hard: no fabricated verdict in output
    grounding_assertions::assert_response_forbids(
        &trace,
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    );

    Ok(())
}

/// Status-check variant: agent answers a grooming status question without
/// Verdict line. Guard should not fire since no Verdict is claimed.
#[tokio::test]
async fn test_dev_groom_status_response_no_verdict() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "mika#920 has a plan committed on branch `bug/920/fix-langfuse-span` \
                 @ `abc123`. The ticket has not been through architect review. \
                 To groom it, I can dispatch `run_claude_pilot_groom`.",
        )])
        .build()
        .await?;

    let trace = harness.run("groom mika issue#920").await?;

    // Guard did NOT fire
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry for status response, got {} LLM calls",
        trace.llm_call_count
    );

    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(
        &trace,
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    );

    Ok(())
}
