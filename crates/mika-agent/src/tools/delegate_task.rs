use anyhow::Result;
use async_trait::async_trait;
use mika_common::agent;
use mika_common::claude::{ClaudeClient, ToolDefinition};
use mika_common::config::Settings;
use mika_common::home;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct DelegateTaskTool {
    /// The global Mika home directory (e.g. `~/.mika/`).
    pub home_dir: PathBuf,
    pub settings: Settings,
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
                    "work_item_id": {
                        "type": "string",
                        "description": "ID of the work item tracking this delegation. You MUST create a work item first using create_work_item, then pass its ID here."
                    }
                },
                "required": ["agent_name", "task", "work_item_id"]
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

        // Validate work_item_id — delegation requires a tracked work item
        let work_item_id = input["work_item_id"].as_str().unwrap_or("");
        if let Some(err) = super::validate_work_item(ctx.db, work_item_id).await {
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

        // Load delegate agent's skills (with DB overrides)
        let mut skills = crate::skills::SkillRegistry::from_dir(&agent_home.join("skills"));
        if let Ok(overrides) = async_db.get_skill_overrides(agent_name).await {
            skills.apply_overrides(&overrides);
        }

        // Build tools: default_tools only — NO management tools (prevents recursion)
        let tool_registry = crate::tools::default_tools();

        // Create a Claude client for the delegate
        let claude = match ClaudeClient::new(
            self.settings.anthropic_api_key.clone(),
            self.settings.claude_model.clone(),
            self.settings.claude_max_tokens,
        ) {
            Ok(c) => c,
            Err(e) => {
                async_db.shutdown();
                return Ok(ToolOutput::error(format!(
                    "Failed to create Claude client: {e}"
                )));
            }
        };

        let embedding_client = self.settings.make_embedding_client();
        let session_id = uuid::Uuid::new_v4().to_string();
        let skills_dirty = AtomicBool::new(false);

        // Look up chat_id from the orchestrator's DB context. The delegate's
        // agent-scoped customer_config won't have it (chat_id is stored under
        // the orchestrator's agent_id).
        let chat_id: Option<i64> = match ctx.db.get_customer_config("chat_id").await {
            Ok(Some(s)) => match s.parse::<i64>() {
                Ok(id) => Some(id),
                Err(e) => {
                    tracing::warn!(error = %e, raw = %s, "corrupt chat_id in customer_config, delegate will lack outbound messaging");
                    None
                }
            },
            _ => None,
        };

        // Create a sender with the delegate's agent_name (not the orchestrator's)
        // so outbound messages are correctly attributed and reply routing works.
        // Pass the explicit chat_id so the sender doesn't need to look it up
        // from the delegate's agent-scoped DB (where it doesn't exist).
        let delegate_sender: Option<Arc<dyn crate::messaging::MessageSender>> =
            if ctx.message_sender.is_some() {
                if let (Some(url), Some(token)) =
                    (&self.settings.routing_url, &self.settings.internal_token)
                {
                    Some(Arc::new(crate::messaging::GatewayMessageSender::new(
                        url.clone(),
                        token.clone(),
                        async_db.clone(),
                        reqwest::Client::new(),
                        None,
                        Some(agent_name.to_string()),
                        chat_id,
                    )))
                } else {
                    None
                }
            } else {
                None
            };

        let params = crate::agent::TeamAgentParams {
            db: &async_db,
            claude: &claude,
            tools: &tool_registry,
            skills: &skills,
            home_dir: &agent_home,
            task_message: task,
            team_context: "You are being consulted by another agent. Provide a thorough, complete answer.",
            session_id: &session_id,
            embedding_client: embedding_client.as_ref(),
            brave_api_key: self.settings.brave_api_key.as_deref(),
            skills_dirty: &skills_dirty,
            mcp_manager: None,
            agent_name,
            child_task_id: None,
            message_sender: delegate_sender,
        };

        let result = crate::agent::run_team_agent(&params).await;

        // Critical: shut down the async DB thread to prevent leaks
        async_db.shutdown();

        match result {
            Ok(Some(text)) => Ok(ToolOutput::success(format!(
                "Response from {agent_name}:\n\n{text}"
            ))),
            Ok(None) => Ok(ToolOutput::success(format!(
                "Agent '{agent_name}' completed the task but produced no text response."
            ))),
            Err(e) => Ok(ToolOutput::error(format!(
                "Delegation to '{agent_name}' failed: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::{TestHarness, create_test_work_item, dummy_settings};

    #[tokio::test]
    async fn test_delegate_task_missing_agent_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
        };

        let result = tool
            .execute(serde_json::json!({"task": "do something"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'agent_name' is required"));
    }

    #[tokio::test]
    async fn test_delegate_task_missing_work_item_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
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
            result.content.contains("create a work item first"),
            "expected work item error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_delegate_task_invalid_work_item_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "agent_name": "researcher",
                    "task": "do something",
                    "work_item_id": "nonexistent-id"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("not found"),
            "expected not found error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_delegate_task_completed_work_item_rejected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let wi_id = create_test_work_item(ctx.db).await;
        // Transition to completed
        ctx.db
            .update_manual_task_status(&wi_id, "completed")
            .await
            .unwrap();
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "agent_name": "researcher",
                    "task": "do something",
                    "work_item_id": wi_id
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("not an active work item"),
            "expected inactive work item error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_delegate_task_missing_task() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let wi_id = create_test_work_item(ctx.db).await;
        let tool = DelegateTaskTool {
            home_dir: PathBuf::from("/tmp"),
            settings: dummy_settings(),
        };

        let result = tool
            .execute(
                serde_json::json!({"agent_name": "researcher", "work_item_id": wi_id}),
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
        let wi_id = create_test_work_item(ctx.db).await;
        let tool = DelegateTaskTool {
            home_dir: tmp.path().to_path_buf(),
            settings: dummy_settings(),
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "agent_name": "nonexistent",
                    "task": "test",
                    "work_item_id": wi_id
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
