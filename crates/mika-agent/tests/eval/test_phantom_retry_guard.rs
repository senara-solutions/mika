//! Integration tests: phantom retry guard (#579).
//!
//! Verifies that the agent loop's `update_task_status` tool rejects
//! retry-semantic metadata writes when the task has an active callback
//! child task (i.e., a dispatch is still running).

use mika_agent::db::NewTask;
use mika_agent::tools::{default_tools, update_task_status::UpdateTaskStatusTool};
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// Build a tool registry that includes `update_task_status`.
fn tools_with_update_task_status() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(UpdateTaskStatusTool));
    tools
}

/// Helper: insert a manual task and return its ID.
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
            created_by_session: Some("eval-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
        })
        .await
        .unwrap()
}

/// Helper: create a callback child task for a task.
async fn create_callback_child(harness: &EvalHarness, parent_id: &str, status: &str) -> String {
    let child_id = harness
        .db
        .create_task(NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.to_string()),
            depth: 1,
            label: "run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("eval-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
        })
        .await
        .unwrap();

    if status != "pending" {
        harness
            .db
            .update_task_status(&child_id, status)
            .await
            .unwrap();
    }
    child_id
}

/// Guard rejects retry metadata when active callback child exists.
/// Simulates the phantom retry: LLM calls update_task_status with
/// pipeline_retry_count while a dispatch is still running.
#[tokio::test]
async fn phantom_retry_rejected_during_active_dispatch() {
    // We need to know the task ID for the mock response, but the ID is
    // generated at DB insert time. Use a placeholder UUID — the mock response
    // includes the task_id in the tool call, but we need the real ID from the DB.
    // Solution: build harness first, seed data, then use mock_provider's
    // `push_response` to add responses dynamically.

    // Build harness with a dummy response first — we'll replace it
    let harness = EvalHarness::builder()
        .responses(vec![
            // Placeholder — will be consumed by the agent loop
            tool_call_response(
                "update_task_status",
                json!({"task_id": "placeholder", "status": "in_progress", "metadata": {"pipeline_retry_count": 1}}),
            ),
            text_response("The dispatch is still running. I'll wait for the callback."),
        ])
        .tools(tools_with_update_task_status())
        .build()
        .await
        .unwrap();

    // Seed the task and active callback in THIS harness's DB
    let task_id = seed_task(&harness, "Implement feature #334").await;
    harness
        .db
        .update_manual_task_status(&task_id, "in_progress")
        .await
        .unwrap();
    create_callback_child(&harness, &task_id, "pending").await;

    // Replace the mock responses with ones that use the real task ID
    harness.mock_provider.clear_and_set(vec![
        tool_call_response(
            "update_task_status",
            json!({
                "task_id": &task_id,
                "status": "in_progress",
                "metadata": {"pipeline_retry_count": 1}
            }),
        ),
        text_response("The dispatch is still running. I'll wait for the callback."),
    ]);

    let trace = harness
        .run("Pipeline produced no commits for mika#334 — retrying")
        .await
        .unwrap();

    assert_has_output(&trace);
    // Tool was called and returned the guard's error
    assert_tools_include(&trace, &["update_task_status"]);
    assert_tool_output_contains(
        &trace,
        "update_task_status",
        0,
        "retry_metadata_rejected_active_dispatch",
    );
}

/// Guard allows retry metadata when callback child is completed.
#[tokio::test]
async fn retry_metadata_allowed_after_callback_completes() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Placeholder
            tool_call_response(
                "update_task_status",
                json!({"task_id": "placeholder", "status": "in_progress", "metadata": {"pipeline_retry_count": 1}}),
            ),
            text_response("Retry count updated. Launching retry."),
        ])
        .tools(tools_with_update_task_status())
        .build()
        .await
        .unwrap();

    // Seed with completed callback child
    let task_id = seed_task(&harness, "Implement feature #335").await;
    harness
        .db
        .update_manual_task_status(&task_id, "in_progress")
        .await
        .unwrap();
    create_callback_child(&harness, &task_id, "completed").await;

    // Replace with real IDs
    harness.mock_provider.clear_and_set(vec![
        tool_call_response(
            "update_task_status",
            json!({
                "task_id": &task_id,
                "status": "in_progress",
                "metadata": {"pipeline_retry_count": 1}
            }),
        ),
        text_response("Retry count updated. Launching retry."),
    ]);

    let trace = harness
        .run("PIPELINE FAILURE: no commits for mika#335")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_tools_include(&trace, &["update_task_status"]);
    // The tool should NOT have returned the guard's error
    assert_output_contains(&trace, "Retry count updated");
}
