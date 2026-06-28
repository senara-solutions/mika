use anyhow::Result;
use async_trait::async_trait;
use mika_common::agent;
use mika_common::claude::ToolDefinition;
use mika_common::config::Settings;
use mika_common::home;
use secrecy::ExposeSecret;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct DelegateTaskTool {
    /// The global Mika home directory (e.g. `~/.mika/`).
    pub home_dir: PathBuf,
    pub settings: Settings,
    pub http_client: reqwest::Client,
    /// Shared GitHub App instance (avoids duplicate `from_settings` calls with separate caches).
    pub github_app: Option<Arc<mika_common::github_app::GitHubApp>>,
}

#[async_trait]
impl Tool for DelegateTaskTool {
    fn name(&self) -> &str {
        "delegate_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "delegate_task".to_string(),
            description: "Delegate a task to another agent and get their response. The delegate agent runs with its own personality, memory, and skills. It has NO management tools (cannot list agents, run teams, or delegate further) and no MCP server connections. Best for single-shot consultations like 'ask researcher to look into X'.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_name": {
                        "type": "string",
                        "description": "Name of the agent to delegate to (e.g. 'researcher')"
                    },
                    "task": {
                        "type": "string",
                        "description": "The task or question for the delegate agent"
                    },
                    "task_id": {
                        "type": "string",
                        "description": "ID of the task tracking this delegation. You MUST create a task first using create_task, then pass its ID here."
                    }
                },
                "required": ["agent_name", "task", "task_id"]
            }),
        }
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(120)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let agent_name = input["agent_name"].as_str().unwrap_or("");
        if agent_name.is_empty() {
            return Ok(ToolOutput::error("'agent_name' is required."));
        }
        if agent_name.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'agent_name' too long: {} characters (max: {MAX_INPUT_LEN})",
                agent_name.len()
            )));
        }
        if let Err(e) = agent::validate_agent_name(agent_name) {
            return Ok(ToolOutput::error(format!("Invalid agent name: {e}")));
        }

        // Block self-delegation
        let current_agent_id = ctx.db.agent_id();
        if agent_name == current_agent_id {
            return Ok(ToolOutput::error(
                "Cannot delegate to yourself. Call the tool directly instead.",
            ));
        }

        // Only orchestrators can delegate
        if !super::is_orchestrator(&self.home_dir, current_agent_id) {
            return Ok(ToolOutput::error(
                "Only orchestrator agents can delegate tasks. You are a specialist — call tools directly.",
            ));
        }

        let task = input["task"].as_str().unwrap_or("");
        if task.is_empty() {
            return Ok(ToolOutput::error("'task' is required."));
        }
        if task.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'task' too long: {} characters (max: {MAX_INPUT_LEN})",
                task.len()
            )));
        }

        // Validate task_id — delegation requires a tracked task
        let task_id_raw = input["task_id"].as_str().unwrap_or("");
        let scoped = match super::AgentScopedTaskId::from_tool_context(ctx, task_id_raw) {
            Ok(s) => s,
            Err(e) => return Ok(e),
        };
        if let Some(err) = super::validate_task(ctx.db, &scoped).await {
            return Ok(ToolOutput::error(err));
        }

        if !agent::agent_exists(&self.home_dir, agent_name) {
            return Ok(ToolOutput::error(format!(
                "Agent '{agent_name}' not found. Use list_agents to see available agents."
            )));
        }

        let agent_home = agent::agent_dir(&self.home_dir, agent_name);
        let db_path = home::container_db_path(&self.home_dir);

        let db = match crate::db::Database::open(&db_path) {
            Ok(db) => db,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "Failed to open database for agent '{agent_name}': {e}"
                )));
            }
        };
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, agent_name);

        // Load delegate agent identity and skills (with identity allowlist + DB overrides)
        let identity = crate::prompt::load_identity(&agent_home);
        let mut skills = crate::skills::SkillRegistry::from_dir(&agent_home.join("skills"));
        if let Some(ref allowlist) = identity.skills.allowlist {
            skills.apply_identity_allowlist(allowlist);
        }
        if let Ok(overrides) = async_db.get_skill_overrides(agent_name).await {
            skills.apply_overrides(&overrides);
        }
        skills.log_summary();

        // Register delegate agent so sessions FK constraint is satisfied.
        // Follows the team engine pattern (engine.rs line 95).
        if let Err(e) = async_db
            .register_agent(
                agent_name,
                &identity.name,
                agent_home.to_str().unwrap_or(""),
            )
            .await
        {
            tracing::warn!(agent = agent_name, error = %e, "failed to register delegate agent");
        }

        // Build tools: default_tools only — NO management tools (prevents recursion)
        let tool_registry = crate::tools::default_tools();

        // Create an LLM provider for the delegate
        let llm = match self.settings.make_llm_provider() {
            Ok(p) => p,
            Err(e) => {
                async_db.shutdown();
                return Ok(ToolOutput::error(format!(
                    "Failed to create LLM provider: {e}"
                )));
            }
        };

        let embedding_client = self.settings.make_embedding_client();
        let session_id = format!("delegate-{}", uuid::Uuid::new_v4());
        let skills_dirty = AtomicBool::new(false);

        // Look up chat_id from the orchestrator's DB context. The delegate's
        // agent-scoped customer_config won't have it (chat_id is stored under
        // the orchestrator's agent_id).
        let chat_id: Option<i64> = match ctx.db.get_customer_config("chat_id").await {
            Ok(Some(s)) => match s.parse::<i64>() {
                Ok(id) => {
                    tracing::debug!(
                        delegate = agent_name,
                        orchestrator = ctx.db.agent_id(),
                        chat_id = id,
                        "delegate_task: resolved chat_id from orchestrator context"
                    );
                    Some(id)
                }
                Err(e) => {
                    tracing::warn!(error = %e, raw = %s, "corrupt chat_id in customer_config, delegate will lack outbound messaging");
                    None
                }
            },
            Ok(None) => {
                tracing::debug!(
                    delegate = agent_name,
                    orchestrator = ctx.db.agent_id(),
                    "delegate_task: no chat_id in orchestrator's customer_config"
                );
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "delegate_task: failed to read chat_id from orchestrator DB");
                None
            }
        };

        // Create a sender with the delegate's agent_name (not the orchestrator's)
        // so outbound messages are correctly attributed and reply routing works.
        // Pass the explicit chat_id so the sender doesn't need to look it up
        // from the delegate's agent-scoped DB (where it doesn't exist).
        let delegate_sender: Option<Arc<dyn crate::messaging::MessageSender>> = if ctx
            .message_sender
            .is_some()
        {
            if let (Some(url), Some(token)) =
                (&self.settings.routing_url, &self.settings.internal_token)
            {
                tracing::debug!(
                    delegate = agent_name,
                    has_chat_id = chat_id.is_some(),
                    "delegate_task: creating delegate sender with agent_name and chat_id override"
                );
                Some(Arc::new(crate::messaging::GatewayMessageSender::new(
                    url.clone(),
                    token.clone(),
                    async_db.clone(),
                    self.http_client.clone(),
                    None,
                    Some(agent_name.to_string()),
                    chat_id,
                    self.settings.customer_id.clone(),
                )))
            } else {
                tracing::warn!(
                    delegate = agent_name,
                    "delegate_task: no routing_url/internal_token, delegate will lack outbound messaging"
                );
                None
            }
        } else {
            tracing::debug!(
                delegate = agent_name,
                "delegate_task: orchestrator has no message_sender, delegate will also lack one"
            );
            None
        };

        // -- Session lifecycle for observability (#253) --
        // Create session BEFORE run_team_agent so tool-level DB writes
        // (audit events) during the agent run have a valid session FK.
        let delegate_metadata = serde_json::json!({
            "trigger": "delegate",
            "orchestrator": current_agent_id,
            "task_id": task_id_raw
        })
        .to_string();
        if let Err(e) = async_db
            .create_session_with_parent(
                &session_id,
                agent_name,
                "delegate",
                Some(&delegate_metadata),
                Some(ctx.session_id),
                if task_id_raw.is_empty() {
                    None
                } else {
                    Some(task_id_raw)
                },
            )
            .await
        {
            tracing::warn!(session = %session_id, error = %e, "failed to create delegate session");
        }

        // Persist the delegated task as a user message (internal — agent-to-agent)
        if let Err(e) = async_db
            .save_message_with_metadata(&session_id, "user", task, None, Some(ctx.trace_id), true)
            .await
        {
            tracing::warn!(session = %session_id, error = %e, "failed to persist delegate task message");
        }

        let params = crate::agent::TeamAgentParams {
            db: &async_db,
            llm: llm.as_ref(),
            tools: &tool_registry,
            skills: &skills,
            home_dir: &agent_home,
            task_message: task,
            team_context: "You are being consulted by another agent. Provide a thorough, complete answer.",
            session_id: &session_id,
            embedding_client: embedding_client.as_ref(),
            brave_api_key: self
                .settings
                .brave_api_key
                .as_ref()
                .map(|s| s.expose_secret()),
            github_token: self.settings.agent_github_token(),
            github_app: self.github_app.as_deref(),
            skills_dirty: &skills_dirty,
            settings: Some(&self.settings),
            mcp_manager: None,
            agent_name,
            child_task_id: None,
            message_sender: delegate_sender,
            // Propagate the orchestrator's trace_id so delegate events correlate.
            trace_id: Some(ctx.trace_id.to_string()),
        };

        let result = crate::agent::run_team_agent(&params).await;

        // Persist result message for observability (#253) — internal (agent-to-agent)
        match &result {
            Ok(outcome) => {
                // Persist non-empty text responses; skip empty (tool-use-only turns)
                let text = match outcome {
                    crate::agent::TeamAgentOutcome::Done(Some(t)) => Some(t.as_str()),
                    crate::agent::TeamAgentOutcome::TimedOut(t) => Some(t.as_str()),
                    crate::agent::TeamAgentOutcome::Done(None) => None,
                };
                if let Some(text) = text
                    && let Err(e) = async_db
                        .save_message_with_metadata(
                            &session_id,
                            "assistant",
                            text,
                            None,
                            Some(ctx.trace_id),
                            true,
                        )
                        .await
                {
                    tracing::warn!(session = %session_id, error = %e, "failed to persist delegate response");
                }
            }
            Err(e) => {
                let error_msg = format!("Delegation failed: {e}");
                if let Err(pe) = async_db
                    .save_message_with_metadata(
                        &session_id,
                        "system",
                        &error_msg,
                        None,
                        Some(ctx.trace_id),
                        true,
                    )
                    .await
                {
                    tracing::warn!(session = %session_id, error = %pe, "failed to persist delegate error");
                }
            }
        }

        // End the delegate session
        if let Err(e) = async_db.end_session(&session_id).await {
            tracing::warn!(session = %session_id, error = %e, "failed to end delegate session");
        }
        if let Some(map) = ctx.pr_reviews_posted {
            map.remove(&session_id);
        }

        // Critical: shut down the async DB thread to prevent leaks
        async_db.shutdown();

        match result {
            Ok(outcome) => {
                let text = outcome.into_text();
                if text.is_empty() {
                    Ok(ToolOutput::success(format!(
                        "Agent '{agent_name}' completed the task but produced no text response."
                    )))
                } else {
                    Ok(ToolOutput::success(format!(
                        "Response from {agent_name}:\n\n{text}"
                    )))
                }
            }
            Err(e) => Ok(ToolOutput::error(format!(
                "Delegation to '{agent_name}' failed: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::{TestHarness, create_test_task, dummy_settings};

    #[tokio::test]
    async fn test_delegate_task_missing_agent_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
            http_client: reqwest::Client::new(),
            github_app: None,
        };

        let result = tool
            .execute(serde_json::json!({"task": "do something"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'agent_name' is required"));
    }

    #[tokio::test]
    async fn test_delegate_task_missing_task_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
            http_client: reqwest::Client::new(),
            github_app: None,
        };

        let result = tool
            .execute(
                serde_json::json!({"agent_name": "researcher", "task": "do something"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("invalid_uuid"),
            "expected UUID validation error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_delegate_task_invalid_task_uuid() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
            http_client: reqwest::Client::new(),
            github_app: None,
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "agent_name": "researcher",
                    "task": "do something",
                    "task_id": "nonexistent-id"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("invalid_uuid"),
            "expected UUID validation error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_delegate_task_task_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
            http_client: reqwest::Client::new(),
            github_app: None,
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "agent_name": "researcher",
                    "task": "do something",
                    "task_id": "00000000-0000-0000-0000-000000000000"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("task_not_found"),
            "expected task_not_found error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_delegate_task_completed_task_rejected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let wi_id = create_test_task(ctx.db).await;
        // Transition to completed
        ctx.db
            .update_manual_task_status(&wi_id, "completed")
            .await
            .unwrap();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
            http_client: reqwest::Client::new(),
            github_app: None,
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "agent_name": "researcher",
                    "task": "do something",
                    "task_id": wi_id
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("not an active task"),
            "expected inactive task error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_delegate_task_missing_task() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let wi_id = create_test_task(ctx.db).await;
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
            http_client: reqwest::Client::new(),
            github_app: None,
        };

        let result = tool
            .execute(
                serde_json::json!({"agent_name": "researcher", "task_id": wi_id}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'task' is required"));
    }

    #[tokio::test]
    async fn test_delegate_task_nonexistent_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let wi_id = create_test_task(ctx.db).await;
        let tool = DelegateTaskTool {
            home_dir: tmp.path().to_path_buf(),
            settings: dummy_settings(),
            http_client: reqwest::Client::new(),
            github_app: None,
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "agent_name": "nonexistent",
                    "task": "test",
                    "task_id": wi_id
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_delegate_task_invalid_agent_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
            http_client: reqwest::Client::new(),
            github_app: None,
        };

        let result = tool
            .execute(
                serde_json::json!({"agent_name": "INVALID", "task": "test"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid agent name"));
    }

    #[tokio::test]
    async fn test_delegate_task_self_delegation_blocked() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let agent_id = ctx.db.agent_id().to_string();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
            http_client: reqwest::Client::new(),
            github_app: None,
        };

        let result = tool
            .execute(
                serde_json::json!({"agent_name": agent_id, "task": "test"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Cannot delegate to yourself"));
    }

    #[tokio::test]
    async fn test_delegate_task_non_orchestrator_blocked() {
        // Create a harness with a non-default agent name
        let harness = TestHarness::with_agent("specialist-agent");
        let ctx = harness.ctx();
        let tmp = tempfile::tempdir().unwrap();
        let tool = DelegateTaskTool {
            home_dir: tmp.path().to_path_buf(),
            settings: dummy_settings(),
            http_client: reqwest::Client::new(),
            github_app: None,
        };

        let result = tool
            .execute(
                serde_json::json!({"agent_name": "other-agent", "task": "test"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Only orchestrator agents"));
    }
}
