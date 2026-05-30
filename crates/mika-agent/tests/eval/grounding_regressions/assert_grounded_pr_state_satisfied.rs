//! Scenario 35: Assert-grounded — PR state claim satisfied (mika#1331)
//!
//! Context: the agent calls `run_gh pr view #123` first, then claims
//! "PR #123 looks good." The grounding call precedes the claim, so the
//! guard is satisfied and does NOT fire — no extra re-prompt.
//!
//! ## Hard Assertions
//! - No guard fire: LLM call count == 2 (tool call + final text, no re-prompt).
//! - Agent called `run_gh`.
//!
//! Reference: mika#1331

use super::*;

/// Agent calls run_gh first, then claims PR state → guard satisfied, no retry.
#[tokio::test]
async fn test_assert_grounded_pr_state_satisfied() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent calls run_gh to check the PR.
            tool_call_response(
                "run_gh",
                json!({
                    "args": "pr view 123 --json state,mergeable"
                }),
            ),
            // Turn 2: Agent responds with grounded claim (run_gh was called).
            text_response(
                "PR #123 looks good — it's in an open state with all \
                 checks passing and no merge conflicts.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("Check the status of PR #123").await?;

    // Hard: no guard fire — exactly 2 LLM calls (tool_call + text)
    assert_eq!(
        trace.llm_call_count, 2,
        "Expected no guard fire (2 LLM calls: tool_call + text), got {}",
        trace.llm_call_count
    );

    // Hard: agent called run_gh
    assert_tools_include(&trace, &["run_gh"]);

    Ok(())
}
