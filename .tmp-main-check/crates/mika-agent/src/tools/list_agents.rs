use anyhow::Result;
use async_trait::async_trait;
use mika_common::agent;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::fmt::Write;
use std::path::PathBuf;

use super::{Tool, ToolContext, ToolOutput};
use crate::prompt::load_identity;

pub struct ListAgentsTool {
    pub home_dir: PathBuf,
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn name(&self) -> &str {
        "list_agents"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_agents".to_string(),
            description: "List all configured agents with their identity and role. Use this to see which agents are available for delegation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let names = agent::list_agents(&self.home_dir);

        if names.is_empty() {
            return Ok(ToolOutput::success("No agents configured."));
        }

        let mut lines = String::new();
        for name in &names {
            let agent_home = agent::agent_dir(&self.home_dir, name);
            let identity = load_identity(&agent_home);

            // Read first line of soul.md for a role hint
            let role_hint = std::fs::read_to_string(agent_home.join("soul.md"))
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| !l.trim().is_empty())
                        .map(|l| l.trim().to_string())
                });

            if let Some(hint) = role_hint {
                writeln!(
                    lines,
                    "- {name} ({} {}): {hint}",
                    identity.emoji, identity.name
                )
                .unwrap();
            } else {
                writeln!(lines, "- {name} ({} {})", identity.emoji, identity.name).unwrap();
            }
        }

        let count = names.len();
        Ok(ToolOutput::success(format!(
            "Found {count} agent{}:\n{lines}",
            if count == 1 { "" } else { "s" },
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::fs;

    #[tokio::test]
    async fn test_list_agents_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = ListAgentsTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No agents configured"));
    }

    #[tokio::test]
    async fn test_list_agents_found() {
        let tmp = tempfile::tempdir().unwrap();

        // Create two agents
        let mika_dir = agent::agent_dir(tmp.path(), "mika");
        fs::create_dir_all(&mika_dir).unwrap();
        fs::write(mika_dir.join("config.toml"), "# config").unwrap();
        fs::write(
            mika_dir.join("identity.toml"),
            "name = \"Mika\"\nemoji = \"✦\"\n",
        )
        .unwrap();
        fs::write(
            mika_dir.join("soul.md"),
            "You are a sharp executive assistant.\n",
        )
        .unwrap();

        let researcher_dir = agent::agent_dir(tmp.path(), "researcher");
        fs::create_dir_all(&researcher_dir).unwrap();
        fs::write(researcher_dir.join("config.toml"), "# config").unwrap();
        fs::write(
            researcher_dir.join("identity.toml"),
            "name = \"Rex\"\nemoji = \"🔬\"\n",
        )
        .unwrap();

        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = ListAgentsTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Found 2 agents"));
        assert!(result.content.contains("mika (✦ Mika)"));
        assert!(result.content.contains("researcher (🔬 Rex)"));
        assert!(result.content.contains("sharp executive assistant"));
    }
}
