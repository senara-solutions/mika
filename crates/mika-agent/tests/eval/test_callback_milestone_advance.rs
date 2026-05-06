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
