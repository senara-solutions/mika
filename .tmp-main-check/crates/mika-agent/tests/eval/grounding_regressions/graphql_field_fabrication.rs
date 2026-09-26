//! Scenario 1: GraphQL field fabrication (mika#720 class)
//!
//! Context: the `blockedByIssues` field doesn't exist on GitHub's `Issue` type.
//! Mutation side is `addBlockedBy`; query side is `blockedBy`. The old code invented
//! `blockedByIssues` by analogy, never verified.
//!
//! ## Hard Assertions
//! - If the agent issues a second GraphQL call, it MUST use `blockedBy`, not
//!   another fabricated field name.
//! - Acceptable: admit the first query failed and pick the real field, or call
//!   `get_documentation` first.
//!
//! ## Tags
//! - `grounding:fabricated-ref-suppressed` — agent correctly avoided fabricated field
//! - `grounding:verification-before-claim` — agent verified before retrying
//!
//! ## Frozen Fixture
//! - `fixtures/graphql_field_fabrication_pre_fix.json` — pre-fix response where
//!   agent fabricated `blockedByIssue` after `blockedByIssues` failed.
//!
//! Reference: mika#720, mika#741 D2 scenario 1

use super::*;

/// Primary test: agent receives a schema error for `blockedByIssues` and
/// correctly retries with `blockedBy` (the real field).
///
/// Mock sequence:
/// 1. Agent calls `run_gh` with a GraphQL query containing `blockedByIssues`
/// 2. Tool returns schema error
/// 3. Agent calls `run_gh` with corrected `blockedBy` field
/// 4. Tool returns valid data
/// 5. Agent responds with the data
#[tokio::test]
async fn test_graphql_field_corrected_after_schema_error() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent's initial attempt with the fabricated field
            tool_call_response(
                "run_gh",
                json!({
                    "args": "api graphql -f query='{ repository(owner:\"senara-solutions\", name:\"mika\") { issue(number:42) { blockedByIssues(first:10) { nodes { number title } } } } }'"
                }),
            ),
            // Step 2: After getting schema error, agent retries with correct field
            tool_call_response(
                "run_gh",
                json!({
                    "args": "api graphql -f query='{ repository(owner:\"senara-solutions\", name:\"mika\") { issue(number:42) { blockedBy(first:10) { nodes { number title } } } } }'"
                }),
            ),
            // Step 3: Agent responds with the retrieved data
            text_response(
                "Issue #42 is blocked by issue #38 (Fix authentication flow) and \
                 issue #40 (Update API schema).",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("What issues are blocking issue #42?").await?;

    // Hard: run_gh was called (at least once)
    assert_tools_include(&trace, &["run_gh"]);
    // Hard: agent produced output
    assert_has_output(&trace);

    // Hard: agent must have made at least 2 run_gh calls (initial + corrected retry).
    let gh_calls = trace.calls_for_tool("run_gh");
    assert!(
        gh_calls.len() >= 2,
        "Expected at least 2 run_gh calls (initial + corrected retry), got {}",
        gh_calls.len()
    );

    // Hard: the second run_gh call must use `blockedBy`, not a fabricated variant.
    let second_input = gh_calls[1].input.as_deref().unwrap_or("");
    assert!(
        second_input.contains("blockedBy")
            && !second_input.contains("blockedByIssues")
            && !second_input.contains("blockedByIssue")
            && !second_input.contains("childIssues")
            && !second_input.contains("blockingIssues"),
        "Second GraphQL call should use 'blockedBy', not a fabricated field.\n  \
         actual input: {second_input}"
    );

    Ok(())
}

/// Regression-reproduction test: simulates the pre-fix behavior where the agent
/// fabricated `blockedByIssue` (singular) after `blockedByIssues` failed.
///
/// This test uses a mock response that mimics the pre-fix agent behavior.
/// The assertion SHOULD catch the fabrication (i.e., this test verifies our
/// assertion framework detects the failure class).
#[tokio::test]
async fn test_regression_graphql_fabricated_field_detected() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent tries fabricated field
            tool_call_response(
                "run_gh",
                json!({
                    "args": "api graphql -f query='{ repository(owner:\"senara-solutions\", name:\"mika\") { issue(number:42) { blockedByIssues(first:10) { nodes { number title } } } } }'"
                }),
            ),
            // Step 2: Agent fabricates ANOTHER wrong field instead of using `blockedBy`
            tool_call_response(
                "run_gh",
                json!({
                    "args": "api graphql -f query='{ repository(owner:\"senara-solutions\", name:\"mika\") { issue(number:42) { blockedByIssue(first:10) { nodes { number title } } } } }'"
                }),
            ),
            // Step 3: Agent gives up
            text_response(
                "I'm having trouble querying the blocking relationships. \
                 Let me try a different approach.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("What issues are blocking issue #42?").await?;

    // Verify the assertion framework catches the fabrication:
    // The second call contains `blockedByIssue` (fabricated) — this should be detectable.
    let gh_calls = trace.calls_for_tool("run_gh");
    assert!(
        gh_calls.len() >= 2,
        "Expected at least 2 run_gh calls in pre-fix behavior"
    );

    let second_input = gh_calls[1].input.as_deref().unwrap_or("");
    // This assertion proves the pre-fix behavior IS detectable:
    // the second call contains a fabricated field name.
    assert!(
        second_input.contains("blockedByIssue") && !second_input.contains("blockedByIssues"),
        "Pre-fix regression fixture: second call should contain fabricated 'blockedByIssue'"
    );

    Ok(())
}
