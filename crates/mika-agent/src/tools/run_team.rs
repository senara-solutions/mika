use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::config::Settings;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

use mika_common::home;

use crate::async_db::AsyncDatabase;
use crate::db::Database;
use crate::messaging::MessageSender;
use crate::teams::types::{TeamEvent, TeamEventCallback};

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct RunTeamTool {
    pub home_dir: PathBuf,
    pub settings: Settings,
    /// Shared GitHub App instance (avoids duplicate `from_settings` calls with separate caches).
    pub github_app: Option<Arc<mika_common::github_app::GitHubApp>>,
}

#[async_trait]
impl Tool for RunTeamTool {
    fn name(&self) -> &str {
        "run_team"
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(300)
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
        if let Err(e) = mika_common::team::validate_team_name(team_name) {
            return Ok(ToolOutput::error(format!("Invalid team name: {e}")));
        }

        // Only orchestrators can run teams
        let current_agent_id = ctx.db.agent_id();
        if !super::is_orchestrator(&self.home_dir, current_agent_id) {
            return Ok(ToolOutput::error(
                "Only orchestrator agents can run teams. You are a specialist — call tools directly.",
            ));
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

        // Open the shared container DB for team persistence
        let db_path = home::container_db_path(&self.home_dir);
        let team_db = match Database::open(&db_path) {
            Ok(db) => AsyncDatabase::new(db),
            Err(e) => return Ok(ToolOutput::error(format!("Failed to open database: {e}"))),
        };

        let callback: Option<TeamEventCallback> = ctx.message_sender.as_ref().map(|sender| {
            let sender: Arc<dyn MessageSender> = Arc::clone(sender);
            let cb: TeamEventCallback = Box::new(move |event: TeamEvent| {
                let text = match &event {
                    TeamEvent::PhaseChanged { phase, iteration } => {
                        Some(format!("[Team] Phase: {} (iteration {})", phase, iteration))
                    }
                    TeamEvent::AgentCompleted { agent, .. } => {
                        Some(format!("[Team] Agent '{}' completed", agent))
                    }
                    TeamEvent::AgentFailed { agent, error } => {
                        Some(format!("[Team] Agent '{}' failed: {}", agent, error))
                    }
                    TeamEvent::Deliverable(_) => Some("[Team] Deliverable ready".to_string()),
                    TeamEvent::RunFailed(msg) => Some(format!("[Team] Run failed: {}", msg)),
                    // Skip noisy/intermediate events
                    TeamEvent::Progress(_)
                    | TeamEvent::AgentStarted { .. }
                    | TeamEvent::TasksAssigned { .. }
                    | TeamEvent::CriticReview { .. } => None,
                };
                if let Some(text) = text {
                    let sender = Arc::clone(&sender);
                    tokio::spawn(async move {
                        let _ = sender.send(&text).await;
                    });
                }
            });
            cb
        });

        let result = crate::teams::run_team(
            team_name,
            goal,
            &self.home_dir,
            &self.settings,
            callback,
            team_db.clone(),
            None,
            self.github_app.clone(),
        )
        .await;
        team_db.shutdown();

        match result {
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
    use crate::test_utils::test_helpers::{TestHarness, dummy_settings};

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

    #[tokio::test]
    async fn test_run_team_invalid_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RunTeamTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
        };

        let result = tool
            .execute(
                serde_json::json!({"team_name": "INVALID", "goal": "test"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid team name"));
    }

    #[tokio::test]
    async fn test_run_team_non_orchestrator_blocked() {
        let harness = TestHarness::with_agent("specialist-agent");
        let ctx = harness.ctx();
        let tmp = tempfile::tempdir().unwrap();
        let tool = RunTeamTool {
            home_dir: tmp.path().to_path_buf(),
            settings: dummy_settings(),
        };

        let result = tool
            .execute(
                serde_json::json!({"team_name": "dev-team", "goal": "test"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Only orchestrator agents"));
    }
}
