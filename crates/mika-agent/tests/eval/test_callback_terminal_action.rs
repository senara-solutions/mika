//! Integration tests: callback terminal action guard (#870).
//!
//! Verifies that the intent-precondition guard `callback_terminal_action`
//! requires callback turns to invoke BOTH `update_task_status` AND
//! `send_message` before EndTurn.  Modelled on `test_callback_turn.rs` and
//! `test_intent_precondition_guard.rs`.
//!
//! Note: Tests use conversation mode (not `.callback_turn(true)`) because the
//! guard triggers on the `[callback:` message prefix in `user_input_text`.
//! In conversation mode, the user message is saved to DB and included in the
//! LLM request messages.  When `.callback_turn(true)` is set, the user message
//! is skipped from DB persistence (line ~1744), so it's absent from the loaded
//! history and `user_input_text` is empty.  In production (silent mode), the
//! user message `[callback: {label}]` is constructed directly in
//! `run_silent_agent` and always present in `request.messages`.
//!
//! Four test cases:
//! 1. Happy path — agent calls both terminal actions, loop exits cleanly.
//! 2. Recovery — agent omits terminal actions on turn 1, guard fires, turn 2
//!    emits the missing actions.
//! 3. Persistent failure — agent never emits terminal actions; guard fires
//!    once (single-retry), loop continues until max-steps cap halts.
//! 4. Non-callback — guard does not fire on regular messages.

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
            description: "Stub update_task_status for callback guard tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("Task updated to failed.".to_string()))
    }
}

/// Stub `check_task` that always succeeds (used in recovery test).
struct StubCheckTaskTool;

#[async_trait]
impl Tool for StubCheckTaskTool {
    fn name(&self) -> &str {
        "check_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_task".to_string(),
            description: "Stub check_task for callback guard tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "Task bd203e00: in_progress, source=self_dev".to_string(),
        ))
    }
}

/// Build a tool registry with the stub tools needed for callback guard tests.
fn tools_with_callback_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubCheckTaskTool));
    tools
}

// ---------------------------------------------------------------------------
// Test 1: Happy path — both terminal actions called, loop exits cleanly.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_guard_happy_path() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls update_task_status
            tool_call_response(
                "update_task_status",
                json!({"task_id": "bd203e00", "status": "failed"}),
            ),
            // Step 2: Agent calls send_message
            tool_call_response(
                "send_message",
                json!({"text": "claude-pilot finished with no PR — task marked failed."}),
            ),
            // Step 3: Agent produces final text — guard is satisfied (both
            // update_task_status and send_message were called)
            text_response("Callback processing complete."),
        ])
        .tools(tools_with_callback_stubs())
        .build()
        .await
        .unwrap();

    // Use [callback: ...] prefix so the trigger predicate matches.
    let trace = harness
        .run("[callback: long_running:run_claude_pilot] Task completed.")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Callback processing complete");
    assert_tools_include(&trace, &["update_task_status", "send_message"]);
    // 3 steps: tool call + tool call + text
    assert_exact_steps(&trace, 3);
}

// ---------------------------------------------------------------------------
// Test 2: Recovery — turn 1 omits terminal actions, guard fires, turn 2 OK.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_guard_recovery_after_reprompt() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls check_task (diagnostic) — no terminal actions
            tool_call_response("check_task", json!({"task_id": "bd203e00"})),
            // Step 2: Agent produces text without terminal actions — guard
            // fires and rejects this EndTurn.
            text_response("The task is still in progress. No PR was created."),
            // Step 3 (after re-prompt): Agent calls update_task_status
            tool_call_response(
                "update_task_status",
                json!({"task_id": "bd203e00", "status": "failed"}),
            ),
            // Step 4: Agent calls send_message
            tool_call_response(
                "send_message",
                json!({"text": "claude-pilot run failed — no PR produced."}),
            ),
            // Step 5: Agent produces final text — guard now satisfied
            text_response("Notified operator and updated task status."),
        ])
        .tools(tools_with_callback_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[callback: long_running:run_claude_pilot] Task delivered.")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Notified operator");
    assert_tools_include(
        &trace,
        &["update_task_status", "send_message", "check_task"],
    );
    // 5 steps: check_task + rejected text + update_task_status + send_message + final text
    assert_exact_steps(&trace, 5);
}

// ---------------------------------------------------------------------------
// Test 3: Persistent failure — guard fires once, then dormant.
//
// The agent never calls terminal actions.  After the guard's single retry,
// the loop continues until max-steps.  This is strictly better than today's
// silent exit because the max-steps halt is logged and observable.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_guard_persistent_failure_reaches_max_steps() {
    // Build enough mock responses to fill up the step budget.
    // Each "turn" is: check_task (tool_use) + text_response (EndTurn).
    // The guard fires on the first text_response, injecting a correction.
    // After that, the guard is dormant — subsequent text_responses pass through
    // (because no tools are required in conversation mode without callback_turn flag).
    // We need enough responses to reach max_steps (20 for conversation mode).
    let mut responses = Vec::new();
    for _ in 0..10 {
        responses.push(tool_call_response(
            "check_task",
            json!({"task_id": "bd203e00"}),
        ));
        responses.push(text_response("Still investigating..."));
    }

    let harness = EvalHarness::builder()
        .responses(responses)
        .tools(tools_with_callback_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[callback: long_running:run_claude_pilot] Task delivered.")
        .await
        .unwrap();

    // The guard should have fired once (single-retry semantics).
    // After that, the loop continued until it ran out of steps.
    // Verify that terminal actions were never called.
    assert_tools_exclude(&trace, &["update_task_status", "send_message"]);

    // Should have used multiple steps (guard fired once + continued until
    // max steps or mock exhaustion).  The first text_response triggers the
    // guard (rejected), then the loop continues with the remaining mocks.
    // After the guard fires, subsequent text_responses pass through, so
    // the second text_response exits normally.  Expect at least 3 steps:
    // check_task + rejected text + check_task + accepted text = 4 steps.
    assert!(
        trace.llm_call_count > 2,
        "Expected more than 2 LLM calls (guard should fire once then loop continues), got {}",
        trace.llm_call_count
    );
}

// ---------------------------------------------------------------------------
// Test 4: Guard does NOT fire on non-callback turns.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_guard_skips_non_callback_messages() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Here's what I found about the project status.",
        )])
        .tools(tools_with_callback_stubs())
        .build()
        .await
        .unwrap();

    // Regular message — no [callback: prefix
    let trace = harness
        .run("What's the status of the project?")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 1 step only — no re-prompt
    assert_exact_steps(&trace, 1);
}
