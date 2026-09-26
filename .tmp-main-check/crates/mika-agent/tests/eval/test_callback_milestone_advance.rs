//! Integration tests: callback milestone advance guard (#991) and HOLD
//! re-entry semantics (#1208).
//!
//! Verifies that the inline guard `callback_milestone_advance` requires
//! milestone/project-context callback turns to either:
//! - Path A: call `run_claude_pilot` (advance to next child), OR
//! - Path B: call `update_task_status` with the parent task ID and
//!   status `blocked`/`completed` (halt or finish the milestone).
//!
//! The guard triggers on `[callback:` + `[milestone-parent: <id>]` markers
//! in the user message. In production, the milestone-parent marker is
//! injected by `run_silent_agent` after a DB lookup of the parent task type.
//! In these tests, we include it directly in the user message.
//!
//! Uses conversation mode (not `.callback_turn(true)`) for the same reason
//! as `test_callback_terminal_action.rs` — see that file's doc comment.
//!
//! Test cases:
//! 1. Happy path — agent advances via `run_claude_pilot`, guard satisfied.
//! 2. Happy path — agent halts via `update_task_status(parent, blocked)`.
//! 3. Guard fires — agent only updates child task, guard rejects, retry advances.
//! 4. Non-milestone callback — guard does not fire (no milestone-parent marker).
//! 5. PostCallbackAdvance trigger message — guard fires on deliberation, retry
//!    advances (Phase 6 Test 3).
//! 6. PostCallbackAdvance trigger message — guard satisfied when agent advances
//!    immediately (Phase 6 Test 4).
//! 7. Heartbeat milestone resume — heartbeat trigger with milestone context
//!    (Phase 6 Test 6).
//! 8. Three-task chained advance: success path (Phase 6 Test 7a / AC#3).
//! 9. Three-task chained advance: auto_skipped path (Phase 6 Test 7b / AC#3).
//! 10. Three-task chained advance: failure path (Phase 6 Test 7c / AC#3).
//!
//! --- HOLD re-entry cohort (#1208) ---
//! 11. Webhook PR-closed advances next child — HOLD child completes, run_gh
//!     verifies MERGED, list_tasks finds next pending, run_claude_pilot dispatches.
//! 12. Webhook PR-closed → operator notification on last child — no pending
//!     children remain, operator surfaces M5 close-out prompt.
//! 13. Webhook PR-closed → deploy-hook path — child has needs-deploy label,
//!     deploy_mika called instead of run_claude_pilot.
//! 14. Idempotent HOLD re-entry on PostCallbackAdvance — backstop fires while
//!     child is HOLD, no re-dispatch, parent milestone blocked.
//! 15. PR state race — webhook arrives but run_gh returns state != MERGED,
//!     child re-set to HOLD.

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

// ---------------------------------------------------------------------------
// Stub tools
// ---------------------------------------------------------------------------

/// Stub `update_task_status` that always succeeds.
struct StubUpdateTaskStatusTool;

#[async_trait]
impl Tool for StubUpdateTaskStatusTool {
    fn name(&self) -> &str {
        "update_task_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_task_status".to_string(),
            description: "Stub update_task_status for milestone advance guard tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("Task updated.".to_string()))
    }
}

/// Stub `run_claude_pilot` that always succeeds — simulates dispatching
/// the next child in a milestone.
struct StubRunClaudePilotTool;

#[async_trait]
impl Tool for StubRunClaudePilotTool {
    fn name(&self) -> &str {
        "run_claude_pilot"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: "Stub run_claude_pilot for milestone advance guard tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "claude-pilot dispatched for next child.".to_string(),
        ))
    }
}

/// Stub `check_task` that always succeeds.
struct StubCheckTaskTool;

#[async_trait]
impl Tool for StubCheckTaskTool {
    fn name(&self) -> &str {
        "check_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_task".to_string(),
            description: "Stub check_task for milestone advance guard tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "Task parent-milestone-id: in_progress, type=milestone".to_string(),
        ))
    }
}

/// Build a tool registry with the stub tools needed for milestone advance tests.
fn tools_with_milestone_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubRunClaudePilotTool));
    tools.register(Box::new(StubCheckTaskTool));
    tools
}

/// The parent milestone task ID used in test user messages.
const PARENT_TASK_ID: &str = "parent-milestone-id";

/// A callback user message with milestone-parent marker.
fn milestone_callback_msg() -> String {
    format!(
        "[callback: long_running:run_claude_pilot] [milestone-parent: {PARENT_TASK_ID}] \
         Task completed."
    )
}

// ---------------------------------------------------------------------------
// Test 1: Happy path — advance via run_claude_pilot (Path A).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn milestone_advance_guard_happy_path_advance() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls update_task_status for the child
            tool_call_response(
                "update_task_status",
                json!({"task_id": "child-task-id", "status": "completed"}),
            ),
            // Step 2: Agent calls send_message
            tool_call_response(
                "send_message",
                json!({"text": "Child completed. Advancing to next."}),
            ),
            // Step 3: Agent dispatches next child — satisfies Path A
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "mika#124", "task_id": "next-child-id"}),
            ),
            // Step 4: Final text
            text_response("Dispatched next child in milestone."),
        ])
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&milestone_callback_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Dispatched next child");
    assert_tools_include(&trace, &["run_claude_pilot", "update_task_status"]);
    assert_exact_steps(&trace, 4);
}

// ---------------------------------------------------------------------------
// Test 2: Happy path — halt milestone via update_task_status(parent, blocked)
// (Path B).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn milestone_advance_guard_happy_path_halt() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls update_task_status for the child
            tool_call_response(
                "update_task_status",
                json!({"task_id": "child-task-id", "status": "completed"}),
            ),
            // Step 2: Agent halts the milestone (Path B) — note parent_task_id
            // in the input, which the guard's satisfied predicate checks.
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": PARENT_TASK_ID,
                    "status": "blocked",
                    "note": "External dependency blocks next child"
                }),
            ),
            // Step 3: Agent notifies operator
            tool_call_response(
                "send_message",
                json!({"text": "Milestone blocked — external dependency."}),
            ),
            // Step 4: Final text — both guards satisfied
            text_response("Milestone halted."),
        ])
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&milestone_callback_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Milestone halted");
    assert_tools_include(&trace, &["update_task_status", "send_message"]);
    assert_exact_steps(&trace, 4);
}

// ---------------------------------------------------------------------------
// Test 3: Guard fires — agent deliberates, guard rejects, retry advances.
//
// The agent only calls update_task_status for the child (not parent) and
// sends a confirmation question. The callback_terminal_action guard is
// satisfied (both update_task_status + send_message present), but
// callback_milestone_advance is NOT (no run_claude_pilot and no parent
// status update). Guard fires, agent retries and advances.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn milestone_advance_guard_fires_on_deliberation() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls update_task_status for child
            tool_call_response(
                "update_task_status",
                json!({"task_id": "child-task-id", "status": "completed"}),
            ),
            // Step 2: Agent calls send_message (deliberation pattern!)
            tool_call_response(
                "send_message",
                json!({"text": "Task done. Want me to proceed to the next issue?"}),
            ),
            // Step 3: Agent tries to EndTurn — guard rejects
            text_response("Waiting for operator confirmation."),
            // Step 4 (after re-prompt): Agent advances via run_claude_pilot
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "mika#125", "task_id": "next-child-id"}),
            ),
            // Step 5: Final text — guard now satisfied
            text_response("Advanced to next child after guard correction."),
        ])
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&milestone_callback_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Advanced to next child");
    assert_tools_include(&trace, &["run_claude_pilot"]);
    // 5 steps: update + send + rejected text + run_claude_pilot + final text
    assert_exact_steps(&trace, 5);
}

// ---------------------------------------------------------------------------
// Test 4: Non-milestone callback — guard does not fire.
//
// Regular callback without [milestone-parent:] marker. The
// callback_terminal_action guard still applies, but
// callback_milestone_advance does NOT fire.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn milestone_advance_guard_skips_non_milestone_callback() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls update_task_status
            tool_call_response(
                "update_task_status",
                json!({"task_id": "some-task", "status": "completed"}),
            ),
            // Step 2: Agent calls send_message
            tool_call_response("send_message", json!({"text": "Task completed."})),
            // Step 3: Final text — no milestone advance required
            text_response("Done."),
        ])
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    // No [milestone-parent:] marker — guard should not fire
    let trace = harness
        .run("[callback: long_running:run_claude_pilot] Task completed.")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Done");
    // Only 3 steps — no guard rejection
    assert_exact_steps(&trace, 3);
}

// ---------------------------------------------------------------------------
// Test 5: PostCallbackAdvance trigger — agent advances immediately.
//
// Simulates the user message produced by `SilentTrigger::PostCallbackAdvance`:
//   `[advance: <parent_id>] [milestone-parent: <parent_id>]`
// The `callback_milestone_advance` guard does NOT trigger on `[advance:]`
// messages (it requires `[callback:` prefix). The advance turn relies on
// the dispatcher's DB-state check after the turn completes. This test
// verifies the agent can advance cleanly on the advance trigger message.
// (Phase 6 Test 3 — validates the advance trigger's user message shape)
// ---------------------------------------------------------------------------

/// A PostCallbackAdvance user message — produced by the engine when
/// the callback turn did not advance the milestone queue.
fn advance_trigger_msg() -> String {
    format!("[advance: {PARENT_TASK_ID}] [milestone-parent: {PARENT_TASK_ID}]")
}

#[tokio::test]
async fn post_callback_advance_agent_advances_immediately() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent dispatches next child immediately
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "mika#200", "task_id": "next-child-id"}),
            ),
            // Step 2: Agent notifies operator
            tool_call_response(
                "send_message",
                json!({"text": "Dispatched next child in milestone."}),
            ),
            // Step 3: Final text
            text_response("Milestone queue advanced."),
        ])
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&advance_trigger_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Milestone queue advanced");
    assert_tools_include(&trace, &["run_claude_pilot"]);
    // 3 steps — no guard rejection (advance trigger is not guarded by
    // callback_milestone_advance; the dispatcher enforces via DB state)
    assert_exact_steps(&trace, 3);
}

// ---------------------------------------------------------------------------
// Test 6: PostCallbackAdvance trigger — agent halts milestone.
//
// The agent receives the advance trigger message and halts the milestone
// via `update_task_status(parent, blocked)`. This is Path B behavior
// on an advance trigger turn. Since the guard doesn't fire on `[advance:]`
// messages, the agent can EndTurn after halting.
// (Phase 6 Test 4 — validates Path B on advance trigger)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_callback_advance_agent_halts_milestone() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent checks milestone state
            tool_call_response("check_task", json!({"task_id": PARENT_TASK_ID})),
            // Step 2: Agent halts the milestone (Path B)
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": PARENT_TASK_ID,
                    "status": "blocked",
                    "note": "All remaining children blocked by external dependency"
                }),
            ),
            // Step 3: Agent notifies operator
            tool_call_response(
                "send_message",
                json!({"text": "Milestone halted — external dependency."}),
            ),
            // Step 4: Final text
            text_response("Milestone blocked."),
        ])
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&advance_trigger_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Milestone blocked");
    assert_tools_include(&trace, &["update_task_status"]);
    // 4 steps — no guard rejection
    assert_exact_steps(&trace, 4);
}

// ---------------------------------------------------------------------------
// Test 7: Heartbeat with milestone context — heartbeat trigger message
// should allow advancing the milestone queue.
//
// The `[heartbeat trigger]` message does NOT carry `[milestone-parent:]`
// marker, so the callback_milestone_advance guard does NOT fire. The agent
// should be able to call `run_claude_pilot` to resume a stalled milestone
// based on prompt-level instructions without the guard interfering.
// (Phase 6 Test 6)
// ---------------------------------------------------------------------------

/// Stub `list_tasks` that returns a milestone with pending children.
struct StubListTasksTool;

#[async_trait]
impl Tool for StubListTasksTool {
    fn name(&self) -> &str {
        "list_tasks"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_tasks".to_string(),
            description: "Stub list_tasks for heartbeat milestone resume tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "1 items total — 1 in_progress\n\n\
             | ID | Label | Status | Type |\n\
             | parent-milestone-id | Milestone #19 | in_progress | milestone |"
                .to_string(),
        ))
    }
}

fn tools_with_heartbeat_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubRunClaudePilotTool));
    tools.register(Box::new(StubCheckTaskTool));
    tools.register(Box::new(StubListTasksTool));
    tools
}

#[tokio::test]
async fn heartbeat_milestone_resume_advances_queue() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent queries tasks to find stalled milestones
            tool_call_response(
                "list_tasks",
                json!({"status": "in_progress", "type": "milestone"}),
            ),
            // Step 2: Agent checks the milestone for pending children
            tool_call_response("check_task", json!({"task_id": PARENT_TASK_ID})),
            // Step 3: Agent dispatches next child via run_claude_pilot
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "mika#202", "task_id": "next-child-id"}),
            ),
            // Step 4: Agent notifies operator
            tool_call_response(
                "send_message",
                json!({"text": "Resumed stalled milestone — dispatched next child."}),
            ),
            // Step 5: Final text
            text_response("Heartbeat: milestone resumed."),
        ])
        .tools(tools_with_heartbeat_stubs())
        .build()
        .await
        .unwrap();

    // Heartbeat message — no [milestone-parent:] marker, guard should not fire
    let trace = harness.run("[heartbeat trigger]").await.unwrap();

    assert_has_output(&trace);
    assert_tools_include(&trace, &["run_claude_pilot", "list_tasks"]);
    // 5 steps — no guard rejection, agent advances via prompt-level behavior
    assert_exact_steps(&trace, 5);
}

// ---------------------------------------------------------------------------
// Tests 8–10: Three-task chained advance integration (AC#3 closure proof).
//
// Simulates a milestone with 3 children. After each child's callback
// completes, the agent must advance to the next child. Uses `run_turn()`
// to simulate multiple sequential callback + advance turns within the
// same test harness.
//
// Sub-runs:
// 8a (success): child 1 → success → child 2 dispatched → success → child 3
// 8b (auto_skipped): child 1 → auto_skipped → child 2 dispatched → success → child 3
// 8c (failure): child 1 → failure → child 2 dispatched (or milestone blocked)
// ---------------------------------------------------------------------------

/// Helper: run a single callback-then-advance cycle. Returns the agent trace.
/// The agent receives a callback message, processes it, and must advance
/// to the next child by calling `run_claude_pilot`.
async fn run_callback_advance_cycle(
    harness: &EvalHarness,
    child_num: usize,
    outcome: &str,
    next_child_num: usize,
) -> super::trace::AgentTrace {
    let callback_msg = format!(
        "[callback: long_running:run_claude_pilot] [milestone-parent: {PARENT_TASK_ID}] \
         Child {} {outcome}.",
        child_num
    );

    let responses = vec![
        // Step 1: Update child status
        tool_call_response(
            "update_task_status",
            json!({"task_id": format!("child-{child_num}"), "status": outcome}),
        ),
        // Step 2: Notify operator
        tool_call_response(
            "send_message",
            json!({"text": format!("Child {child_num} {outcome}. Advancing to child {next_child_num}.")}),
        ),
        // Step 3: Dispatch next child — satisfies Path A
        tool_call_response(
            "run_claude_pilot",
            json!({
                "skill": "dev-pilot",
                "prompt": format!("mika#{}", 300 + next_child_num),
                "task_id": format!("child-{next_child_num}")
            }),
        ),
        // Step 4: Final text
        text_response(&format!("Dispatched child {next_child_num}.")),
    ];

    harness.run_turn(&callback_msg, responses).await.unwrap()
}

/// Helper: run a callback cycle where the agent halts the milestone (for
/// failure scenarios where halting is a valid outcome).
async fn run_callback_halt_cycle(
    harness: &EvalHarness,
    child_num: usize,
    outcome: &str,
) -> super::trace::AgentTrace {
    let callback_msg = format!(
        "[callback: long_running:run_claude_pilot] [milestone-parent: {PARENT_TASK_ID}] \
         Child {} {outcome}.",
        child_num
    );

    let responses = vec![
        // Step 1: Update child status
        tool_call_response(
            "update_task_status",
            json!({"task_id": format!("child-{child_num}"), "status": outcome}),
        ),
        // Step 2: Halt the milestone (Path B)
        tool_call_response(
            "update_task_status",
            json!({
                "task_id": PARENT_TASK_ID,
                "status": "blocked",
                "note": format!("Child {child_num} failed — blocking milestone for review")
            }),
        ),
        // Step 3: Notify operator
        tool_call_response(
            "send_message",
            json!({"text": format!("Milestone blocked after child {child_num} failure.")}),
        ),
        // Step 4: Final text
        text_response("Milestone halted due to failure."),
    ];

    harness.run_turn(&callback_msg, responses).await.unwrap()
}

#[tokio::test]
async fn chained_advance_three_tasks_success() {
    // Sub-run 8a: child1 success → child2 → child2 success → child3
    let harness = EvalHarness::builder()
        .responses(vec![
            // Initial turn (not a callback — just set up the harness)
            text_response("Ready."),
        ])
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    // Warm up harness
    let _ = harness.run("Initialize.").await.unwrap();

    // Child 1 completes → agent advances to child 2
    let trace1 = run_callback_advance_cycle(&harness, 1, "completed", 2).await;
    assert_tools_include(&trace1, &["run_claude_pilot"]);
    assert_exact_steps(&trace1, 4);

    // Child 2 completes → agent advances to child 3
    let trace2 = run_callback_advance_cycle(&harness, 2, "completed", 3).await;
    assert_tools_include(&trace2, &["run_claude_pilot"]);
    assert_exact_steps(&trace2, 4);
}

#[tokio::test]
async fn chained_advance_three_tasks_auto_skipped() {
    // Sub-run 8b: child1 auto_skipped → child2 → child2 success → child3
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Ready.")])
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    let _ = harness.run("Initialize.").await.unwrap();

    // Child 1 auto_skipped → agent advances to child 2
    let trace1 = run_callback_advance_cycle(&harness, 1, "auto_skipped", 2).await;
    assert_tools_include(&trace1, &["run_claude_pilot"]);
    assert_exact_steps(&trace1, 4);

    // Child 2 completes → agent advances to child 3
    let trace2 = run_callback_advance_cycle(&harness, 2, "completed", 3).await;
    assert_tools_include(&trace2, &["run_claude_pilot"]);
    assert_exact_steps(&trace2, 4);
}

#[tokio::test]
async fn chained_advance_three_tasks_failure() {
    // Sub-run 8c: child1 failure → milestone blocked OR child2 dispatched
    // Per AC#3: failure-class outcomes legitimately block.
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Ready.")])
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    let _ = harness.run("Initialize.").await.unwrap();

    // Child 1 failed → agent halts milestone (Path B) — valid outcome
    let trace = run_callback_halt_cycle(&harness, 1, "failed").await;

    assert_has_output(&trace);
    assert_tools_include(&trace, &["update_task_status", "send_message"]);
    // Verify the halt targeted the parent milestone
    assert_tool_args_contain(
        &trace,
        "update_task_status",
        1,
        json!({"task_id": PARENT_TASK_ID}),
    );
    assert_exact_steps(&trace, 4);
}

// ===========================================================================
// HOLD re-entry cohort (#1208) — Webhook-originated milestone advance tests.
//
// These tests verify the M4 HOLD re-entry semantics introduced by mika#1208:
// - The `pull_request.closed(merged: true)` webhook handler must advance the
//   milestone queue (step 5.5 in self-dev-webhook-qa).
// - The PostCallbackAdvance backstop must detect HOLD state as a no-op.
// - PR state races must be handled gracefully.
//
// The user message for webhook tests uses the `[GitHub] PR closed:` prefix
// (matching the gateway's `format_event_text()` output), which is on the
// qa-territory allowlist in `is_unauthorized_webhook_dispatch()`.
// ===========================================================================

/// Parameterized `run_gh` stub — returns MERGED or OPEN based on `merged` flag.
struct StubRunGhTool {
    merged: bool,
}

#[async_trait]
impl Tool for StubRunGhTool {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Stub run_gh for HOLD re-entry tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        if self.merged {
            Ok(ToolOutput::success(
                r#"{"state": "MERGED", "mergedAt": "2026-05-20T10:00:00Z"}"#.to_string(),
            ))
        } else {
            Ok(ToolOutput::success(
                r#"{"state": "OPEN", "mergedAt": null}"#.to_string(),
            ))
        }
    }
}

/// Stub `deploy_mika` that always succeeds.
struct StubDeployMikaTool;

#[async_trait]
impl Tool for StubDeployMikaTool {
    fn name(&self) -> &str {
        "deploy_mika"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "deploy_mika".to_string(),
            description: "Stub deploy_mika for HOLD re-entry deploy-hook tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("deploy_mika dispatched.".to_string()))
    }
}

/// Stub `update_task_metadata` that always succeeds.
struct StubUpdateTaskMetadataTool;

#[async_trait]
impl Tool for StubUpdateTaskMetadataTool {
    fn name(&self) -> &str {
        "update_task_metadata"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_task_metadata".to_string(),
            description: "Stub update_task_metadata for HOLD note persistence".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("Metadata updated.".to_string()))
    }
}

/// Base registry for HOLD re-entry tests — shared stubs without run_gh variant.
fn hold_reentry_base_tools() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubRunClaudePilotTool));
    tools.register(Box::new(StubCheckTaskTool));
    tools.register(Box::new(StubListTasksTool));
    tools.register(Box::new(StubUpdateTaskMetadataTool));
    tools
}

/// HOLD re-entry tests with MERGED run_gh stub (happy path).
fn tools_with_hold_reentry_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = hold_reentry_base_tools();
    tools.register(Box::new(StubRunGhTool { merged: true }));
    tools
}

/// HOLD re-entry tests with NOT-MERGED run_gh stub (race condition).
fn tools_with_hold_reentry_not_merged_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = hold_reentry_base_tools();
    tools.register(Box::new(StubRunGhTool { merged: false }));
    tools
}

/// HOLD re-entry tests with deploy_mika for deploy-hook path.
fn tools_with_hold_reentry_deploy_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = hold_reentry_base_tools();
    tools.register(Box::new(StubRunGhTool { merged: true }));
    tools.register(Box::new(StubDeployMikaTool));
    tools
}

/// A webhook PR-closed user message (matching gateway `format_event_text` output).
fn webhook_pr_closed_msg() -> String {
    "[GitHub] PR closed: senara-solutions/mika#1050 — feat: add health endpoint (branch: feat/health)\nhttps://github.com/senara-solutions/mika/pull/1050".to_string()
}

// ---------------------------------------------------------------------------
// Test 11: Webhook PR-closed advances next child (#1208 Phase 2 step 5.5.c).
//
// HOLD child completes via webhook. Agent verifies PR merged via run_gh,
// checks milestone parent, finds next pending child, dispatches via
// run_claude_pilot.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hold_reentry_webhook_advances_next_child() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Correlate — find task with verdict_merge: auto
            tool_call_response(
                "list_tasks",
                json!({"status": "in_progress"}),
            ),
            // Step 2: Complete the HOLD child
            tool_call_response(
                "update_task_status",
                json!({"task_id": "child-hold-id", "status": "completed"}),
            ),
            // Step 3: Verify PR actually merged (step 5.5.a)
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "view", "1050", "--json", "state,mergedAt"], "repo": "senara-solutions/mika"}),
            ),
            // Step 4: Check parent to determine if milestone context
            tool_call_response("check_task", json!({"task_id": PARENT_TASK_ID})),
            // Step 5: Find next pending child (step 5.5.c)
            tool_call_response(
                "list_tasks",
                json!({"parent_task_id": PARENT_TASK_ID}),
            ),
            // Step 6: Dispatch next child via run_claude_pilot
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "mika#1051", "task_id": "next-child-id"}),
            ),
            // Step 7: Final text
            text_response("Webhook: PR merged, dispatched next milestone child."),
        ])
        .tools(tools_with_hold_reentry_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&webhook_pr_closed_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "dispatched next milestone child");
    assert_tools_include(
        &trace,
        &["run_gh", "run_claude_pilot", "update_task_status"],
    );
    // Verify dispatch targets the next child (not arbitrary)
    assert_tool_args_contain(
        &trace,
        "run_claude_pilot",
        0,
        json!({"task_id": "next-child-id"}),
    );
}

// ---------------------------------------------------------------------------
// Test 12: Webhook PR-closed → last child, operator notification (#1208
// Phase 2 step 5.5.c last-child branch).
//
// HOLD child is the last pending child. Agent completes it, verifies merge,
// but finds no more pending children. Surfaces M5 close-out prompt to
// operator instead of dispatching.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hold_reentry_webhook_last_child_operator_notification() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Correlate task
            tool_call_response(
                "list_tasks",
                json!({"status": "in_progress"}),
            ),
            // Step 2: Complete the HOLD child
            tool_call_response(
                "update_task_status",
                json!({"task_id": "child-hold-id", "status": "completed"}),
            ),
            // Step 3: Verify PR merged
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "view", "1050", "--json", "state,mergedAt"], "repo": "senara-solutions/mika"}),
            ),
            // Step 4: Check parent (milestone)
            tool_call_response("check_task", json!({"task_id": PARENT_TASK_ID})),
            // Step 5: List children — no pending children remain
            tool_call_response(
                "list_tasks",
                json!({"parent_task_id": PARENT_TASK_ID}),
            ),
            // Step 6: Update milestone parent status (operator resume needed)
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": PARENT_TASK_ID,
                    "status": "in_progress",
                    "note": "Auto-merge of mika issue#1050 completed via webhook — operator-resume needed to drive M5 close-out"
                }),
            ),
            // Step 7: Notify operator
            tool_call_response(
                "send_message",
                json!({"text": "Milestone mika milestone#19 last child auto-merged via webhook. Reply 'continue' to run M5 close-out."}),
            ),
            // Step 8: Final text
            text_response("Last child merged. Awaiting operator to drive M5."),
        ])
        .tools(tools_with_hold_reentry_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&webhook_pr_closed_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "M5");
    // run_claude_pilot should NOT be called — no next child to dispatch
    assert_tools_exclude(&trace, &["run_claude_pilot"]);
    assert_tools_include(&trace, &["update_task_status", "send_message", "run_gh"]);
    // Verify parent milestone update targets the correct task
    // (update_task_status call 0 is the child completed; call 1 is the parent)
    assert_tool_args_contain(
        &trace,
        "update_task_status",
        1,
        json!({"task_id": PARENT_TASK_ID}),
    );
}

// ---------------------------------------------------------------------------
// Test 13: Webhook PR-closed → deploy-hook path (#1208 Phase 2 step 5.5.b).
//
// HOLD child has `needs-deploy` label. Agent completes child, verifies
// merge, detects deploy-hook label, calls deploy_mika instead of
// run_claude_pilot.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hold_reentry_webhook_deploy_hook() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Correlate task
            tool_call_response(
                "list_tasks",
                json!({"status": "in_progress"}),
            ),
            // Step 2: Complete the HOLD child
            tool_call_response(
                "update_task_status",
                json!({"task_id": "child-hold-id", "status": "completed"}),
            ),
            // Step 3: Verify PR merged
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "view", "1050", "--json", "state,mergedAt"], "repo": "senara-solutions/mika"}),
            ),
            // Step 4: Check parent (milestone)
            tool_call_response("check_task", json!({"task_id": PARENT_TASK_ID})),
            // Step 5: Notify about deploy hook
            tool_call_response(
                "send_message",
                json!({"text": "Deploy hook triggered for mika#1050 via auto-merge webhook (label: needs-deploy). Running build+deploy before next ticket."}),
            ),
            // Step 6: Call deploy_mika (step 5.5.b)
            tool_call_response(
                "deploy_mika",
                json!({"task_id": PARENT_TASK_ID}),
            ),
            // Step 7: Final text — turn ends, deploy callback drives next iteration
            text_response("Deploy triggered. Callback will drive next milestone step."),
        ])
        .tools(tools_with_hold_reentry_deploy_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&webhook_pr_closed_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_tools_include(&trace, &["deploy_mika", "run_gh"]);
    // run_claude_pilot should NOT be called — deploy_mika takes over
    assert_tools_exclude(&trace, &["run_claude_pilot"]);
    // Verify deploy_mika targets the parent milestone (task_id == milestone_wi)
    assert_tool_args_contain(&trace, "deploy_mika", 0, json!({"task_id": PARENT_TASK_ID}));
}

// ---------------------------------------------------------------------------
// Test 14: Idempotent HOLD re-entry on PostCallbackAdvance (#1208 Phase 1).
//
// A PostCallbackAdvance backstop fires while the child is still in HOLD
// (webhook hasn't arrived yet). The agent should detect the HOLD state,
// NOT re-dispatch, and block the parent milestone with an operator
// notification.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hold_reentry_idempotent_postcallbackadvance() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent checks the child task — sees HOLD state
            tool_call_response("check_task", json!({"task_id": "child-hold-id"})),
            // Step 2: Agent blocks the parent milestone
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": PARENT_TASK_ID,
                    "status": "blocked",
                    "note": "HOLD child not yet merged after PostCallbackAdvance — auto-merge may be stuck; operator review"
                }),
            ),
            // Step 3: Agent notifies operator
            tool_call_response(
                "send_message",
                json!({"text": "PostCallbackAdvance fired but child is still in HOLD — auto-merge may be stuck. Blocking milestone for operator review."}),
            ),
            // Step 4: Final text
            text_response("HOLD re-entry: no-op, milestone blocked for operator review."),
        ])
        .tools(tools_with_hold_reentry_stubs())
        .build()
        .await
        .unwrap();

    // PostCallbackAdvance trigger message — same format as test 5 above
    let trace = harness.run(&advance_trigger_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "HOLD");
    // run_claude_pilot should NOT be called — child is still HOLD
    assert_tools_exclude(&trace, &["run_claude_pilot"]);
    assert_tools_include(
        &trace,
        &["check_task", "update_task_status", "send_message"],
    );
    // Verify the block targeted the parent milestone
    assert_tool_args_contain(
        &trace,
        "update_task_status",
        0,
        json!({"task_id": PARENT_TASK_ID, "status": "blocked"}),
    );
}

// ---------------------------------------------------------------------------
// Test 15: PR state race — webhook arrives but state != MERGED (#1208
// Phase 2 step 5.5.a).
//
// The pull_request.closed webhook arrives but run_gh returns state=OPEN
// (race condition or non-merge close). The agent should re-set the child
// to HOLD, notify Vincent, and NOT advance the milestone.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hold_reentry_pr_state_race_not_merged() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Correlate task
            tool_call_response(
                "list_tasks",
                json!({"status": "in_progress"}),
            ),
            // Step 2: Complete the HOLD child (premature — will be reverted)
            tool_call_response(
                "update_task_status",
                json!({"task_id": "child-hold-id", "status": "completed"}),
            ),
            // Step 3: Verify PR — returns OPEN (not merged!)
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "view", "1050", "--json", "state,mergedAt"], "repo": "senara-solutions/mika"}),
            ),
            // Step 4: Re-set child to HOLD (step 5.5.a race handling)
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": "child-hold-id",
                    "status": "in_progress",
                    "note": "HOLD: webhook arrived but PR state != MERGED; awaiting confirmation"
                }),
            ),
            // Step 5: Notify Vincent
            tool_call_response(
                "send_message",
                json!({"text": "PR mika#1050 webhook fired but state is OPEN (not MERGED). Re-setting child to HOLD."}),
            ),
            // Step 6: Final text
            text_response("PR state race detected. Child remains in HOLD."),
        ])
        .tools(tools_with_hold_reentry_not_merged_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&webhook_pr_closed_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "HOLD");
    // run_claude_pilot should NOT be called — PR not actually merged
    assert_tools_exclude(&trace, &["run_claude_pilot"]);
    assert_tools_include(&trace, &["run_gh", "update_task_status", "send_message"]);
    // Verify the HOLD re-set targets the correct child with in_progress status
    // (update_task_status call 1 is the re-set; call 0 is the premature complete)
    assert_tool_args_contain(
        &trace,
        "update_task_status",
        1,
        json!({"task_id": "child-hold-id", "status": "in_progress"}),
    );
}

// ===========================================================================
// Webhook milestone advance guard cohort (#1218).
//
// Mirrors the callback milestone advance guard (#991) but fires on
// `[GitHub] PR closed:` webhook turns instead of `[callback:` callback turns.
// The guard triggers when the user message contains BOTH
// `[milestone-parent: <id>]` AND `[GitHub] PR closed:`.
//
// Satisfaction has 3 paths:
// - Path A: `run_claude_pilot` or `run_claude_pilot_groom` called.
// - Path B: `update_task_status` targeting the parent with `blocked`/`completed`.
// - Path C: BOTH `deploy_mika` AND `send_message` called (deploy-hook ack).
//
// Tests 16–27.
// ===========================================================================

/// A webhook PR-closed user message WITH the milestone-parent marker
/// (as enriched by milestone_context_handler in production).
fn webhook_milestone_pr_closed_msg() -> String {
    format!(
        "[milestone-parent: {PARENT_TASK_ID}]\n\
         [GitHub] PR closed: senara-solutions/mika#1050 — feat: add health endpoint (branch: feat/health)\n\
         https://github.com/senara-solutions/mika/pull/1050\n\
         Merged: true"
    )
}

/// Stub `run_claude_pilot_groom` that always succeeds — simulates grooming
/// dispatch on a milestone child.
struct StubRunClaudePilotGroomTool;

#[async_trait]
impl Tool for StubRunClaudePilotGroomTool {
    fn name(&self) -> &str {
        "run_claude_pilot_groom"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_claude_pilot_groom".to_string(),
            description: "Stub run_claude_pilot_groom for webhook milestone advance guard tests"
                .to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "claude-pilot grooming dispatched.".to_string(),
        ))
    }
}

/// Tool registry with milestone stubs + run_claude_pilot_groom for webhook tests.
fn tools_with_webhook_milestone_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubRunClaudePilotTool));
    tools.register(Box::new(StubRunClaudePilotGroomTool));
    tools.register(Box::new(StubCheckTaskTool));
    tools.register(Box::new(StubDeployMikaTool));
    tools
}

// ---------------------------------------------------------------------------
// Test 16: Webhook Path A — run_claude_pilot satisfies guard (AC2a).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_path_a_accepts_run_claude_pilot() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent dispatches next child — satisfies Path A
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "mika#1051", "task_id": "next-child-id"}),
            ),
            // Step 2: Final text
            text_response("Webhook: dispatched next milestone child."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(&webhook_milestone_pr_closed_msg())
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "dispatched next milestone child");
    assert_tools_include(&trace, &["run_claude_pilot"]);
    // 2 steps — guard does NOT fire (Path A satisfied)
    assert_exact_steps(&trace, 2);
}

// ---------------------------------------------------------------------------
// Test 17: Webhook Path A — run_claude_pilot_groom satisfies guard (AC2a).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_path_a_accepts_run_claude_pilot_groom() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent dispatches grooming — satisfies Path A
            tool_call_response(
                "run_claude_pilot_groom",
                json!({"skill": "dev-groom", "prompt": "mika#1051", "task_id": "next-child-id"}),
            ),
            // Step 2: Final text
            text_response("Webhook: grooming dispatched for next child."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(&webhook_milestone_pr_closed_msg())
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "grooming dispatched");
    assert_tools_include(&trace, &["run_claude_pilot_groom"]);
    // 2 steps — guard does NOT fire (Path A satisfied)
    assert_exact_steps(&trace, 2);
}

// ---------------------------------------------------------------------------
// Test 18: Webhook Path B — update_task_status(parent, blocked) (AC2b).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_path_b_accepts_halt_blocked() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent halts the milestone — Path B
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": PARENT_TASK_ID,
                    "status": "blocked",
                    "note": "All children completed or blocked"
                }),
            ),
            // Step 2: Agent notifies operator
            tool_call_response(
                "send_message",
                json!({"text": "Milestone blocked after webhook."}),
            ),
            // Step 3: Final text
            text_response("Milestone halted via webhook path."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(&webhook_milestone_pr_closed_msg())
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "halted");
    assert_tools_include(&trace, &["update_task_status"]);
    // 3 steps — guard does NOT fire (Path B satisfied)
    assert_exact_steps(&trace, 3);
}

// ---------------------------------------------------------------------------
// Test 19: Webhook Path B — update_task_status(parent, completed) (AC2b).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_path_b_accepts_halt_completed() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent completes the milestone — Path B
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": PARENT_TASK_ID,
                    "status": "completed",
                    "note": "All milestone children merged"
                }),
            ),
            // Step 2: Agent notifies operator
            tool_call_response(
                "send_message",
                json!({"text": "Milestone completed — all children merged."}),
            ),
            // Step 3: Final text
            text_response("Milestone completed via webhook."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(&webhook_milestone_pr_closed_msg())
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "completed");
    assert_tools_include(&trace, &["update_task_status"]);
    // 3 steps — guard does NOT fire (Path B satisfied)
    assert_exact_steps(&trace, 3);
}

// ---------------------------------------------------------------------------
// Test 20: Webhook Path C — deploy_mika + send_message satisfies guard.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_path_c_accepts_deploy_with_send_message_any_text() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls deploy_mika
            tool_call_response("deploy_mika", json!({"task_id": PARENT_TASK_ID})),
            // Step 2: Agent calls send_message — completes Path C
            tool_call_response(
                "send_message",
                json!({"text": "Deploy triggered for webhook PR merge."}),
            ),
            // Step 3: Final text
            text_response("Deploy hook ack complete."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(&webhook_milestone_pr_closed_msg())
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Deploy hook ack");
    assert_tools_include(&trace, &["deploy_mika", "send_message"]);
    // 3 steps — guard does NOT fire (Path C satisfied)
    assert_exact_steps(&trace, 3);
}

// ---------------------------------------------------------------------------
// Test 21: Webhook Path C rejection — deploy_mika alone (no send_message).
//
// deploy_mika without send_message does NOT satisfy Path C. Guard fires,
// then on retry the agent calls run_claude_pilot (Path A) to recover.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_path_c_rejects_deploy_only_no_send_message() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls deploy_mika only
            tool_call_response("deploy_mika", json!({"task_id": PARENT_TASK_ID})),
            // Step 2: Text EndTurn — guard fires (deploy_mika alone is not Path C)
            text_response("Deploy kicked off."),
            // Step 3 (after re-prompt): Agent dispatches via run_claude_pilot (Path A)
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "mika#1051", "task_id": "next-child-id"}),
            ),
            // Step 4: Final text — guard satisfied
            text_response("Corrected: dispatched next child after guard."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(&webhook_milestone_pr_closed_msg())
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Corrected");
    assert_tools_include(&trace, &["deploy_mika", "run_claude_pilot"]);
    // 4 steps: deploy → rejected text → run_claude_pilot → final text
    assert_exact_steps(&trace, 4);
}

// ---------------------------------------------------------------------------
// Test 22: Webhook Path C rejection — send_message alone (no deploy_mika).
//
// send_message without deploy_mika does NOT satisfy any path. Guard fires,
// then on retry the agent calls run_claude_pilot (Path A) to recover.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_path_c_rejects_send_message_only_no_deploy() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls send_message only
            tool_call_response(
                "send_message",
                json!({"text": "PR merged for milestone child."}),
            ),
            // Step 2: Text EndTurn — guard fires (send_message alone is not any path)
            text_response("Notified operator."),
            // Step 3 (after re-prompt): Agent dispatches via run_claude_pilot (Path A)
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "mika#1051", "task_id": "next-child-id"}),
            ),
            // Step 4: Final text — guard satisfied
            text_response("Corrected: dispatched next child."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(&webhook_milestone_pr_closed_msg())
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Corrected");
    assert_tools_include(&trace, &["send_message", "run_claude_pilot"]);
    // 4 steps: send_message → rejected text → run_claude_pilot → final text
    assert_exact_steps(&trace, 4);
}

// ---------------------------------------------------------------------------
// Test 23: Webhook text-only rejection then retry succeeds (AC2c).
//
// Agent emits text-only on first EndTurn (no tools at all). Guard fires,
// re-prompts. Agent calls run_claude_pilot on second try.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_silent_text_rejection_then_retry_succeeds() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Text-only EndTurn — guard fires
            text_response("I see the PR was merged. Let me review the milestone."),
            // Step 2 (after re-prompt): Agent dispatches next child (Path A)
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "mika#1051", "task_id": "next-child-id"}),
            ),
            // Step 3: Final text — guard satisfied
            text_response("Dispatched next child after guard correction."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(&webhook_milestone_pr_closed_msg())
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Dispatched next child");
    assert_tools_include(&trace, &["run_claude_pilot"]);
    // 3 steps: rejected text → run_claude_pilot → final text
    assert_exact_steps(&trace, 3);
}

// ---------------------------------------------------------------------------
// Test 24: Webhook single retry exhaustion — guard fires once, then accepts.
//
// Agent fails to satisfy the guard on both attempts (two text-only EndTurns).
// The guard fires on the first violation, inserts the label into
// intent_guard_retries. On the second text-only EndTurn, the label is already
// in the set, so the guard does NOT fire again — EndTurn is accepted.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_single_retry_semantics() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Text-only EndTurn — guard fires
            text_response("The PR was merged for the milestone child."),
            // Step 2 (after re-prompt): Still text-only — guard already
            // retried, so it accepts EndTurn this time.
            text_response("I cannot dispatch right now."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(&webhook_milestone_pr_closed_msg())
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "cannot dispatch");
    // No dispatch tools called — agent failed both attempts
    assert_tools_exclude(&trace, &["run_claude_pilot", "run_claude_pilot_groom"]);
    // 2 steps: rejected text (guard fires) → accepted text (guard exhausted)
    assert_exact_steps(&trace, 2);
}

// ---------------------------------------------------------------------------
// Test 25: No milestone-parent marker — guard does NOT fire.
//
// A webhook PR-closed message without [milestone-parent:] should not trigger
// the webhook_milestone_advance guard, even on a zero-tool EndTurn.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_no_marker_no_fire() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Single step: text-only EndTurn — both `webhook_milestone_advance`
            // and `webhook_zero_tools` should NOT fire.
            //
            // Post-mika#1469: `webhook_zero_tools` no longer fires on
            // `[GitHub] PR closed:` messages (added to the trigger's prefix-skip
            // list — see agent.rs `webhook_zero_tools_trigger`). PR-closed events
            // for out-of-band PRs are always informational; the agent's text-only
            // acknowledgement is the correct response and the engine accepts it
            // without re-prompting.
            //
            // `webhook_milestone_advance` separately does NOT fire because the
            // message carries no `[milestone-parent: ...]` marker.
            text_response("PR merged. No milestone context."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    // Webhook PR-closed WITHOUT milestone-parent marker
    let trace = harness
        .run(
            "[GitHub] PR closed: senara-solutions/mika#1050 — feat: add health endpoint (branch: feat/health)\n\
             https://github.com/senara-solutions/mika/pull/1050\n\
             Merged: true",
        )
        .await
        .unwrap();

    assert_has_output(&trace);
    // 1 step — post-mika#1469, `[GitHub] PR closed:` is in the
    // `webhook_zero_tools` prefix-skip list, so no guard re-prompt fires
    // and the text-only EndTurn is accepted on the first step.
    assert_exact_steps(&trace, 1);
}

// ---------------------------------------------------------------------------
// Test 26: Callback marker triggers callback guard, NOT webhook guard.
//
// A callback turn with [milestone-parent:] should trigger the existing
// callback_milestone_advance guard (#991), not the webhook guard (#1218).
// This verifies mutual exclusivity — the webhook guard checks for
// `[GitHub] PR closed:` which is absent in callback messages.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_milestone_advance_callback_marker_does_not_trigger_webhook_guard() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls update_task_status for the child
            tool_call_response(
                "update_task_status",
                json!({"task_id": "child-task-id", "status": "completed"}),
            ),
            // Step 2: Agent calls send_message (satisfies callback_terminal_action)
            tool_call_response("send_message", json!({"text": "Child completed."})),
            // Step 3: Agent dispatches next child — satisfies callback_milestone_advance Path A
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "mika#1051", "task_id": "next-child-id"}),
            ),
            // Step 4: Final text
            text_response("Callback: dispatched next child."),
        ])
        .tools(tools_with_webhook_milestone_stubs())
        .build()
        .await
        .unwrap();

    // Callback message with milestone-parent marker — no [GitHub] PR closed:
    let trace = harness.run(&milestone_callback_msg()).await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Callback: dispatched next child");
    assert_tools_include(&trace, &["run_claude_pilot", "update_task_status"]);
    // 4 steps — callback guard path, not webhook guard
    assert_exact_steps(&trace, 4);
}

// ---------------------------------------------------------------------------
// Test 27: Predicate coverage note (empty-text branch mirror).
//
// The webhook_milestone_advance guard has both a non-empty-text branch
// (guard #6c in the EndTurn chain) and an empty-text exit mirror guard
// (same pattern as callback_milestone_advance). In conversation mode,
// `follow_up_on_empty()` returns true, so the empty-text branch is never
// taken — the LLM is re-prompted for content instead.
//
// Tests 16–26 above collectively cover the trigger and satisfaction
// predicate logic through the harness:
// - Trigger: tests 16–24 (marker present → guard evaluates),
//   test 25 (no marker → guard skips), test 26 (callback marker → wrong guard).
// - Path A: tests 16, 17 (run_claude_pilot, run_claude_pilot_groom).
// - Path B: tests 18, 19 (blocked, completed targeting parent).
// - Path C: test 20 (both deploy_mika + send_message), tests 21–22
//   (partial Path C rejected).
// - Single-retry semantics: test 24 (fires once, accepts on exhaustion).
//
// No additional harness test is needed for the empty-text branch — it is
// structurally identical to the non-empty path and is tested implicitly
// by the conversation-mode re-prompt behavior.
// ---------------------------------------------------------------------------
