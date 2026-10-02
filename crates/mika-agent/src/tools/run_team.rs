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
            None,           // run_team tool: no AppState access for session-scoped dedup (#821)
            ctx.tier,       // mika#1962 — cached at agent init, never re-read here
            ctx.deployment, // mika#2290 — same, for the hosting ground truth
        )
        .await;
        team_db.shutdown();

        // Consolidated team-run notification (sync path).
        // Paired with dispatch_invoke_orchestrator (async path); see
        // docs/plans/2026-04-24-003-fix-team-callback-consolidation-plan.md
        // for why this lives here and not in TeamEngine::finalize_and_shutdown
        // (keeps the team engine free of a user_message_sender field).
        if let Ok(ref run) = result
            && let Some(msg) = crate::teams::notification::build_run_completion_message(run)
        {
            if let Some(ref sender) = ctx.message_sender {
                match sender.send(&msg.text).await {
                    Ok(crate::messaging::SendOutcome::Delivered) => {
                        tracing::info!(
                            team_run_id = %run.run_id,
                            team_name = %run.team_name,
                            status = %run.status,
                            notification_kind = %msg.notification_kind,
                            deliverable_chars = msg.deliverable_chars,
                            truncated = msg.truncated,
                            path = "sync",
                            "team_run_notified"
                        );
                    }
                    Ok(crate::messaging::SendOutcome::NoChannel) => {
                        // NoChannel is permanent — silent per dispatcher policy.
                    }
                    Ok(crate::messaging::SendOutcome::Failed { reason }) => {
                        tracing::warn!(
                            team_run_id = %run.run_id,
                            error = %reason,
                            "team_run_notification_delivery_failed"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            team_run_id = %run.run_id,
                            error = %e,
                            "team_run_notification_send_error"
                        );
                    }
                }
            } else {
                tracing::debug!(
                    team_run_id = %run.run_id,
                    "no message_sender available for team-run notification (sync path)"
                );
            }
            // Log warning for completed-without-deliverable
            if msg.notification_kind == "fallback" {
                tracing::warn!(
                    team_run_id = %run.run_id,
                    "team run completed without a deliverable"
                );
            }
        }

        match result {
            Ok(run) => Ok(format_run_result(&run)),
            Err(e) => Ok(ToolOutput::error(format!(
                "Team '{}' failed: {}",
                team_name, e
            ))),
        }
    }
}

/// The tool result the **calling agent** reads at the end of a run.
///
/// A withheld deliverable (mika#2633) gets an agent-register line rather than
/// the neutral line itself: that line is written for the PERSON, and its "Ask
/// again and it will be re-written" hands the agent exactly one affordance — a
/// fresh 15-minute run on the same goal, unbounded, while it believes it is
/// following an instruction. The engine has already spent its single re-write.
/// The comparison is against the one constant the engine poses, so the two
/// cannot drift. Names no workaround (mika#2520).
fn format_run_result(run: &crate::teams::types::TeamRun) -> ToolOutput {
    if run.deliverable.as_deref() == Some(crate::teams::engine::TEAM_DELIVERABLE_WITHHELD) {
        return ToolOutput::success(format!(
            "Team '{}' completed (status: {}), but its deliverable was WITHHELD by the \
             engine: it proposed opening access to testimony-grade data, which the \
             non-transit doctrine refuses, and the single re-write the engine allows \
             did not remove the proposal. NOTHING from the team was passed on; the \
             person has already received a neutral line saying so. Re-running the \
             same goal will most likely reproduce the refusal, so do not relaunch the \
             team for it. You may tell the person what the team CAN do without \
             opening such access.",
            run.team_name, run.status
        ));
    }
    if let Some(ref deliverable) = run.deliverable {
        ToolOutput::success(format!(
            "Team '{}' completed (status: {}). Deliverable:\n\n{}",
            run.team_name, run.status, deliverable
        ))
    } else {
        ToolOutput::success(format!(
            "Team '{}' finished (status: {}). No deliverable produced.",
            run.team_name, run.status
        ))
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
            github_app: None,
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
            github_app: None,
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
            github_app: None,
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
            github_app: None,
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
            github_app: None,
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

    fn completed_run(deliverable: &str) -> crate::teams::types::TeamRun {
        let mut run: crate::teams::types::TeamRun = serde_json::from_value(serde_json::json!({
            "run_id": "run-2633",
            "team_name": "dev-team",
            "goal": "produce a report",
        }))
        .unwrap();
        run.status = crate::teams::types::RunStatus::Completed;
        run.deliverable = Some(deliverable.to_string());
        run
    }

    /// Constat de revue (agent-native W1, mika#2633) — la ligne neutre est
    /// écrite pour la **personne** (« Ask again and it will be re-written »).
    /// Rendue telle quelle à l'agent appelant, elle lui donne une seule
    /// affordance : relancer `run_team` sur le même but, c'est-à-dire un run
    /// multi-agents complet de 15 minutes que rien ne borne, en croyant suivre
    /// l'instruction reçue. L'agent doit lire un fait dans son registre : le
    /// livrable a été retenu par le moteur, la personne a reçu la ligne neutre,
    /// et relancer reproduira probablement le refus.
    #[test]
    fn mika2633_lagent_appelant_lit_la_retenue_dans_son_registre() {
        let out = format_run_result(&completed_run(
            crate::teams::engine::TEAM_DELIVERABLE_WITHHELD,
        ));

        assert!(!out.is_error);
        assert!(
            !out.content.contains("Ask again"),
            "l'impératif adressé à la personne ne doit pas atteindre l'agent \
             appelant : {}",
            out.content
        );
        assert!(out.content.contains("WITHHELD"), "{}", out.content);
        assert!(out.content.contains("non-transit"), "{}", out.content);
    }

    /// Le contrôle négatif : un livrable ordinaire est rendu tel quel, octet
    /// pour octet comme avant.
    #[test]
    fn mika2633_un_livrable_ordinaire_est_rendu_tel_quel() {
        let out = format_run_result(&completed_run("Le rapport demandé."));
        assert_eq!(
            out.content,
            "Team 'dev-team' completed (status: completed). Deliverable:\n\nLe rapport demandé."
        );
    }
}
