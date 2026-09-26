//! Scenario 34: Assert-grounded — PR state claim caught (mika#1331)
//!
//! Context: the agent claims "I reviewed PR #123, no issues" with no `run_gh`
//! call in the turn. The assert-grounded EndTurn guard detects the affirmative
//! state claim, rejects the turn, and re-prompts the agent to verify.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count > 1 (re-prompt occurred).
//! - Agent calls `run_gh` on the retry turn (corrective behavior).
//!
//! Reference: mika#1331

use super::*;

/// Agent claims PR #123 state without grounding → guard fires → agent corrects.
#[tokio::test]
async fn test_assert_grounded_pr_state_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent claims PR state without calling run_gh.
            // The assert-grounded guard should reject this EndTurn.
            text_response(
                "I reviewed PR #123 and it looks good — no issues found. \
                 The changes are clean and ready to merge.",
            ),
            // Turn 2 (after corrective re-prompt): Agent calls run_gh to verify.
            tool_call_response(
                "run_gh",
                json!({
                    "args": "pr view 123 --json state,mergeable"
                }),
            ),
            // Turn 3: Agent responds with grounded information.
            text_response(
                "After checking PR #123 via `run_gh`, I can confirm the PR \
                 is in an open state and passes all checks.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("Check the status of PR #123").await?;

    // Hard: guard fired — more than 1 LLM call
    assert!(
        trace.llm_call_count > 1,
        "Expected guard to fire and re-prompt (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent called run_gh after corrective re-prompt
    assert_tools_include(&trace, &["run_gh"]);

    Ok(())
}
