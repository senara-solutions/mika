use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::config::Settings;
use serde_json::Value;
use std::path::PathBuf;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct RunTeamTool {
    pub home_dir: PathBuf,
    pub settings: Settings,
}

#[async_trait]
impl Tool for RunTeamTool {
    fn name(&self) -> &str {
        "run_team"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_team".to_string(),
            description: "Run a team workflow with a specified goal. The team's agents will collaborate to decompose, execute, review, and deliver results.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "team_name": {
                        "type": "string",
                        "description": "Name of the team to run (e.g. 'dev-team')"
                    },
                    "goal": {
                        "type": "string",
                        "description": "The goal or task for the team to accomplish"
                    }
                },
                "required": ["team_name", "goal"]
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

        let goal = input["goal"].as_str().unwrap_or("");
        if goal.is_empty() {
            return Ok(ToolOutput::error("'goal' is required."));
        }
        if goal.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'goal' too long: {} characters (max: {MAX_INPUT_LEN})",
                goal.len()
            )));
        }

        match crate::teams::run_team(team_name, goal, &self.home_dir, &self.settings, None).await {
            Ok(run) => {
                if let Some(ref deliverable) = run.deliverable {
                    Ok(ToolOutput::success(format!(
                        "Team '{}' completed (status: {}). Deliverable:\n\n{}",
                        run.team_name, run.status, deliverable
                    )))
                } else {
                    Ok(ToolOutput::success(format!(
                        "Team '{}' finished (status: {}). No deliverable produced.",
                        run.team_name, run.status
                    )))
                }
            }
            Err(e) => Ok(ToolOutput::error(format!(
                "Team '{}' failed: {}",
                team_name, e
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    fn dummy_settings() -> Settings {
        // Create minimal settings for testing (API key not needed for validation tests)
        Settings {
            anthropic_api_key: None,
            claude_model: "claude-sonnet-4-6".to_string(),
            claude_max_tokens: 4096,
            db_path: PathBuf::from("test.db"),
            log_level: "info".to_string(),
            routing_url: None,
            customer_id: None,
            server_port: 8080,
            internal_token: None,
            openai_api_key: None,
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_dimensions: 512,
            home_dir: PathBuf::from("/tmp"),
        }
    }

    #[tokio::test]
    async fn test_run_team_missing_team_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RunTeamTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
        };

        let result = tool
            .execute(serde_json::json!({"goal": "do something"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'team_name' is required"));
    }

    #[tokio::test]
    async fn test_run_team_missing_goal() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RunTeamTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
        };

        let result = tool
            .execute(serde_json::json!({"team_name": "dev-team"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'goal' is required"));
    }

    #[tokio::test]
    async fn test_run_team_nonexistent_team() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RunTeamTool {
            home_dir: tmp.path().to_path_buf(),
            settings: dummy_settings(),
        };

        let result = tool
            .execute(
                serde_json::json!({"team_name": "nonexistent", "goal": "test"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("failed"));
    }
}
