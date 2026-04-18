//! Integration tests: completion-claim guard (#483).
//!
//! Verifies that the agent loop rejects EndTurn responses containing
//! completion-claim keywords when `update_task_status` was not called
//! and active tasks exist.

use async_trait::async_trait;
use mika_agent::db::NewTask;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// Stub tool that mimics `update_task_status` for the guard's registry check.
/// The guard only checks `tools.get("update_task_status").is_some()` —
/// it never actually dispatches to it, so a minimal stub is sufficient.
struct StubUpdateTaskStatusTool;

#[async_trait]
impl Tool for StubUpdateTaskStatusTool {
    fn name(&self) -> &str {
        "update_task_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_task_status".to_string(),
            description: "Stub for completion-claim guard tests".to_string(),
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

/// Helper: insert a manual task in "pending" status for the test agent.
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
        })
        .await
        .unwrap()
}

/// Helper: set a task's status to the given value.
async fn set_task_status(harness: &EvalHarness, task_id: &str, status: &str) {
    harness
        .db
        .update_manual_task_status(task_id, status)
        .await
        .unwrap();
}

/// Build a tool registry that includes `update_task_status` (orchestrator tool).
fn tools_with_update_task_status() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools
}

/// Guard fires when agent says "merged" without calling update_task_status,
/// and active tasks exist.
#[tokio::test]
async fn guard_fires_on_fabricated_completion() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Fabricated completion claim — guard should reject this
            text_response("PR merged successfully. Main synced."),
            // Step 2: After re-prompt, agent calls update_task_status
            // then gives a proper response (we just test the re-prompt happened)
            text_response("Let me verify the actual state first."),
        ])
        .tools(tools_with_update_task_status())
        .build()
        .await
        .unwrap();

    // Seed an active task
    seed_task(&harness, "Implement feature #480").await;

    let trace = harness.run("The QA bot approved PR #482").await.unwrap();

    assert_has_output(&trace);
    // The guard should have caused a re-prompt, resulting in 2 LLM calls
    assert_exact_steps(&trace, 2);
    // Final output should be the second response (after re-prompt)
    assert_output_contains(&trace, "verify");
}

/// Guard does NOT fire when there are no active tasks.
#[tokio::test]
async fn guard_skips_when_no_active_items() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Task completed successfully.")])
        .tools(tools_with_update_task_status())
        .build()
        .await
        .unwrap();

    // No tasks seeded — guard should not fire
    let trace = harness.run("How did the task go?").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "completed");
    // Only 1 step — no re-prompt
    assert_exact_steps(&trace, 1);
}

/// Guard does NOT fire when update_task_status is not in the tool registry
/// (e.g., delegate agents with default_tools() only).
#[tokio::test]
async fn guard_skips_when_tool_not_registered() {
    // Use default_tools() which does NOT include update_task_status
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Task completed. Everything is deployed.",
        )])
        .build() // default_tools() — no update_task_status
        .await
        .unwrap();

    // Seed a task anyway — guard should still not fire because tool isn't registered
    seed_task(&harness, "Some task").await;

    let trace = harness.run("Status update?").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "deployed");
    assert_exact_steps(&trace, 1);
}

/// Guard does NOT fire when update_task_status was called in the turn.
#[tokio::test]
async fn guard_skips_when_tool_was_called() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls update_task_status
            tool_call_response(
                "update_task_status",
                json!({"task_id": "test-id", "status": "completed"}),
            ),
            // Step 2: Agent produces completion text
            text_response("Task completed and status updated."),
        ])
        .tools(tools_with_update_task_status())
        .build()
        .await
        .unwrap();

    // Seed task
    seed_task(&harness, "Feature work").await;

    let trace = harness.run("Finish up the task").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "completed");
    // 2 steps: tool call + final response (no extra re-prompt step)
    assert_exact_steps(&trace, 2);
}

/// Guard does NOT fire when there are no completion-claim keywords.
#[tokio::test]
async fn guard_skips_on_no_keywords() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "I've analyzed the code and found several issues to address.",
        )])
        .tools(tools_with_update_task_status())
        .build()
        .await
        .unwrap();

    seed_task(&harness, "Code review task").await;

    let trace = harness.run("Review the code").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "analyzed");
    assert_exact_steps(&trace, 1);
}

/// Guard fires only once — if the retry also fabricates, it passes through.
#[tokio::test]
async fn guard_fires_once_only() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Fabricated — guard fires
            text_response("PR merged and deployed to production."),
            // Step 2: Retry also fabricates — guard does NOT fire again
            text_response("Confirmed: everything is deployed and running."),
        ])
        .tools(tools_with_update_task_status())
        .build()
        .await
        .unwrap();

    seed_task(&harness, "Deploy feature").await;

    let trace = harness.run("What's the status?").await.unwrap();

    assert_has_output(&trace);
    // 2 steps: first rejected + second passes through (even though it also has keywords)
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "deployed");
}

/// Guard does NOT fire for blocked-only tasks (they can't be completed).
#[tokio::test]
async fn guard_skips_when_only_blocked_items() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Investigation complete. The item is blocked on API access.",
        )])
        .tools(tools_with_update_task_status())
        .build()
        .await
        .unwrap();

    let task_id = seed_task(&harness, "Blocked task").await;
    // Transition to in_progress first (required by state machine), then to blocked
    set_task_status(&harness, &task_id, "in_progress").await;
    set_task_status(&harness, &task_id, "blocked").await;

    let trace = harness
        .run("What's the status of the blocked item?")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "complete");
    // No re-prompt — blocked items are filtered out
    assert_exact_steps(&trace, 1);
}
