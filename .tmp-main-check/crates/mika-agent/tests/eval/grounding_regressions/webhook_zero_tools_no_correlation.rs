//! Scenario: webhook_zero_tools guard prefix-narrowing for no-correlation events (mika#1469)
//!
//! Context: three always-informational GitHub webhook event classes
//! (`Check suite success`, `PR closed`, `discussion.*`) were triggering
//! the `webhook_zero_tools` intent-precondition guard even though they
//! have no actionable response. The guard re-prompted the LLM, producing
//! 25+ documented misfires where the agent was pressured to call a tool
//! just to satisfy the precondition.
//!
//! mika#1469 narrows the trigger so these three prefix classes are skipped.
//!
//! ## Hard Assertions
//! - **Skipped prefixes:** text-only EndTurn on `Check suite success`,
//!   `PR closed:`, `discussion.` → guard does NOT fire → 1 LLM call.
//! - **Retained prefix (regression):** text-only EndTurn on `Check suite
//!   failure` → guard DOES fire → 2 LLM calls.
//!
//! Reference: mika#1469

use super::*;

/// `[GitHub] Check suite success on …` + text-only EndTurn → guard skipped.
#[tokio::test]
async fn test_check_suite_success_no_guard_fire() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("CI passed on main. No action needed.")])
        .build()
        .await?;

    let trace = harness
        .run("[GitHub] Check suite success on senara-solutions/mika (branch: main)")
        .await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "webhook_zero_tools guard should NOT fire on Check suite success \
         (expected 1 LLM call, got {})",
        trace.llm_call_count
    );

    Ok(())
}

/// `[GitHub] PR closed: …` + text-only EndTurn → guard skipped.
#[tokio::test]
async fn test_pr_closed_no_guard_fire() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Noted the PR closure event. No action needed on this one.",
        )])
        .build()
        .await?;

    let trace = harness
        .run("[GitHub] PR closed: senara-solutions/mika#999 — title (branch: foo)")
        .await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "webhook_zero_tools guard should NOT fire on PR closed \
         (expected 1 LLM call, got {})",
        trace.llm_call_count
    );

    Ok(())
}

/// `[GitHub] discussion.created on …` + text-only EndTurn → guard skipped.
#[tokio::test]
async fn test_discussion_no_guard_fire() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Discussion event noted. No action required.",
        )])
        .build()
        .await?;

    let trace = harness
        .run("[GitHub] discussion.created on senara-solutions/mika")
        .await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "webhook_zero_tools guard should NOT fire on discussion events \
         (expected 1 LLM call, got {})",
        trace.llm_call_count
    );

    Ok(())
}

/// Regression: `[GitHub] Check suite failure on …` + text-only EndTurn →
/// guard STILL fires → correction injected → 2 LLM calls.
#[tokio::test]
async fn test_check_suite_failure_guard_fires() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: text-only response (no tool calls) — guard fires.
            text_response("CI failed on fix/foo."),
            // Turn 2: after correction re-prompt, still text-only.
            // Guard has already used its single retry, so this EndTurn is accepted.
            text_response("Acknowledged the CI failure."),
        ])
        .build()
        .await?;

    let trace = harness
        .run("[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)")
        .await?;

    assert!(
        trace.llm_call_count > 1,
        "webhook_zero_tools guard should fire on Check suite failure \
         (expected >1 LLM calls, got {})",
        trace.llm_call_count
    );

    Ok(())
}
