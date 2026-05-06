//! Integration tests: callback milestone advance guard (#991).
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
