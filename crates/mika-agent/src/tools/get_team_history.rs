use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::team;
use serde_json::Value;
use std::fmt::Write;
use std::path::PathBuf;

use crate::async_db::AsyncDatabase;
use crate::db::{Database, format_unix_ts};

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

        // Open team DB
        let team_data_dir = team::team_dir(&self.home_dir, team_name).join("data");
        if !team_data_dir.exists() {
            return Ok(ToolOutput::success(format!(
                "No runs found for team '{team_name}'."
            )));
        }
        let team_db_path = team_data_dir.join("mika.db");
        let team_db = match Database::open(&team_db_path) {
            Ok(db) => AsyncDatabase::new(db),
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "Failed to open team database: {e}"
                )));
            }
        };

        let runs = match team_db.load_team_runs(team_name, limit).await {
            Ok(r) => r,
            Err(e) => {
                team_db.shutdown();
                return Ok(ToolOutput::error(format!(
                    "Failed to load history for team '{team_name}': {e}"
                )));
            }
        };
        team_db.shutdown();

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
                .map(format_unix_ts)
                .unwrap_or_else(|| "in progress".to_string());
            writeln!(
                out,
                "  - [{}] {} | {} | started: {} | ended: {}",
                run.id,
                run.status,
                run.goal,
                format_unix_ts(run.started_at),
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

    /// Create a team DB with `n` runs and return the home_dir.
    fn setup_team_db(team_name: &str, n: usize) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let data_dir = team::team_dir(&home, team_name).join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let db = Database::open(&data_dir.join("mika.db")).unwrap();
        for i in 0..n {
            let run_id = format!("run-{i:04}");
            let ts = 1740000000 + (i as i64 * 300); // spaced 5 min apart
            db.insert_team_run(&run_id, team_name, &format!("Goal {i}"), 3, ts)
                .unwrap();
            db.update_team_run(&run_id, "completed", None, 1, Some("Done"), Some(ts + 60))
                .unwrap();
        }
        (tmp, home)
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
        let (_tmp, home) = setup_team_db("dev-team", 2);

        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamHistoryTool { home_dir: home };

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
        let (_tmp, home) = setup_team_db("dev-team", 5);

        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTeamHistoryTool { home_dir: home };

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
