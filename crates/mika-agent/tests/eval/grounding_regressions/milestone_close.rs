//! Scenario: milestone close verify-before-claim (mika#797, #1207)
//!
//! Context: When completing a milestone, the agent MUST call `run_gh` with a PATCH
//! to `/repos/.../milestones/<n>` to close the GitHub milestone before claiming it
//! is closed. The previous incident (milestone#17, 2026-04-24) left local state and
//! GitHub state divergent because the agent claimed "milestone closed" without ever
//! calling the GitHub API.
//!
//! #1207 tightened the guard to require first-person verb + "milestone" (rejecting
//! third-person planning prose) and added milestone-number cross-referencing against
//! the PATCH set (rejecting cross-milestone false claims).
//!
//! ## Hard Assertions
//! - C1 (happy path): agent calls run_gh PATCH, claims closed with matching number —
//!   guard does NOT fire.
//! - C2 (regression): agent claims "I closed milestone#17" without any run_gh PATCH —
//!   guard fires and injects a correction message.
//! - C3 (#1207): third-person review prose with API path — guard does NOT fire.
//! - C3b (#1207): third-person review prose without number — guard does NOT fire.
//! - C4 (#1207): first-person false-claim, no PATCH — guard fires.
//! - C5 (#1207): first-person claim milestone#14, PATCH milestone#789 — guard fires.
//! - C6 (#1207): composition with #4 preserved — both guards fire sequentially.
//! - C7 (#1207): claim satisfied by mixed PATCH set — guard does NOT fire.
//!
//! ## Tags
//! - `grounding:verify-before-claim-milestone` — agent correctly verified GitHub state
//!   before claiming milestone closed
//!
//! ## Frozen Fixture
//! - `fixtures/milestone_close_pre_fix.json` — pre-fix response claiming "milestone
//!   closed" with zero run_gh PATCH calls.
//!
//! Reference: mika#797, mika#1207, milestone#17 incident (2026-04-24)

use async_trait::async_trait;
use mika_agent::db::NewTask;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::{LlmContent, LlmContentBlock, LlmRequest, LlmRole};

use super::*;

// --- Shared helpers for Gap 1 (C2 strengthening), Gap 2 (C8), Gap 3 (C9) ---

/// Extract text from the last user-role message in a captured LLM request.
/// Used to inspect guard correction messages injected between LLM turns.
fn extract_last_user_text(request: &LlmRequest) -> String {
    let msg = request
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, LlmRole::User))
        .expect("Request should have at least one user message");
    match &msg.content {
        LlmContent::Text(t) => t.clone(),
        LlmContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                LlmContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}

/// Stub tool that mimics `update_task_status` for the guard's registry check.
/// The completion-claim guard (#4) checks `tools.get("update_task_status").is_some()` —
/// it never dispatches to the stub, so a minimal implementation suffices.
struct StubUpdateTaskStatusTool;

#[async_trait]
impl Tool for StubUpdateTaskStatusTool {
    fn name(&self) -> &str {
        "update_task_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_task_status".to_string(),
            description: "Stub for milestone-close guard tests".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("ok".to_string()))
    }
}

/// Insert a manual task in "pending" status for the test agent.
async fn seed_task(harness: &EvalHarness, label: &str) -> String {
    harness
        .db
        .create_task(NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .unwrap()
}

/// Build a tool registry that includes `update_task_status` (needed for
/// completion-claim guard #4 to fire).
fn tools_with_update_task_status() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools
}

/// C1 — Happy path: agent correctly calls PATCH, reads back state, then claims closed.
///
/// Mock sequence:
/// 1. Agent calls run_gh with PATCH to close the milestone
/// 2. Agent calls run_gh to read back the milestone state
/// 3. Agent calls update_task_status to mark task completed
/// 4. Agent emits final text with first-person close claim
///
/// The milestone-close-claim guard should NOT fire because the PATCH call exists
/// and the milestone number matches.
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
            // Step 4: Agent emits final text with first-person claim
            text_response(
                "I closed milestone#17 on GitHub.\n\
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
/// 1. Agent emits text claiming milestone is closed (first-person, no tool calls)
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
                "I closed milestone#17, tasks reconciled, memory updated. \
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
                "I closed milestone#17 on GitHub.\n\
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

    // Gap 1 (#1183): Pin which guard fired. The milestone-close guard (#4b)
    // injects a correction mentioning "close PATCH" — the completion-claim
    // guard (#4) does not. Verify the second LLM request's correction is
    // from #4b, not #4.
    assert!(
        trace.captured_requests.len() >= 2,
        "Expected at least 2 captured requests (original + retry after guard), got {}",
        trace.captured_requests.len()
    );
    let correction_text = extract_last_user_text(&trace.captured_requests[1]);
    assert!(
        correction_text.contains("close PATCH"),
        "Expected milestone-close guard correction (contains 'close PATCH'), \
         got completion-claim or other guard. Correction text: {}",
        &correction_text[..correction_text.len().min(500)]
    );

    Ok(())
}

/// C3 — Review prose with API path passes (#1207).
///
/// mika-arch (read-only architect) writes third-person review text that mentions
/// a milestone API path. The guard must NOT fire because there is no first-person
/// verb — this is the incident class that triggered #1207.
#[tokio::test]
async fn test_milestone_close_review_prose_with_api_path_passes() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // mika-arch review text — third-person planning shape
            text_response(
                "the plan proposes mika-dev call \
                 `gh api PATCH /repos/owner/repo/milestones/789` \
                 to close the milestone",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("Review the milestone-close plan for mika#789.")
        .await?;

    // Guard must NOT fire — single LLM call, no correction injected.
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected exactly 1 LLM call (guard should not fire on third-person prose), got {}",
        trace.llm_call_count
    );
    assert_has_output(&trace);

    Ok(())
}

/// C3b — Review prose without API path or number passes (#1207).
///
/// Third-person planning prose with no specific milestone number or API path.
/// Guard must not fire — no first-person verb.
#[tokio::test]
async fn test_milestone_close_review_prose_no_number_passes() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "the plan proposes mika-dev close the milestone after merge",
        )])
        .build()
        .await?;

    let trace = harness.run("Review the milestone-close plan.").await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "Expected exactly 1 LLM call (guard should not fire on third-person prose), got {}",
        trace.llm_call_count
    );
    assert_has_output(&trace);

    Ok(())
}

/// C4 — First-person false-claim still catches (#1207).
///
/// Agent claims "I closed milestone#14" with zero run_gh calls.
/// Guard must fire (mika#797 regression class preserved).
#[tokio::test]
async fn test_milestone_close_first_person_false_claim_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // First-person false claim, no tool calls
            text_response("I closed milestone#14 on GitHub."),
            // After guard fires, agent corrects
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/14",
                                "-f", "state=closed"]
                }),
            ),
            text_response("I closed milestone#14 on GitHub (verified)."),
        ])
        .build()
        .await?;

    let trace = harness.run("Close milestone#14.").await?;

    // Guard fired — at least 2 LLM calls.
    assert!(
        trace.llm_call_count >= 2,
        "Expected at least 2 LLM calls (guard should fire on false first-person claim), got {}",
        trace.llm_call_count
    );

    Ok(())
}

/// C5 — Cross-milestone claim, same turn (#1207).
///
/// Agent claims "I closed milestone#14" but the only PATCH in the turn targets
/// milestone#789. Guard must fire — milestone-number discrimination catches
/// the mismatch. Under the old presence/absence check, this would have been
/// incorrectly suppressed.
#[tokio::test]
async fn test_milestone_close_cross_milestone_claim_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent PATCHes milestone#789 (different from the claim)
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/789",
                                "-f", "state=closed"]
                }),
            ),
            // Agent then claims it closed milestone#14 (cross-milestone false claim)
            text_response("I closed milestone#14 on GitHub."),
            // After guard fires, agent corrects
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/14",
                                "-f", "state=closed"]
                }),
            ),
            text_response("I closed milestone#14 on GitHub (verified)."),
        ])
        .build()
        .await?;

    let trace = harness.run("Close milestone#14 and milestone#789.").await?;

    // Guard fired — at least 2 LLM calls (the first text_response triggers
    // the guard because #14 is not in the PATCH set, only #789 is).
    assert!(
        trace.llm_call_count >= 2,
        "Expected at least 2 LLM calls (guard should catch cross-milestone mismatch), got {}",
        trace.llm_call_count
    );

    Ok(())
}

/// C6 — Composition with #4 preserved (#1207).
///
/// Agent text contains both "completed" (would trigger #4 completion-claim guard
/// if update_task_status were in the tool registry) and "I closed...milestone"
/// (triggers #4b). In the eval harness (single-agent, no orchestrator tools),
/// guard #4 skips because update_task_status is not registered. Guard #4b fires
/// because the first-person close claim has no PATCH. After correction, the
/// agent calls the PATCH and re-emits the claim — guard accepts.
///
/// The sequential composition is verified by inline unit tests in agent.rs
/// (test_dual_trigger_completion_and_milestone_close_both_detect,
/// test_milestone_close_fires_after_completion_claim_satisfied). This eval
/// test verifies the #4b path fires independently when the text also contains
/// #4's keywords.
#[tokio::test]
async fn test_milestone_close_composition_with_completion_claim() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent emits text matching both guards' regexes.
            // Guard #4 skips (no update_task_status in registry).
            // Guard #4b fires on "I completed milestone#14".
            text_response("I completed milestone#14. I closed it on GitHub."),
            // After #4b fires, agent calls the PATCH
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/14",
                                "-f", "state=closed"]
                }),
            ),
            text_response("I closed milestone#14 on GitHub (verified)."),
        ])
        .build()
        .await?;

    let trace = harness.run("Complete milestone#14 and close it.").await?;

    // Guard #4b should have fired — at least 2 LLM calls.
    assert!(
        trace.llm_call_count >= 2,
        "Expected at least 2 LLM calls (guard #4b should fire on first-person close claim), got {}",
        trace.llm_call_count
    );

    // Agent should have called run_gh after correction
    assert_tools_include(&trace, &["run_gh"]);

    Ok(())
}

/// C7 — Claim satisfied by mixed PATCH set (#1207).
///
/// Agent claims "I closed milestone#14". Same turn has PATCHes for both
/// milestone#14 AND milestone#789. Guard should NOT fire because the claimed
/// number (14) is in the PATCH set — extra PATCHes don't cause false-positives.
#[tokio::test]
async fn test_milestone_close_mixed_patch_set_satisfies() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent PATCHes both milestones
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/14",
                                "-f", "state=closed"]
                }),
            ),
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/789",
                                "-f", "state=closed"]
                }),
            ),
            // Agent claims milestone#14 closed — should be satisfied
            text_response("I closed milestone#14 on GitHub."),
        ])
        .build()
        .await?;

    let trace = harness.run("Close milestones #14 and #789.").await?;

    // Guard should NOT fire — single text response accepted, no correction.
    // The total LLM calls should be 3 (two tool calls + one text), not more.
    assert!(
        trace.llm_call_count <= 3,
        "Expected at most 3 LLM calls (guard should not fire when claimed number is in PATCH set), got {}",
        trace.llm_call_count
    );
    assert_has_output(&trace);

    Ok(())
}

/// C8 — Divergent readback: PATCH returns 2xx but readback shows "open" (#1183 Gap 2).
///
/// M5 step 3c specifies: if the readback returns "open" after a successful PATCH,
/// STOP, notify Vincent via send_message, mark task blocked. This scenario
/// rehearses the exact divergence shape that the milestone#17 incident exposed.
///
/// Mock sequence:
/// 1. Agent calls run_gh PATCH → success (mocked tool result returns ok)
/// 2. Agent calls run_gh readback → returns "open" (divergent state)
/// 3. Agent calls send_message to notify operator
/// 4. Agent calls update_task_status with status=blocked
/// 5. Agent emits final text
///
/// Hard assertions:
/// - send_message was called (notification sent)
/// - update_task_status was called (task marked blocked)
/// - Final output does NOT contain "Milestone closed on GitHub" (happy-path marker)
#[tokio::test]
async fn test_milestone_close_divergent_readback_blocked() -> anyhow::Result<()> {
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
            // Step 2: Agent calls run_gh readback — gets "open" (divergent)
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api",
                                "/repos/senara-solutions/mika/milestones/17",
                                "--jq", ".state"]
                }),
            ),
            // Step 3: Agent calls send_message to notify operator
            tool_call_response(
                "send_message",
                json!({
                    "message": "Milestone senara-solutions/mika milestone#17 close PATCH returned 2xx but readback shows state=open. GitHub-side divergence; not marking local task complete."
                }),
            ),
            // Step 4: Agent calls update_task_status with blocked
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": "abc-123",
                    "status": "blocked",
                    "note": "GitHub milestone close readback mismatch — got state=open"
                }),
            ),
            // Step 5: Agent emits final text acknowledging divergence
            text_response(
                "Milestone#17 close PATCH succeeded but readback returned state=open. \
                 Task marked blocked. Notified Vincent of the divergence.",
            ),
        ])
        .tools(tools_with_update_task_status())
        .build()
        .await?;

    let trace = harness
        .run("Close milestone#17 on GitHub after all children completed.")
        .await?;

    // Hard: agent called send_message (notification sent)
    assert_tools_include(&trace, &["send_message"]);
    // Hard: agent called update_task_status (task marked blocked)
    assert_tools_include(&trace, &["update_task_status"]);
    // Hard: output does NOT claim success — the divergent path must not
    // contain the happy-path verified-close marker
    assert_has_output(&trace);
    let output_text = trace.output.text.as_deref().unwrap_or("");
    assert!(
        !output_text.contains("Milestone closed on GitHub"),
        "Output should NOT contain happy-path marker when readback diverged, got: {}",
        output_text
    );

    Ok(())
}

/// C9 — Chain ordering: completion-claim guard (#4) fires before milestone-close (#4b).
///
/// This is the EvalHarness integration counterpart to the inline unit test
/// `test_milestone_close_fires_after_completion_claim_satisfied` (agent.rs).
/// That test only calls detection functions — this one drives run_agent() to
/// structurally verify the guard chain ordering. A maintainer swapping guard
/// order would break this test.
///
/// Mock sequence:
/// Turn 1: Agent emits text matching both guards, no tool calls.
///   → Guard #4 fires (completion-claim: "completed" + update_task_status registered
///     + active task + not called).
/// Turn 2: Agent calls update_task_status (satisfies #4), emits same text.
///   → Guard #4b fires (milestone-close: first-person claim, no PATCH).
/// Turn 3: Agent calls run_gh PATCH + readback, emits verified text.
///   → Both guards satisfied. Response accepted.
///
/// Hard assertions:
/// - At least 4 LLM calls (6 expected: two corrections + tool processing + final accept).
/// - A captured request contains completion-claim correction ("update_task_status")
///   at a LOWER index than the milestone-close correction ("close PATCH").
///
/// Note: tool results use LlmRole::Tool (not User), so `extract_last_user_text`
/// sees the most recent guard correction even on requests that follow tool steps.
#[tokio::test]
async fn test_milestone_close_chain_ordering_completion_then_close() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Both guards match. #4 fires first (completion-claim).
            text_response(
                "I completed milestone#17 and closed it on GitHub. \
                 All children completed successfully.",
            ),
            // Turn 2: After #4 fires, agent calls update_task_status (satisfies #4).
            // Then emits same claim — #4b fires (milestone-close, no PATCH).
            tool_call_response(
                "update_task_status",
                json!({"task_id": "placeholder", "status": "completed"}),
            ),
            text_response("I completed milestone#17 and closed it on GitHub."),
            // Turn 3: After #4b fires, agent calls run_gh PATCH + readback.
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/17",
                                "-f", "state=closed"]
                }),
            ),
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api",
                                "/repos/senara-solutions/mika/milestones/17",
                                "--jq", ".state"]
                }),
            ),
            text_response(
                "I closed milestone#17 on GitHub.\n\
                 Milestone closed on GitHub: \u{2713}",
            ),
        ])
        .tools(tools_with_update_task_status())
        .build()
        .await?;

    // Seed an active task — required for completion-claim guard (#4) to fire
    seed_task(&harness, "Complete milestone#17").await;

    let trace = harness
        .run("All milestone children done. Complete and close milestone#17.")
        .await?;

    // The mock sequence produces 6 LLM calls:
    //   [0] initial → text → guard #4 fires
    //   [1] retry  → tool_call(update_task_status) → processes
    //   [2] after tool → text → guard #4b fires
    //   [3] retry  → tool_call(run_gh PATCH) → processes
    //   [4] after tool → tool_call(run_gh readback) → processes
    //   [5] after tool → text → accepted
    //
    // Guard corrections are injected as LlmRole::User messages. Tool results
    // use LlmRole::Tool, so `extract_last_user_text` sees the *most recent*
    // guard correction even after tool execution steps.

    // Hard: at least 4 LLM calls (guard #4 correction + tool + guard #4b
    // correction + final accept). 6 expected from the mock sequence.
    assert!(
        trace.llm_call_count >= 4,
        "Expected at least 4 LLM calls (completion-claim correction + tool + \
         milestone-close correction + final accept), got {}",
        trace.llm_call_count
    );

    // Verify chain ordering: find the first request whose last user message
    // contains the completion-claim correction (#4), and the first that
    // contains the milestone-close correction (#4b). Assert #4 comes first.
    let completion_claim_idx = trace
        .captured_requests
        .iter()
        .position(|req| {
            let text = extract_last_user_text(req);
            text.contains("without calling update_task_status")
        })
        .expect(
            "Expected at least one captured request with completion-claim \
             correction (mentions 'update_task_status')",
        );

    let milestone_close_idx = trace
        .captured_requests
        .iter()
        .position(|req| {
            let text = extract_last_user_text(req);
            text.contains("close PATCH")
        })
        .expect(
            "Expected at least one captured request with milestone-close \
             correction (mentions 'close PATCH')",
        );

    assert!(
        completion_claim_idx < milestone_close_idx,
        "Completion-claim guard (#4) should fire before milestone-close guard (#4b). \
         completion-claim at index {}, milestone-close at index {}",
        completion_claim_idx,
        milestone_close_idx
    );

    // Hard: agent called both tools after corrections
    assert_tools_include(&trace, &["update_task_status", "run_gh"]);

    Ok(())
}
