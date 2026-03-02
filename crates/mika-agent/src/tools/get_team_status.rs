use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::team;
use serde_json::Value;
use std::fmt::Write;
use std::path::PathBuf;

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
            description: "Get the status of a team's most recent run, or a specific run by ID. Shows status, goal, iteration, timestamps, and task summary.".to_string(),
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

        let history_dir = team::history_dir(&self.home_dir, team_name);
        let run_id_filter = input["run_id"].as_str().filter(|s| !s.is_empty());
        if let Some(id) = run_id_filter
            && id.len() > MAX_INPUT_LEN
        {
            return Ok(ToolOutput::error(format!(
                "'run_id' too long: {} characters (max: {MAX_INPUT_LEN})",
                id.len()
            )));
        }

        let run = if let Some(target_id) = run_id_filter {
            // Find specific run by ID
            match crate::teams::history::list_runs(&history_dir) {
                Ok(runs) => runs.into_iter().find(|r| r.run_id == target_id),
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to load runs for team '{team_name}': {e}"
                    )));
                }
            }
        } else {
            // Load most recent
            match crate::teams::history::load_latest_run(&history_dir) {
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
        writeln!(out, "Team: {}", run.team_name).unwrap();
        writeln!(out, "Run ID: {}", run.run_id).unwrap();
        writeln!(out, "Status: {}", run.status).unwrap();
        writeln!(out, "Goal: {}", run.goal).unwrap();
        writeln!(out, "Iteration: {}/{}", run.iteration, run.max_iterations).unwrap();
        writeln!(out, "Started: {}", run.started_at).unwrap();
        if let Some(ref ended) = run.ended_at {
            writeln!(out, "Ended: {ended}").unwrap();
        }

        if !run.tasks.is_empty() {
            writeln!(out, "\nTasks:").unwrap();
            for task in &run.tasks {
                writeln!(
                    out,
                    "  - [{}] {} ({}): {}",
                    task.status, task.agent, task.role, task.task
                )
                .unwrap();
            }
        }

        if let Some(ref deliverable) = run.deliverable {
            let preview = if deliverable.len() > 500 {
                // Find a valid UTF-8 char boundary near 500 bytes
                let mut boundary = 500;
                while boundary > 0 && !deliverable.is_char_boundary(boundary) {
                    boundary -= 1;
                }
                format!("{}...", &deliverable[..boundary])
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
    use crate::teams::history::save_run;
    use crate::test_utils::test_helpers::{TestHarness, test_team_run};

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
        let tmp = tempfile::tempdir().unwrap();
        let history = team::history_dir(tmp.path(), "dev-team");
        save_run(&history, &test_team_run()).unwrap();

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
        assert!(result.content.contains("Status: completed"));
        assert!(result.content.contains("abcd1234"));
        assert!(result.content.contains("Test goal"));
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
