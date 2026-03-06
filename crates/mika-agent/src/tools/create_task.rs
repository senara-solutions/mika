use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::db::NewTask;
use crate::task_engine::types::action_type;
use crate::task_engine::types::trigger_type as tt;

/// Agent tool for creating scheduled/callback tasks.
///
/// Removed from `default_tools()` — long-running skills auto-create callback tasks.
/// Retained for tests (e.g. `complete_task::tests`).
#[cfg_attr(not(test), allow(dead_code))]
pub struct CreateTaskTool;

#[async_trait]
impl Tool for CreateTaskTool {
    fn name(&self) -> &str {
        "create_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_task".to_string(),
            description: "Create a scheduled or callback-triggered task. \
                Use trigger_type='time' for one-shot tasks (requires fire_at), \
                'recurring' for cron-scheduled tasks (requires cron_expr), \
                or 'callback' for tasks that fire when an external process calls \
                POST /tasks/{id}/complete. action_type controls what happens: \
                'send_message' sends a message (action_config: {\"text\":\"...\"}), \
                'run_skill' runs a skill (action_config: {\"skill_name\":\"...\",\"args\":{}}), \
                'inject_context' injects text into the next agent turn \
                (action_config: {\"context\":\"...\"}), \
                'resume_agent' re-runs the agent with the callback result as context."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "label": {
                        "type": "string",
                        "description": "Human-readable label for the task"
                    },
                    "trigger_type": {
                        "type": "string",
                        "description": "When the task fires: 'time' (one-shot, requires fire_at), 'recurring' (cron, requires cron_expr), or 'callback' (fired by external POST /tasks/{id}/complete)",
                        "enum": ["time", "recurring", "callback"]
                    },
                    "action_type": {
                        "type": "string",
                        "description": "What to do when the task fires",
                        "enum": ["send_message", "run_skill", "inject_context", "resume_agent"]
                    },
                    "action_config": {
                        "type": "string",
                        "description": "JSON string with action parameters. Defaults to '{}' if omitted."
                    },
                    "fire_at": {
                        "type": "string",
                        "description": "ISO 8601 datetime (UTC). Required for trigger_type='time'."
                    },
                    "cron_expr": {
                        "type": "string",
                        "description": "6-field cron expression (seconds field first). Required for trigger_type='recurring'. Example: '0 0 9 * * *' fires at 9am UTC daily."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Optional. Task expires if not completed within this many seconds from creation."
                    }
                },
                "required": ["label", "trigger_type", "action_type"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let label = input["label"].as_str().unwrap_or("").trim();
        let trigger_type_str = input["trigger_type"].as_str().unwrap_or("").trim();
        let action_type_val = input["action_type"].as_str().unwrap_or("").trim();

        if label.is_empty() {
            return Ok(ToolOutput::error("'label' is required."));
        }
        if label.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'label' too long: {} characters (max: {MAX_INPUT_LEN})",
                label.len()
            )));
        }
        if trigger_type_str.is_empty() {
            return Ok(ToolOutput::error("'trigger_type' is required."));
        }
        if action_type_val.is_empty() {
            return Ok(ToolOutput::error("'action_type' is required."));
        }

        match trigger_type_str {
            tt::TIME | tt::RECURRING | tt::CALLBACK => {}
            other => {
                return Ok(ToolOutput::error(format!(
                    "Invalid trigger_type '{other}'. Must be one of: time, recurring, callback"
                )));
            }
        }

        match action_type_val {
            action_type::SEND_MESSAGE
            | action_type::RUN_SKILL
            | action_type::INJECT_CONTEXT
            | action_type::RESUME_AGENT => {}
            other => {
                return Ok(ToolOutput::error(format!(
                    "Invalid action_type '{other}'. Must be one of: send_message, run_skill, inject_context, resume_agent"
                )));
            }
        }

        let action_config_str = input["action_config"].as_str().unwrap_or("{}");
        if action_config_str.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'action_config' too long: {} characters (max: {MAX_INPUT_LEN})",
                action_config_str.len()
            )));
        }
        if serde_json::from_str::<serde_json::Value>(action_config_str).is_err() {
            return Ok(ToolOutput::error(
                "'action_config' must be a valid JSON string.",
            ));
        }

        let (next_fire_at, cron_expr) = match trigger_type_str {
            tt::TIME => {
                let fire_at_str = input["fire_at"].as_str().unwrap_or("");
                if fire_at_str.is_empty() {
                    return Ok(ToolOutput::error(
                        "'fire_at' is required for trigger_type='time'.",
                    ));
                }
                if fire_at_str.len() > 64 {
                    return Ok(ToolOutput::error("'fire_at' is too long."));
                }
                let parsed = match chrono::DateTime::parse_from_rfc3339(fire_at_str) {
                    Ok(dt) => dt.with_timezone(&Utc),
                    Err(_) => {
                        return Ok(ToolOutput::error(
                            "Invalid 'fire_at'. Use ISO 8601 format like '2026-02-25T15:00:00Z'.",
                        ));
                    }
                };
                if parsed <= Utc::now() {
                    return Ok(ToolOutput::error("'fire_at' must be in the future."));
                }
                (Some(parsed.timestamp()), None)
            }
            tt::RECURRING => {
                let cron = input["cron_expr"].as_str().unwrap_or("").trim();
                if cron.is_empty() {
                    return Ok(ToolOutput::error(
                        "'cron_expr' is required for trigger_type='recurring'.",
                    ));
                }
                if cron.len() > 128 {
                    return Ok(ToolOutput::error("'cron_expr' is too long."));
                }
                match crate::task_engine::cron::next_fire_from_cron(cron, Utc::now().timestamp()) {
                    Ok(ts) => (Some(ts), Some(cron.to_string())),
                    Err(_) => {
                        return Ok(ToolOutput::error(
                            "Invalid 'cron_expr'. Use a 6-field cron expression (seconds field first).",
                        ));
                    }
                }
            }
            _ => (None, None), // callback: no next_fire_at
        };

        const MAX_TIMEOUT_SECS: i64 = 7_776_000; // 90 days

        let timeout_at = if let Some(secs) = input["timeout_secs"].as_i64() {
            if secs <= 0 {
                return Ok(ToolOutput::error(
                    "'timeout_secs' must be a positive integer.",
                ));
            }
            if secs > MAX_TIMEOUT_SECS {
                return Ok(ToolOutput::error(format!(
                    "'timeout_secs' cannot exceed {} seconds (90 days).",
                    MAX_TIMEOUT_SECS
                )));
            }
            Some(Utc::now().timestamp() + secs)
        } else {
            None
        };

        let task = NewTask {
            agent_id: ctx.db.agent_id.clone(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: trigger_type_str.to_string(),
            cron_expr,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at,
            timeout_at,
            action_type: action_type_val.to_string(),
            action_config: action_config_str.to_string(),
            input_context: None,
            created_by_session: Some(ctx.session_id.to_string()),
        };

        let id = ctx.db.create_task(task).await?;

        ctx.db
            .log_memory_event(
                ctx.session_id,
                "create_task",
                &format!("task:{id}"),
                None,
                &format!("{trigger_type_str}/{action_type_val} — {label}"),
                None,
            )
            .await?;

        Ok(ToolOutput::success(format!(
            "Task {id} created ({trigger_type_str}/{action_type_val})."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_create_task_callback_resume_agent() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTaskTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Analyze codebase",
                    "trigger_type": "callback",
                    "action_type": "resume_agent",
                    "action_config": "{}"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("created"));
        assert!(result.content.contains("callback/resume_agent"));
    }

    #[tokio::test]
    async fn test_create_task_time_send_message() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTaskTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Daily standup reminder",
                    "trigger_type": "time",
                    "action_type": "send_message",
                    "action_config": "{\"text\":\"Time for standup!\"}",
                    "fire_at": "2099-12-31T09:00:00Z"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("time/send_message"));
    }

    #[tokio::test]
    async fn test_create_task_time_requires_fire_at() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTaskTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Missing fire_at",
                    "trigger_type": "time",
                    "action_type": "send_message"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("fire_at"));
    }

    #[tokio::test]
    async fn test_create_task_recurring_requires_cron() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTaskTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Missing cron",
                    "trigger_type": "recurring",
                    "action_type": "run_skill"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("cron_expr"));
    }

    #[tokio::test]
    async fn test_create_task_invalid_action_type() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTaskTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Bad action",
                    "trigger_type": "callback",
                    "action_type": "explode"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid action_type"));
    }

    #[tokio::test]
    async fn test_create_task_invalid_trigger_type() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTaskTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Bad trigger",
                    "trigger_type": "magic",
                    "action_type": "send_message"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid trigger_type"));
    }

    #[tokio::test]
    async fn test_create_task_with_timeout() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTaskTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Timed callback",
                    "trigger_type": "callback",
                    "action_type": "resume_agent",
                    "timeout_secs": 3600
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);

        // Verify task was stored
        let tasks = harness
            .db
            .get_tasks_by_status(vec!["pending".to_string()])
            .await
            .unwrap();
        let task = tasks.iter().find(|t| t.label == "Timed callback").unwrap();
        assert!(task.timeout_at.is_some());
    }

    #[tokio::test]
    async fn test_create_task_timeout_exceeds_max() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTaskTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Too long timeout",
                    "trigger_type": "callback",
                    "action_type": "resume_agent",
                    "timeout_secs": 99_999_999
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("90 days"));
    }

    #[tokio::test]
    async fn test_create_task_invalid_json_action_config() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTaskTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Bad config",
                    "trigger_type": "callback",
                    "action_type": "resume_agent",
                    "action_config": "not json"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("valid JSON"));
    }
}
