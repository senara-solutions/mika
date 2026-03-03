use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::team;
use serde_json::Value;
use std::fmt::Write;
use std::path::PathBuf;

use crate::db::{TeamRunRow, format_unix_ts};
use crate::teams::{TeamDbError, open_team_db};

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct GetTeamStatusTool {
    pub home_dir: PathBuf,
}

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

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
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

        // Open team DB
        let team_db = match open_team_db(&self.home_dir, team_name) {
            Ok(db) => db,
            Err(TeamDbError::NoRuns(msg)) => return Ok(ToolOutput::success(msg)),
            Err(TeamDbError::OpenFailed(msg)) => return Ok(ToolOutput::error(msg)),
        };

        let run: Option<TeamRunRow> = if let Some(target_id) = run_id_filter {
            match team_db.load_team_run_by_id(target_id).await {
                Ok(run) => run,
                Err(e) => {
                    team_db.shutdown();
                    return Ok(ToolOutput::error(format!(
                        "Failed to load run '{target_id}' for team '{team_name}': {e}"
                    )));
                }
            }
        } else {
            match team_db.load_latest_team_run(team_name).await {
                Ok(run) => run,
                Err(e) => {
                    team_db.shutdown();
                    return Ok(ToolOutput::error(format!(
                        "Failed to load status for team '{team_name}': {e}"
                    )));
                }
            }
        };

        let Some(run) = run else {
            team_db.shutdown();
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
        if let Ok(messages) = team_db.load_team_messages(&run.id).await
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
                    msg.message_type, msg.iteration, preview
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

        team_db.shutdown();
        Ok(ToolOutput::success(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::{TestHarness, setup_team_db};

    #[tokio::test]
    async fn test_get_team_status_no_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamStatusTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(serde_json::json!({"team_name": "dev-team"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No runs found"));
    }

    #[tokio::test]
    async fn test_get_team_status_latest() {
        let (_tmp, home) = setup_team_db("dev-team", 1);

        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamStatusTool { home_dir: home };

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
        let tool = GetTeamStatusTool {
            home_dir: PathBuf::from("/tmp"),
        };

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'team_name' is required"));
    }

    #[tokio::test]
    async fn test_get_team_status_invalid_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamStatusTool {
            home_dir: PathBuf::from("/tmp"),
        };

        let result = tool
            .execute(serde_json::json!({"team_name": "INVALID"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid team name"));
    }
}
