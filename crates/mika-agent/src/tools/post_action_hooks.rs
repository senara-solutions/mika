//! Post-action hooks (mika#772) — side-effect-only callbacks that fire after
//! specific tool calls succeed. NOT gating (failure is warn-and-continue).
//! NOT part of the EndTurn post-condition guard chain — see CLAUDE.md.

use serde_json::Value;
use tracing::warn;

use crate::async_db::AsyncDatabase;
use crate::task_state::tasks::Task;

use super::ToolOutput;

/// A post-action hook that fires after a specific tool call succeeds.
struct PostActionHook {
    tool_name: &'static str,
    /// Returns `true` when the hook should fire based on tool input and output.
    fires_on: fn(&Value, &ToolOutput) -> bool,
}

/// Registry of post-action hooks.
const POST_ACTION_HOOKS: &[PostActionHook] = &[PostActionHook {
    tool_name: "update_task_status",
    fires_on: is_task_completion,
}];

/// Run all matching post-action hooks for a given tool call.
///
/// Called after a tool execution succeeds, before the tool result is returned
/// to the LLM. Hook failures are logged and swallowed — they never affect the
/// tool's own result.
pub(crate) async fn run_post_action_hooks(
    tool_name: &str,
    input: &Value,
    output: &ToolOutput,
    db: &AsyncDatabase,
) {
    for hook in POST_ACTION_HOOKS
        .iter()
        .filter(|h| h.tool_name == tool_name)
    {
        if (hook.fires_on)(input, output) {
            emit_auto_fact_on_task_completion(input, db).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Predicate: is_task_completion (U2)
// ---------------------------------------------------------------------------

/// Returns `true` when `update_task_status` succeeded with `status="completed"`.
///
/// The milestone-parent exclusion happens inside the action (U3) where the full
/// task row is available. Keeping the predicate cheap — no DB calls.
fn is_task_completion(input: &Value, output: &ToolOutput) -> bool {
    if output.is_error {
        return false;
    }
    input.get("status").and_then(|v| v.as_str()) == Some("completed")
}

// ---------------------------------------------------------------------------
// Action: emit_auto_fact_on_task_completion (U3)
// ---------------------------------------------------------------------------

/// Emits a `store_fact(category="event")` for a completed task.
///
/// Uses the same DB write path as the `store_fact` tool (`db.add_event`),
/// NOT by re-dispatching through ToolRegistry (avoids recursive dispatch +
/// audit_events double-counting).
async fn emit_auto_fact_on_task_completion(input: &Value, db: &AsyncDatabase) {
    let task_id = match input.get("task_id").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return,
    };

    let task = match db.get_task(task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => return,
        Err(e) => {
            warn!(
                event = "auto_fact_task_lookup_failed",
                task_id = %task_id,
                error = %e,
                "post-action auto-fact: failed to fetch task (non-blocking)"
            );
            return;
        }
    };

    // Milestone/project parent exclusion (iteration 1 scope)
    if task.r#type == "milestone" || task.r#type == "project" {
        return;
    }

    let description = build_completion_fact_description(&task);

    if let Err(e) = db.add_event(&description, None, None).await {
        warn!(
            event = "auto_fact_emission_failed",
            task_id = %task_id,
            error = %e,
            "post-action auto-fact emission failed (non-blocking)"
        );
    }
}

/// Build a structured fact description from completed task metadata.
fn build_completion_fact_description(task: &Task) -> String {
    let repo_issue = task.reference_url.as_deref().unwrap_or("(no ref)");
    let title = &task.label;

    // Parse metadata JSON for claude_pilot fields
    let metadata: Option<Value> = task
        .metadata
        .as_deref()
        .and_then(|m| serde_json::from_str(m).ok());

    let pr = metadata
        .as_ref()
        .and_then(|m| m.pointer("/claude_pilot/pr_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("\u{2014}");

    let turns = metadata
        .as_ref()
        .and_then(|m| m.pointer("/claude_pilot/turns"))
        .and_then(|v| v.as_u64())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "\u{2014}".to_string());

    let cost = metadata
        .as_ref()
        .and_then(|m| m.pointer("/claude_pilot/cost_usd"))
        .and_then(|v| v.as_f64())
        .map(|n| format!("${n:.4}"))
        .unwrap_or_else(|| "\u{2014}".to_string());

    let task_id_short = &task.id[..8.min(task.id.len())];

    format!(
        "Completed {repo_issue}: {title}. PR: {pr}. Turns: {turns}. Cost: {cost}. Task: {task_id_short}."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    // -- U1: Hook registry tests --

    #[test]
    fn hook_registry_has_update_task_status_entry() {
        assert_eq!(POST_ACTION_HOOKS.len(), 1);
        assert_eq!(POST_ACTION_HOOKS[0].tool_name, "update_task_status");
    }

    #[tokio::test]
    async fn run_post_action_hooks_noop_for_unmatched_tool() {
        let harness = TestHarness::new();
        let input = serde_json::json!({"status": "completed"});
        let output = ToolOutput::success("done");

        // Should not panic or error — no hooks match "read_agent_file"
        run_post_action_hooks("read_agent_file", &input, &output, &harness.db).await;
    }

    // -- U2: Predicate tests --

    #[test]
    fn predicate_fires_on_completed_status() {
        let input = serde_json::json!({"task_id": "abc", "status": "completed"});
        let output = ToolOutput::success("Status updated");
        assert!(is_task_completion(&input, &output));
    }

    #[test]
    fn predicate_does_not_fire_on_blocked_status() {
        let input = serde_json::json!({"task_id": "abc", "status": "blocked"});
        let output = ToolOutput::success("Status updated");
        assert!(!is_task_completion(&input, &output));
    }

    #[test]
    fn predicate_does_not_fire_on_tool_error() {
        let input = serde_json::json!({"task_id": "abc", "status": "completed"});
        let output = ToolOutput::error("Invalid transition");
        assert!(!is_task_completion(&input, &output));
    }

    #[test]
    fn predicate_does_not_fire_on_missing_status() {
        let input = serde_json::json!({"task_id": "abc"});
        let output = ToolOutput::success("done");
        assert!(!is_task_completion(&input, &output));
    }

    // -- U3: Action and fact description tests --

    #[test]
    fn build_description_with_metadata() {
        let task = Task {
            id: "abcdef1234567890".to_string(),
            agent_id: "test".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Fix login bug".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            status: "completed".to_string(),
            process_id: None,
            input_context: None,
            result: None,
            created_by_session: None,
            created_trace_id: None,
            execution_trace_id: None,
            created_at: "2026-06-13T00:00:00Z".to_string(),
            updated_at: "2026-06-13T00:00:00Z".to_string(),
            fired_at: None,
            completed_at: None,
            reference_url: Some("https://github.com/senara-solutions/mika/issues/100".to_string()),
            source: None,
            metadata: Some(
                serde_json::json!({
                    "claude_pilot": {
                        "pr_url": "https://github.com/senara-solutions/mika/pull/101",
                        "turns": 5,
                        "cost_usd": 0.1234
                    }
                })
                .to_string(),
            ),
            r#type: "issue".to_string(),
            dispatch_class: None,
        };

        let desc = build_completion_fact_description(&task);
        assert!(desc.contains("mika/issues/100"));
        assert!(desc.contains("Fix login bug"));
        assert!(desc.contains("mika/pull/101"));
        assert!(desc.contains("Turns: 5"));
        assert!(desc.contains("$0.1234"));
        assert!(desc.contains("abcdef12"));
    }

    #[test]
    fn build_description_without_metadata() {
        let task = Task {
            id: "abcdef1234567890".to_string(),
            agent_id: "test".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Manual task".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            status: "completed".to_string(),
            process_id: None,
            input_context: None,
            result: None,
            created_by_session: None,
            created_trace_id: None,
            execution_trace_id: None,
            created_at: "2026-06-13T00:00:00Z".to_string(),
            updated_at: "2026-06-13T00:00:00Z".to_string(),
            fired_at: None,
            completed_at: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: "issue".to_string(),
            dispatch_class: None,
        };

        let desc = build_completion_fact_description(&task);
        assert!(desc.contains("(no ref)"));
        assert!(desc.contains("Manual task"));
        assert!(desc.contains("\u{2014}")); // em dash for missing fields
    }

    fn test_new_task(label: &str, r#type: Option<&str>) -> crate::db::NewTask {
        use crate::task_engine::types::{action_type, trigger_type};
        crate::db::NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: trigger_type::MANUAL.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::NONE.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: r#type.map(|t| t.to_string()),
            dispatch_class: None,
        }
    }

    #[tokio::test]
    async fn milestone_task_does_not_emit_fact() {
        let harness = TestHarness::new();

        let task_id = harness
            .db
            .create_task(test_new_task("Milestone parent", Some("milestone")))
            .await
            .unwrap();

        let input = serde_json::json!({
            "task_id": task_id,
            "status": "completed"
        });

        emit_auto_fact_on_task_completion(&input, &harness.db).await;

        let events = harness.db.list_events().await.unwrap();
        assert!(
            events.is_empty(),
            "milestone tasks should not emit auto-facts"
        );
    }

    #[tokio::test]
    async fn issue_task_emits_fact() {
        let harness = TestHarness::new();

        let mut new_task = test_new_task("Fix bug #100", Some("issue"));
        new_task.reference_url =
            Some("https://github.com/senara-solutions/mika/issues/100".to_string());

        let task_id = harness.db.create_task(new_task).await.unwrap();

        let input = serde_json::json!({
            "task_id": task_id,
            "status": "completed"
        });

        emit_auto_fact_on_task_completion(&input, &harness.db).await;

        let events = harness.db.list_events().await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].description.contains("Fix bug #100"));
        assert!(events[0].description.contains("mika/issues/100"));
    }

    #[tokio::test]
    async fn nonexistent_task_does_not_panic() {
        let harness = TestHarness::new();
        let input = serde_json::json!({
            "task_id": "nonexistent-id",
            "status": "completed"
        });

        // Should return silently — no panic, no error
        emit_auto_fact_on_task_completion(&input, &harness.db).await;
    }

    #[tokio::test]
    async fn missing_task_id_in_input_does_not_panic() {
        let harness = TestHarness::new();
        let input = serde_json::json!({"status": "completed"});

        emit_auto_fact_on_task_completion(&input, &harness.db).await;
    }
}
