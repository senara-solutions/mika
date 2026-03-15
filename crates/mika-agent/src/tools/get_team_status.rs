use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::team;
use serde_json::Value;
use std::fmt::Write;

use crate::db::{TeamRunRow, format_unix_ts};

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct GetTeamStatusTool;

#[async_trait]
impl Tool for GetTeamStatusTool {
    fn name(&self) -> &str {
        "get_team_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_team_status".to_string(),
            description: "Get the status of a team's most recent run, or a specific run by ID. Shows status, goal, iteration, timestamps, and message graph.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "team_name": {
                        "type": "string",
                        "description": "Name of the team to check status for"
                    },
                    "run_id": {
                        "type": "string",
                        "description": "Optional: specific run ID to look up (defaults to most recent)"
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

        let run_id_filter = input["run_id"].as_str().filter(|s| !s.is_empty());
        if let Some(id) = run_id_filter
            && id.len() > MAX_INPUT_LEN
        {
            return Ok(ToolOutput::error(format!(
                "'run_id' too long: {} characters (max: {MAX_INPUT_LEN})",
                id.len()
            )));
        }

        // Use the shared container DB from ToolContext
        let db = ctx.db;

        let run: Option<TeamRunRow> = if let Some(target_id) = run_id_filter {
            match db.load_team_run_by_id(target_id).await {
                Ok(run) => run,
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to load run '{target_id}' for team '{team_name}': {e}"
                    )));
                }
            }
        } else {
            match db.load_latest_team_run(team_name).await {
                Ok(run) => run,
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to load status for team '{team_name}': {e}"
                    )));
                }
            }
        };

        let Some(run) = run else {
            return Ok(ToolOutput::success(format!(
                "No runs found for team '{team_name}'."
            )));
        };

        let mut out = String::new();
        writeln!(out, "Team: {team_name}").unwrap();
        writeln!(out, "Run ID: {}", run.id).unwrap();
        writeln!(out, "Status: {}", run.status).unwrap();
        writeln!(out, "Goal: {}", run.goal).unwrap();
        writeln!(out, "Iteration: {}/{}", run.iteration, run.max_iterations).unwrap();
        writeln!(out, "Started: {}", format_unix_ts(run.started_at)).unwrap();
        if let Some(ended) = run.ended_at {
            writeln!(out, "Ended: {}", format_unix_ts(ended)).unwrap();
        }

        if let Some(ref reason) = run.failure_reason {
            writeln!(out, "Failure reason: {reason}").unwrap();
        }

        // Load messages for this run
        if let Ok(messages) = db.load_team_workspace(&run.id).await
            && !messages.is_empty()
        {
            writeln!(out, "\nMessages ({}):", messages.len()).unwrap();
            for msg in &messages {
                let agent = msg.agent_name.as_deref().unwrap_or("system");
                let preview = if msg.content.len() > 200 {
                    format!(
                        "{}...",
                        &msg.content[..msg.content.floor_char_boundary(200)]
                    )
                } else {
                    msg.content.clone()
                };
                writeln!(
                    out,
                    "  - [{}] {agent} (iter {}): {}",
                    msg.entry_type, msg.iteration, preview
                )
                .unwrap();
            }
        }

        if let Some(ref deliverable) = run.deliverable {
            let preview = if deliverable.len() > 500 {
                format!(
                    "{}...",
                    &deliverable[..deliverable.floor_char_boundary(500)]
                )
            } else {
                deliverable.clone()
            };
            writeln!(out, "\nDeliverable:\n{preview}").unwrap();
        }

        Ok(ToolOutput::success(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_get_team_status_no_runs() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamStatusTool;

        let result = tool
            .execute(serde_json::json!({"team_name": "dev-team"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No runs found"));
    }

    #[tokio::test]
    async fn test_get_team_status_latest() {
        let harness = TestHarness::new();
        // Seed team run data in the shared DB
        harness
            .db
            .insert_team_run("run-0000", "dev-team", "Goal 0", 3, 1_740_000_000, None)
            .await
            .unwrap();
        harness
            .db
            .update_team_run(
                "run-0000",
                "completed",
                None,
                1,
                Some("Done"),
                Some(1_740_000_060),
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = GetTeamStatusTool;

        let result = tool
            .execute(serde_json::json!({"team_name": "dev-team"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Status: completed"));
        assert!(result.content.contains("run-0000"));
        assert!(result.content.contains("Goal 0"));
    }

    #[tokio::test]
    async fn test_get_team_status_missing_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamStatusTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'team_name' is required"));
    }

    #[tokio::test]
    async fn test_get_team_status_invalid_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamStatusTool;

        let result = tool
            .execute(serde_json::json!({"team_name": "INVALID"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid team name"));
    }
}
