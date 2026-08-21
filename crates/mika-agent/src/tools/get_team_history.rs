use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::team;
use serde_json::Value;
use std::fmt::Write;

use crate::db::format_ts;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct GetTeamHistoryTool;

#[async_trait]
impl Tool for GetTeamHistoryTool {
    fn name(&self) -> &str {
        "get_team_history"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_team_history".to_string(),
            description:
                "List recent runs for a team. Shows run IDs, status, goals, and timestamps."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "team_name": {
                        "type": "string",
                        "description": "Name of the team to get history for"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of runs to show (default: 5, max: 20)"
                    }
                },
                "required": ["team_name"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let team_name = input["team_name"].as_str().unwrap_or("");
        if team_name.is_empty() {
            return Ok(ToolOutput::error("'team_name' is required."));
        }
        if team_name.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'team_name' too long: {} characters (max: {MAX_INPUT_LEN})",
                team_name.len()
            )));
        }
        if let Err(e) = team::validate_team_name(team_name) {
            return Ok(ToolOutput::error(format!("Invalid team name: {e}")));
        }

        let limit = input["limit"].as_u64().unwrap_or(5).min(20) as usize;

        // Use the shared container DB from ToolContext
        let db = ctx.db;

        let runs = match db.load_team_runs(team_name, limit).await {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "Failed to load history for team '{team_name}': {e}"
                )));
            }
        };

        if runs.is_empty() {
            return Ok(ToolOutput::success(format!(
                "No runs found for team '{team_name}'."
            )));
        }

        let mut out = String::new();
        writeln!(
            out,
            "Recent runs for team '{team_name}' (showing {}):",
            runs.len()
        )
        .unwrap();

        for run in &runs {
            let ended = run
                .ended_at
                .as_ref()
                .map(|s| format_ts(s))
                .unwrap_or_else(|| "in progress".to_string());
            writeln!(
                out,
                "  - [{}] {} | {} | started: {} | ended: {}",
                run.id,
                run.status,
                run.goal,
                format_ts(&run.started_at),
                ended
            )
            .unwrap();
        }

        Ok(ToolOutput::success(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_get_team_history_empty() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamHistoryTool;

        let result = tool
            .execute(serde_json::json!({"team_name": "dev-team"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No runs found"));
    }

    #[tokio::test]
    async fn test_get_team_history_multiple_runs() {
        let harness = TestHarness::new();
        // Seed team runs in shared DB
        let base_times = ["2025-02-19T15:06:40Z", "2025-02-19T15:11:40Z"];
        let end_times = ["2025-02-19T15:07:40Z", "2025-02-19T15:12:40Z"];
        for i in 0..2 {
            let run_id = format!("run-{i:04}");
            harness
                .db
                .insert_team_run(
                    &run_id,
                    "dev-team",
                    &format!("Goal {i}"),
                    3,
                    base_times[i],
                    None,
                )
                .await
                .unwrap();
            harness
                .db
                .update_team_run(
                    &run_id,
                    "completed",
                    None,
                    1,
                    Some("Done"),
                    Some(end_times[i]),
                    0,
                    false,
                    None,
                )
                .await
                .unwrap();
        }

        let ctx = harness.ctx();
        let tool = GetTeamHistoryTool;

        let result = tool
            .execute(serde_json::json!({"team_name": "dev-team"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("showing 2"));
        assert!(result.content.contains("run-0000"));
        assert!(result.content.contains("run-0001"));
    }

    #[tokio::test]
    async fn test_get_team_history_respects_limit() {
        let harness = TestHarness::new();
        let base_times = [
            "2025-02-19T15:06:40Z",
            "2025-02-19T15:11:40Z",
            "2025-02-19T15:16:40Z",
            "2025-02-19T15:21:40Z",
            "2025-02-19T15:26:40Z",
        ];
        let end_times = [
            "2025-02-19T15:07:40Z",
            "2025-02-19T15:12:40Z",
            "2025-02-19T15:17:40Z",
            "2025-02-19T15:22:40Z",
            "2025-02-19T15:27:40Z",
        ];
        for i in 0..5 {
            let run_id = format!("run-{i:04}");
            harness
                .db
                .insert_team_run(
                    &run_id,
                    "dev-team",
                    &format!("Goal {i}"),
                    3,
                    base_times[i],
                    None,
                )
                .await
                .unwrap();
            harness
                .db
                .update_team_run(
                    &run_id,
                    "completed",
                    None,
                    1,
                    Some("Done"),
                    Some(end_times[i]),
                    0,
                    false,
                    None,
                )
                .await
                .unwrap();
        }

        let ctx = harness.ctx();
        let tool = GetTeamHistoryTool;

        let result = tool
            .execute(
                serde_json::json!({"team_name": "dev-team", "limit": 2}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("showing 2"));
    }

    #[tokio::test]
    async fn test_get_team_history_missing_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamHistoryTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'team_name' is required"));
    }
}
