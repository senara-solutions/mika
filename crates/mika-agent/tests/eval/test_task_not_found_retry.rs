//! Integration tests: task_not_found retry via list_tasks lookup (#693).
//!
//! Verifies that when `update_task_status` returns `task_not_found` (e.g., due
//! to a hallucinated task UUID), the agent calls `list_tasks` to discover the
//! correct task ID and retries `update_task_status` with the recovered ID.

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

/// Helper: insert a manual task with a reference_url and return its ID.
async fn seed_task_with_ref(harness: &EvalHarness, label: &str, reference_url: &str) -> String {
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
            reference_url: Some(reference_url.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .unwrap()
}

/// Agent recovers from task_not_found by calling list_tasks and retrying
/// with the correct task ID.
///
/// Scenario: LLM hallucinates a task UUID suffix. First `update_task_status`
/// returns `task_not_found`. Agent calls `list_tasks` to find the correct ID
/// by matching `reference_url`, then retries `update_task_status` successfully.
#[tokio::test]
async fn task_not_found_triggers_list_and_retry() {
    // Build harness with placeholder responses — we'll replace after seeding
    let harness = EvalHarness::builder()
        .responses(vec![
            // Placeholder: will be replaced with real IDs
            tool_call_response(
                "update_task_status",
                json!({"task_id": "placeholder", "status": "completed"}),
            ),
            text_response("placeholder"),
        ])
        .tools(tools_with_update_task_status())
        .build()
        .await
        .unwrap();

    // Seed the target task with a known reference_url
    let real_task_id = seed_task_with_ref(
        &harness,
        "mika issue#677",
        "https://github.com/senara-solutions/mika/issues/677",
    )
    .await;
    harness
        .db
        .update_manual_task_status(&real_task_id, "in_progress")
        .await
        .unwrap();

    // Seed a noise task (different reference_url) to verify the agent
    // matches by reference_url, not just picking the first task
    let _noise_task_id = seed_task_with_ref(
        &harness,
        "mika issue#500",
        "https://github.com/senara-solutions/mika/issues/500",
    )
    .await;
    harness
        .db
        .update_manual_task_status(&_noise_task_id, "in_progress")
        .await
        .unwrap();

    // Fabricate a wrong UUID (same first 8 chars, different suffix)
    let wrong_id = format!(
        "{}-0000-0000-0000-000000000000",
        mika_common::text::safe_truncate(&real_task_id, 8)
    );

    // Mock LLM sequence:
    // 1. Call update_task_status with wrong ID → tool returns task_not_found
    // 2. Call list_tasks to find the correct task
    // 3. Call update_task_status with the correct ID → success
    // 4. End turn with summary text
    harness.mock().clear_and_set(vec![
        tool_call_response(
            "update_task_status",
            json!({
                "task_id": &wrong_id,
                "status": "completed",
                "metadata": {"pr_url": "https://github.com/senara-solutions/mika/pull/685"}
            }),
        ),
        tool_call_response("list_tasks", json!({"status": "in_progress"})),
        tool_call_response(
            "update_task_status",
            json!({
                "task_id": &real_task_id,
                "status": "completed",
                "metadata": {"pr_url": "https://github.com/senara-solutions/mika/pull/685"}
            }),
        ),
        text_response(
            "Task status updated to completed after recovering the correct task ID via list_tasks.",
        ),
    ]);

    let trace = harness
        .run("PR mika#685 merged for mika#677. Update task to completed.")
        .await
        .unwrap();

    assert_has_output(&trace);

    // The key assertion: the tool sequence must be
    // update_task_status (fail) → list_tasks → update_task_status (success)
    assert_tool_order(
        &trace,
        &["update_task_status", "list_tasks", "update_task_status"],
    );

    // First update_task_status should have returned task_not_found
    assert_tool_output_contains(&trace, "update_task_status", 0, "task_not_found");

    // Second update_task_status should have used the correct (recovered) task_id
    assert_tool_args_contain(
        &trace,
        "update_task_status",
        1,
        json!({"task_id": &real_task_id}),
    );

    // And it should have succeeded (no task_not_found error)
    let second_calls: Vec<_> = trace.calls_for_tool("update_task_status");
    assert!(
        second_calls.len() >= 2,
        "Expected at least 2 update_task_status calls, got {}",
        second_calls.len()
    );
    let second_output = second_calls[1].output.as_deref().unwrap_or("");
    assert!(
        !second_output.contains("task_not_found"),
        "Second update_task_status should have succeeded, but got: {}",
        mika_common::text::safe_truncate(second_output, 300)
    );
}
