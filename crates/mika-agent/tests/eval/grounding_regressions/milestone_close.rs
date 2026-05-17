//! Scenario: milestone close verify-before-claim (mika#797)
//!
//! Context: When completing a milestone, the agent MUST call `run_gh` with a PATCH
//! to `/repos/.../milestones/<n>` to close the GitHub milestone before claiming it
//! is closed. The previous incident (milestone#17, 2026-04-24) left local state and
//! GitHub state divergent because the agent claimed "milestone closed" without ever
//! calling the GitHub API.
//!
//! ## Hard Assertions
//! - C1 (happy path): agent calls run_gh PATCH, calls readback, response contains
//!   "Milestone closed on GitHub" — guard does NOT fire.
//! - C2 (regression): agent claims "milestone closed" without any run_gh PATCH call —
//!   guard fires and injects a correction message.
//!
//! ## Tags
//! - `grounding:verify-before-claim-milestone` — agent correctly verified GitHub state
//!   before claiming milestone closed
//!
//! ## Frozen Fixture
//! - `fixtures/milestone_close_pre_fix.json` — pre-fix response claiming "milestone
//!   closed" with zero run_gh PATCH calls.
//!
//! Reference: mika#797, milestone#17 incident (2026-04-24)

use super::*;

/// C1 — Happy path: agent correctly calls PATCH, reads back state, then claims closed.
///
/// Mock sequence:
/// 1. Agent calls run_gh with PATCH to close the milestone
/// 2. Agent calls run_gh to read back the milestone state
/// 3. Agent calls update_task_status to mark task completed
/// 4. Agent emits final text with "Milestone closed on GitHub: ✓"
///
/// The milestone-close-claim guard should NOT fire because the PATCH call exists.
#[tokio::test]
async fn test_milestone_close_happy_path() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls run_gh PATCH to close milestone
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/17",
                                "-f", "state=closed"]
                }),
            ),
            // Step 2: Agent calls run_gh readback
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api",
                                "/repos/senara-solutions/mika/milestones/17",
                                "--jq", ".state"]
                }),
            ),
            // Step 3: Agent calls update_task_status
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": "abc-123",
                    "status": "completed"
                }),
            ),
            // Step 4: Agent emits final text
            text_response(
                "Milestone senara-solutions/mika milestone#17 complete.\n\
                 Milestone closed on GitHub: \u{2713}\n\
                 \u{2705} Completed: 5 | \u{274c} Failed: 0 | \u{23f8}\u{fe0f} Blocked: 0\n\
                 Total cost: $2.34 | Total turns: 47\n\
                 Build + deploy: done.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("All milestone children are processed. Complete the milestone.")
        .await?;

    // Hard: agent called run_gh (PATCH + readback)
    assert_tools_include(&trace, &["run_gh"]);
    // Hard: agent called update_task_status
    assert_tools_include(&trace, &["update_task_status"]);
    // Hard: output exists and contains the verified-close marker
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Milestone closed on GitHub");

    Ok(())
}

/// C2 — Regression reproduction: pre-fix behavior where agent claimed "milestone
/// closed" without any run_gh PATCH call. The guard fires and injects a correction.
///
/// Mock sequence:
/// 1. Agent emits text claiming milestone is closed (no tool calls)
/// 2. Guard fires — correction message injected
/// 3. Agent responds with corrected behavior (calls the PATCH)
///
/// Verifies that the `detect_milestone_close_claim_without_patch` guard catches
/// the pre-fix failure class.
#[tokio::test]
async fn test_milestone_close_regression_no_patch() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Pre-fix behavior: agent claims milestone closed without PATCH
            text_response(
                "Milestone#17 closed, tasks reconciled, memory updated. \
                 All children completed successfully.\n\n\
                 \u{2705} Completed: 5 | \u{274c} Failed: 0 | \u{23f8}\u{fe0f} Blocked: 0\n\
                 Total cost: $2.34 | Total turns: 47\n\
                 Build + deploy: done.",
            ),
            // After guard fires and correction is injected, agent corrects:
            // calls the PATCH
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/17",
                                "-f", "state=closed"]
                }),
            ),
            // Readback
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api",
                                "/repos/senara-solutions/mika/milestones/17",
                                "--jq", ".state"]
                }),
            ),
            // Now agent emits corrected text
            text_response(
                "Milestone senara-solutions/mika milestone#17 complete.\n\
                 Milestone closed on GitHub: \u{2713}\n\
                 \u{2705} Completed: 5",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("All milestone children are processed. Complete the milestone.")
        .await?;

    // The guard should have fired — verify the correction happened by checking
    // that 2+ LLM calls were made (original + retry after correction).
    assert!(
        trace.llm_call_count >= 2,
        "Expected at least 2 LLM calls (original + retry after guard correction), got {}",
        trace.llm_call_count
    );

    // After correction, the agent should have called run_gh
    assert_tools_include(&trace, &["run_gh"]);

    // Final output should contain the verified-close marker
    assert_has_output(&trace);

    Ok(())
}
