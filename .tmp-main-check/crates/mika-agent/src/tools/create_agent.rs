use anyhow::Result;
use async_trait::async_trait;
use mika_common::agent;
use mika_common::claude::ToolDefinition;
use mika_common::home::bootstrap_agent;
use serde_json::Value;
use std::path::PathBuf;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::startup::seed_bundled_skills_if_needed;

pub struct CreateAgentTool {
    pub home_dir: PathBuf,
}

#[async_trait]
impl Tool for CreateAgentTool {
    fn name(&self) -> &str {
        "create_agent"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_agent".to_string(),
            description: "Create a new agent with optional personality customization. \
                The agent will be available for delegation after creation."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Agent name (lowercase, hyphens allowed, e.g. 'elon-musk')"
                    },
                    "display_name": {
                        "type": "string",
                        "description": "Human-readable display name for the agent (e.g. 'Elon Musk')"
                    },
                    "soul": {
                        "type": "string",
                        "description": "Full soul.md content defining the agent's personality, communication style, expertise, and philosophy"
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let raw_name = input["name"].as_str().unwrap_or("").trim().to_string();
        if raw_name.is_empty() {
            return Ok(ToolOutput::error("'name' is required and cannot be empty."));
        }
        if raw_name.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'name' exceeds maximum length of {MAX_INPUT_LEN} characters."
            )));
        }

        let name = agent::normalize_agent_name(&raw_name);

        if let Err(e) = agent::validate_agent_name(&name) {
            return Ok(ToolOutput::error(format!("Invalid agent name: {e}")));
        }

        if agent::agent_exists(&self.home_dir, &name) {
            return Ok(ToolOutput::error(format!("Agent '{name}' already exists.")));
        }

        // Ensure agents directory exists
        let agents_dir = self.home_dir.join("agents");
        if let Err(e) = std::fs::create_dir_all(&agents_dir) {
            return Ok(ToolOutput::error(format!(
                "Failed to create agents directory: {e}"
            )));
        }

        // Bootstrap the agent (creates dirs + default files)
        if let Err(e) = bootstrap_agent(&self.home_dir, &name) {
            return Ok(ToolOutput::error(format!(
                "Failed to bootstrap agent '{name}': {e}"
            )));
        }

        let agent_home = agent::agent_dir(&self.home_dir, &name);

        // Overwrite identity.toml if display_name provided
        if let Some(display_name) = input["display_name"].as_str() {
            let display_name = display_name.trim();
            if !display_name.is_empty() {
                let identity_content = format!("name = \"{display_name}\"\nemoji = \"✦\"\n");
                if let Err(e) = std::fs::write(agent_home.join("identity.toml"), &identity_content)
                {
                    return Ok(ToolOutput::error(format!(
                        "Agent created but failed to write identity.toml: {e}"
                    )));
                }
            }
        }

        // Overwrite soul.md if soul content provided
        if let Some(soul) = input["soul"].as_str() {
            let soul = soul.trim();
            if !soul.is_empty() {
                if soul.len() > MAX_INPUT_LEN {
                    return Ok(ToolOutput::error(format!(
                        "'soul' exceeds maximum length of {MAX_INPUT_LEN} characters."
                    )));
                }
                if let Err(e) = std::fs::write(agent_home.join("soul.md"), soul) {
                    return Ok(ToolOutput::error(format!(
                        "Agent created but failed to write soul.md: {e}"
                    )));
                }
            }
        }

        // Seed bundled skills
        seed_bundled_skills_if_needed(&agent_home, false);

        let display = input["display_name"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&name);

        Ok(ToolOutput::success(format!(
            "Agent '{name}' created successfully as {display}. \
             The agent is now available for delegation via `delegate_task`."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::fs;

    #[tokio::test]
    async fn test_create_agent_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateAgentTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(serde_json::json!({"name": "researcher"}), &ctx)
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "Expected success, got: {}",
            result.content
        );
        assert!(result.content.contains("researcher"));

        // Verify agent was created
        assert!(agent::agent_exists(tmp.path(), "researcher"));
    }

    #[tokio::test]
    async fn test_create_agent_with_display_name_and_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateAgentTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "elon-musk",
                    "display_name": "Elon Musk",
                    "soul": "You are Elon Musk. Think from first principles."
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "Expected success, got: {}",
            result.content
        );
        assert!(result.content.contains("Elon Musk"));

        let agent_home = agent::agent_dir(tmp.path(), "elon-musk");
        let identity = fs::read_to_string(agent_home.join("identity.toml")).unwrap();
        assert!(identity.contains("Elon Musk"));

        let soul = fs::read_to_string(agent_home.join("soul.md")).unwrap();
        assert!(soul.contains("first principles"));
    }

    #[tokio::test]
    async fn test_create_agent_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateAgentTool {
            home_dir: tmp.path().to_path_buf(),
        };

        // Create it once
        tool.execute(serde_json::json!({"name": "researcher"}), &ctx)
            .await
            .unwrap();

        // Try to create again
        let result = tool
            .execute(serde_json::json!({"name": "researcher"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("already exists"));
    }

    #[tokio::test]
    async fn test_create_agent_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateAgentTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(serde_json::json!({"name": ""}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);

        let result = tool
            .execute(serde_json::json!({"name": "UPPER_CASE!"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_create_agent_normalizes_name() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateAgentTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(serde_json::json!({"name": "  MyAgent  "}), &ctx)
            .await
            .unwrap();
        // "myagent" after normalization (lowercase, trimmed)
        assert!(
            !result.is_error,
            "Expected success, got: {}",
            result.content
        );
        assert!(agent::agent_exists(tmp.path(), "myagent"));
    }
}
