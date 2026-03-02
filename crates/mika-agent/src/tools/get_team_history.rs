use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::team;
use serde_json::Value;
use std::fmt::Write;
use std::path::PathBuf;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct GetTeamHistoryTool {
    pub home_dir: PathBuf,
}

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

        let limit = input["limit"].as_u64().unwrap_or(5).min(20) as usize;

        let history_dir = team::history_dir(&self.home_dir, team_name);
        let runs = match crate::teams::history::list_runs_limited(&history_dir, limit) {
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

        let shown = &runs;
        let mut out = String::new();
        writeln!(
            out,
            "Recent runs for team '{team_name}' (showing {}):",
            shown.len()
        )
        .unwrap();

        for run in shown {
            let ended = run.ended_at.as_deref().unwrap_or("in progress");
            writeln!(
                out,
                "  - [{}] {} | {} | started: {} | ended: {}",
                run.run_id, run.status, run.goal, run.started_at, ended
            )
            .unwrap();
        }

        Ok(ToolOutput::success(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::history::save_run;
    use crate::test_utils::test_helpers::{TestHarness, test_team_run};

    fn test_run(id: &str, date: &str) -> crate::teams::types::TeamRun {
        let mut run = test_team_run();
        run.run_id = id.to_string();
        run.started_at = format!("{date}T10:00:00Z");
        run.ended_at = Some(format!("{date}T10:05:00Z"));
        run
    }

    #[tokio::test]
    async fn test_get_team_history_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamHistoryTool {
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
    async fn test_get_team_history_multiple_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let history = team::history_dir(tmp.path(), "dev-team");

        save_run(&history, &test_run("aaaa1111", "2026-02-24")).unwrap();
        save_run(&history, &test_run("bbbb2222", "2026-02-25")).unwrap();

        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamHistoryTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(serde_json::json!({"team_name": "dev-team"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("showing 2"));
        assert!(result.content.contains("aaaa1111"));
        assert!(result.content.contains("bbbb2222"));
    }

    #[tokio::test]
    async fn test_get_team_history_respects_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let history = team::history_dir(tmp.path(), "dev-team");

        for i in 0..5 {
            save_run(
                &history,
                &test_run(&format!("run{i:04}"), &format!("2026-02-{:02}", 20 + i)),
            )
            .unwrap();
        }

        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamHistoryTool {
            home_dir: tmp.path().to_path_buf(),
        };

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
        let tool = GetTeamHistoryTool {
            home_dir: PathBuf::from("/tmp"),
        };

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'team_name' is required"));
    }
}
