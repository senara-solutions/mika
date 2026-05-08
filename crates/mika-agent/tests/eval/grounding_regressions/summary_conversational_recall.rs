//! Scenario: Conversational-recall regression from summary shape (mika#1024)
//!
//! The compaction summarizer's output is injected into the system prompt as
//! context for the next session. When the summary uses conversational shape
//! ("we discussed", "as we agreed"), the LLM misreads it as prior turns it
//! participated in, producing first-person conversational-recall patterns in
//! callback responses.
//!
//! The reformed (factual) shape uses state-assertion prefixes (`Fact:`,
//! `Decision:`, `Outcome:`, `Open:`) and third-person framing, which prevents
//! the LLM from treating the summary as a shared conversation.
//!
//! ## Hard Assertions
//! - Response from a callback turn with reformed-shape summary MUST NOT
//!   contain conversational-recall phrases ("we discussed", "as we agreed",
//!   "in our prior conversation", "as you mentioned", "we decided")
//! - System prompt MUST contain the reformed-shape summary content
//!
//! ## Negative Control
//! - Same callback turn setup with a conversational-shape summary (pre-reform)
//!   produces a response WITH conversational-recall phrases — proving the
//!   regression class is the summary shape, not the summary content per se
//!
//! ## Tags
//! - `grounding:conversational-recall-suppressed` — reformed summary did not
//!   trigger conversational-recall patterns
//! - `grounding:conversational-recall-triggered` (failure) — conversational
//!   summary caused the LLM to produce first-person recall
//!
//! ## Frozen Fixture
//! - `fixtures/summary_conversational_recall_pre_fix.json` — pre-reform
//!   conversational-shape summary that triggers recall patterns
//!
//! Reference: mika#1024 (Axis 2 — content reform), mika#1009

use super::*;

/// Conversational-recall phrases that indicate the LLM is treating the
/// summary as a prior conversation it participated in.
const CONVERSATIONAL_RECALL_PHRASES: &[&str] = &[
    "we discussed",
    "as we agreed",
    "in our prior conversation",
    "as you mentioned",
    "we decided",
    "you asked me",
    "we talked about",
    "our earlier discussion",
];

/// Reformed-shape (factual) summary using the #1024 prefix vocabulary.
/// This is what the post-fix summarizer produces.
const REFORMED_SUMMARY: &str = "\
- Fact: PR #890 targets the `mika-agent` crate, modifying `required_tools_gate.rs`
- Decision: transport-contract fix chosen over prompt-only mitigation
- Outcome: CI green on branch `feat/890/required-tools-retry-self-contained`; QA review requested
- Open: whether the `attempt_continuation_turn` path needs the same self-contained guard";

/// Conversational-shape summary (pre-reform). This is what the old
/// summarizer produced — uses first-person, conversational verbs.
const CONVERSATIONAL_SUMMARY: &str = "\
We discussed the required-tools retry issue in PR #890 targeting `mika-agent`. \
We agreed that a transport-contract fix was better than a prompt-only approach. \
After our discussion, we decided to push the branch and request QA review. \
You mentioned that the `attempt_continuation_turn` path might need the same guard, \
and I said we should look into it after the current fix lands.";

/// Primary test: reformed-shape summary injected into a callback turn does NOT
/// produce conversational-recall patterns in the agent response.
///
/// Setup: seed reformed summary → trigger callback turn with inject=true
/// Assertion: response contains no conversational-recall phrases
#[tokio::test]
async fn test_reformed_summary_no_conversational_recall() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .callback_turn(true)
        .responses(vec![
            // Agent processes callback result — clean factual response
            text_response(
                "The CI pipeline completed successfully for PR #890. \
                 The transport-contract fix in `required_tools_gate.rs` passes all \
                 2815 tests. The branch is ready for merge pending QA approval. \
                 The `attempt_continuation_turn` guard question remains open.",
            ),
        ])
        .build()
        .await?;

    // Seed the reformed-shape summary in DB
    harness.db.replace_with_summary(REFORMED_SUMMARY, 0).await?;

    let trace = harness
        .run("[callback: run_claude_pilot] PR #890 CI completed: all checks passed")
        .await?;

    // Hard: output exists
    assert_has_output(&trace);

    // Hard: system prompt contains the reformed summary content
    assert_system_prompt_contains(&trace, "Fact: PR #890");
    assert_system_prompt_contains(&trace, "Decision: transport-contract fix");
    assert_system_prompt_contains(&trace, "Outcome: CI green");

    // Hard: response does NOT contain conversational-recall phrases
    for phrase in CONVERSATIONAL_RECALL_PHRASES {
        let lower_response = trace.output.text.as_deref().unwrap_or("").to_lowercase();
        assert!(
            !lower_response.contains(&phrase.to_lowercase()),
            "Reformed-shape summary should not trigger conversational recall: \
             found '{}' in response: {:?}",
            phrase,
            &trace.output.text.as_deref().unwrap_or("")
                [..100.min(trace.output.text.as_deref().unwrap_or("").len())]
        );
    }

    Ok(())
}

/// Negative control: conversational-shape summary (pre-reform) fed to the same
/// callback turn setup produces a response WITH conversational-recall patterns.
///
/// This proves the regression class is the conversational shape of the summary,
/// not the summary content per se. The frozen fixture response mirrors what a
/// real LLM produces when given a conversational-shape summary as context.
#[tokio::test]
async fn test_conversational_summary_triggers_recall_regression() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .callback_turn(true)
        .responses(vec![
            // Pre-reform behavior: agent echoes conversational framing from summary
            text_response(
                "As we discussed regarding PR #890, the transport-contract fix was \
                 the right call. In our prior conversation, we agreed on the approach \
                 and now CI confirms it works. As you mentioned, the \
                 `attempt_continuation_turn` path is still an open question we decided \
                 to defer.",
            ),
        ])
        .build()
        .await?;

    // Seed the conversational-shape summary (pre-reform)
    harness
        .db
        .replace_with_summary(CONVERSATIONAL_SUMMARY, 0)
        .await?;

    let trace = harness
        .run("[callback: run_claude_pilot] PR #890 CI completed: all checks passed")
        .await?;

    // Verify the assertion framework catches the conversational-recall regression:
    // at least one recall phrase must be present in the pre-reform response.
    let response_lower = trace.output.text.as_deref().unwrap_or("").to_lowercase();
    let has_recall = CONVERSATIONAL_RECALL_PHRASES
        .iter()
        .any(|phrase| response_lower.contains(&phrase.to_lowercase()));

    assert!(
        has_recall,
        "Negative control failed: the pre-reform conversational-shape response should \
         contain at least one conversational-recall phrase from {:?}, but none were found \
         in: {:?}",
        CONVERSATIONAL_RECALL_PHRASES,
        &trace.output.text.as_deref().unwrap_or("")
            [..200.min(trace.output.text.as_deref().unwrap_or("").len())]
    );

    // Also verify the system prompt contains the conversational-shape summary
    assert_system_prompt_contains(&trace, "We discussed");

    Ok(())
}
