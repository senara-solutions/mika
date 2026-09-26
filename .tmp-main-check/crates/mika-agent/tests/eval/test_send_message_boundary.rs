//! Integration tests: send_message turn boundary guard (#771).
//!
//! Verifies that the agent loop enforces the send_message turn boundary:
//! after a successful `send_message` in conversation mode, the agent loop
//! forces EndTurn — preventing further LLM calls that could dispatch writes.

use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

// ---------------------------------------------------------------------------
// Test 1: Read-only tools after send_message are allowed (same step).
//
// The boundary guard does not suppress read tools in the same step as
// send_message — only duplicate send_message calls. After the step
// completes, EndTurn is forced (no further LLM calls).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn send_message_then_read_tool_same_step_passes() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: send_message + list_tasks in same response (multi-tool)
            multi_tool_response(vec![
                (
                    "send_message",
                    json!({"text": "What should I work on next?"}),
                ),
                ("list_tasks", json!({"status": "pending"})),
            ]),
            // This text_response will NOT be reached — boundary forces EndTurn.
            text_response("Here are your pending tasks."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What tasks are pending?").await.unwrap();

    // Both tools should have executed (list_tasks is read, allowed in same step)
    assert_tools_include(&trace, &["send_message", "list_tasks"]);
    // 1 LLM call made, then boundary forced EndTurn (no continuation)
    assert_exact_steps(&trace, 1);
}

// ---------------------------------------------------------------------------
// Test 2: Write tool in NEXT step after send_message is prevented.
//
// The inter-step gate forces EndTurn after a step containing send_message.
// The create_task in step 2 never executes because no second LLM call is made.
// This is the incident case from mika#771.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn send_message_then_write_tool_next_step_prevented() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: send_message
            tool_call_response(
                "send_message",
                json!({"text": "Deploy done? Or dispatch mika#744 now?"}),
            ),
            // Step 2: would dispatch — but boundary forces EndTurn
            tool_call_response("create_task", json!({"label": "unauthorized work"})),
            text_response("Working on it."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("what next?").await.unwrap();

    // send_message executed
    assert_tools_include(&trace, &["send_message"]);
    // create_task never reached (EndTurn forced after step 1)
    assert_tools_exclude(&trace, &["create_task"]);
    assert_exact_steps(&trace, 1);
}

// ---------------------------------------------------------------------------
// Test 3: Two consecutive send_message calls — second suppressed.
//
// In the same LLM response, the second send_message is treated as a write
// and suppressed by the intra-step gate. The suppressed call does not appear
// in the tool_calls DB table (it was never executed), so we verify by
// checking that only one send_message DB row exists.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn two_consecutive_send_messages_second_suppressed() {
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                (
                    "send_message",
                    json!({"text": "Question: deploy or dispatch?"}),
                ),
                (
                    "send_message",
                    json!({"text": "Actually, let me just dispatch."}),
                ),
            ]),
            text_response("Done."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("what should I do?").await.unwrap();

    // Only one send_message should have been executed (saved to DB).
    // The second was suppressed by the intra-step gate.
    let send_calls = trace.calls_for_tool("send_message");
    assert_eq!(
        send_calls.len(),
        1,
        "Only the first send_message should be saved to DB (second was suppressed); got {}",
        send_calls.len()
    );
    assert!(send_calls[0].success, "First send_message should succeed");
    // EndTurn forced after step 1
    assert_exact_steps(&trace, 1);
}

// ---------------------------------------------------------------------------
// Test 4: Write tools BEFORE send_message are unaffected.
//
// The boundary only gates tools AFTER send_message. Tools executed before
// send_message in the same step are not suppressed.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn write_tools_before_send_message_unaffected() {
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("search_memory", json!({"query": "sprint status"})),
                ("send_message", json!({"text": "Sprint has started!"})),
            ]),
            text_response("Notified."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("start the sprint").await.unwrap();

    // Both tools executed successfully (search_memory was before send_message)
    assert_tools_include(&trace, &["search_memory", "send_message"]);
    let search_calls = trace.calls_for_tool("search_memory");
    assert!(
        search_calls[0].success,
        "search_memory should succeed (it's before send_message)"
    );
    // 1 LLM call, then boundary forced EndTurn
    assert_exact_steps(&trace, 1);
}

// ---------------------------------------------------------------------------
// Test 5: Completion-claim guard still works via PostConditionGuard registry.
//
// Verifies behavior preservation after migrating the completion-claim guard
// to the PostConditionGuard registry dispatch in #771.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn completion_claim_guard_via_registry() {
    use async_trait::async_trait;
    use mika_agent::db::NewTask;
    use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
    use mika_common::claude::ToolDefinition;

    // Stub update_task_status so the guard's registry check finds it
    struct StubUpdateTaskStatusTool;

    #[async_trait]
    impl Tool for StubUpdateTaskStatusTool {
        fn name(&self) -> &str {
            "update_task_status"
        }
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "update_task_status".into(),
                description: "stub".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            }
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext<'_>,
        ) -> anyhow::Result<ToolOutput> {
            Ok(ToolOutput::success("updated"))
        }
    }

    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));

    let harness = EvalHarness::builder()
        .responses(vec![
            // First: claims completion without update_task_status
            text_response("PR merged successfully. Main synced."),
            // Second: after re-prompt, corrects
            text_response("Let me verify the actual state first."),
        ])
        .tools(tools)
        .build()
        .await
        .unwrap();

    // Create an active task so the guard has something to check
    harness
        .db
        .create_task(NewTask {
            agent_id: harness.db.agent_id.clone(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Deploy widget".to_string(),
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
        .unwrap();

    let trace = harness.run("What happened with the deploy?").await.unwrap();

    assert_has_output(&trace);
    // Guard fired: the response should be from the second text_response
    assert_output_contains(&trace, "verify the actual state");
    // Should be 2 steps: rejected first response + accepted second
    assert_exact_steps(&trace, 2);
}

// ---------------------------------------------------------------------------
// Test 6: Callback context exemption — send_message + write tools compose.
//
// When the user message starts with [callback:], the boundary guard does NOT
// fire. This allows callback turns to combine send_message (notification)
// with write tools (status updates, dispatch).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn callback_context_exempts_boundary_guard() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: update_task_status
            tool_call_response(
                "update_task_status",
                json!({"task_id": "task-1", "status": "completed"}),
            ),
            // Step 2: send_message (notification)
            tool_call_response("send_message", json!({"text": "Task completed."})),
            // Step 3: another tool (dispatch) — should NOT be blocked
            tool_call_response("search_memory", json!({"query": "next task"})),
            // Step 4: EndTurn text
            text_response("Callback processed."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[callback: long_running:run_claude_pilot] Task completed.")
        .await
        .unwrap();

    assert_has_output(&trace);
    // All tools should have executed (callback exempt from boundary)
    assert_tools_include(
        &trace,
        &["update_task_status", "send_message", "search_memory"],
    );
    assert_exact_steps(&trace, 4);
}
