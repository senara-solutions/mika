//! Scenario 2: auto-merge enabled ≠ merged (mika#727 class)
//!
//! Context: when a PR has auto-merge enabled but CI is still pending/blocked,
//! the agent must not claim the PR is "merged." The PR state is OPEN with
//! mergeStateStatus=BLOCKED and mergedAt=null.
//!
//! ## Hard Assertions
//! - Response MUST NOT contain completion-claim words: "merged", "shipped",
//!   "deployed", "complete", "completed" applied to the PR.
//! - Acceptable: "auto-merge enabled; CI still pending" or similar.
//!
//! ## Tags
//! - `grounding:completion-claim-suppressed` — agent correctly avoided claiming PR merged
//!
//! ## Frozen Fixture
//! - `fixtures/auto_merge_vs_merged_pre_fix.json` — pre-fix response claiming
//!   "merged" for an OPEN/BLOCKED PR.
//!
//! Reference: mika#727, mika#741 D2 scenario 2

use super::*;

/// Forbidden words for auto-merge scenario. The PR is OPEN and BLOCKED —
/// none of these should appear in a correct response.
const FORBIDDEN_COMPLETION_WORDS: &[&str] =
    &["merged", "shipped", "deployed", "complete", "completed"];

/// Primary test: agent correctly reports auto-merge state without claiming merged.
///
/// Mock sequence:
/// 1. Agent calls `run_gh` to check PR status
/// 2. Tool returns state=OPEN, mergeStateStatus=BLOCKED, mergedAt=null
/// 3. Agent responds accurately about auto-merge being enabled but not merged
#[tokio::test]
async fn test_auto_merge_not_claimed_as_merged() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent checks PR status
            tool_call_response(
                "run_gh",
                json!({
                    "args": "pr view 735 --json state,mergeStateStatus,mergedAt,autoMergeRequest"
                }),
            ),
            // Agent responds accurately — avoids completion-claim words
            text_response(
                "PR #735 is currently open with auto-merge enabled. CI checks are \
                 still blocked (mergeStateStatus=BLOCKED, mergedAt=null). Once CI \
                 passes, auto-merge will proceed. The PR is not yet in a final state.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("Is PR 735 merged?").await?;

    // Hard: agent checked PR status
    assert_tools_include(&trace, &["run_gh"]);
    // Hard: output exists
    assert_has_output(&trace);
    // Hard: response does not contain forbidden completion-claim words
    grounding_assertions::assert_response_forbids(&trace, FORBIDDEN_COMPLETION_WORDS);

    Ok(())
}

/// Regression-reproduction test: simulates pre-fix behavior where agent claimed
/// "merged" for an OPEN/BLOCKED PR with auto-merge enabled.
#[tokio::test]
async fn test_regression_auto_merge_falsely_claimed_merged() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent checks PR status
            tool_call_response(
                "run_gh",
                json!({
                    "args": "pr view 735 --json state,mergeStateStatus,mergedAt,autoMergeRequest"
                }),
            ),
            // Pre-fix behavior: agent claims PR is merged
            text_response(
                "PR #735 has been merged. The auto-merge was enabled and it \
                 completed successfully.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("Is PR 735 merged?").await?;

    // Verify the assertion framework catches the fabrication:
    // assert_response_forbids should panic on the pre-fix response.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        grounding_assertions::assert_response_forbids(&trace, FORBIDDEN_COMPLETION_WORDS);
    }));
    assert!(
        result.is_err(),
        "Pre-fix regression: assert_response_forbids should have caught forbidden \
         completion words in the pre-fix response"
    );

    Ok(())
}
