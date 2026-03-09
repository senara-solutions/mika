use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::team;
use serde_json::Value;
use std::path::PathBuf;

use super::{Tool, ToolContext, ToolOutput};

pub struct ListTeamsTool {
    pub home_dir: PathBuf,
}

#[async_trait]
impl Tool for ListTeamsTool {
    fn name(&self) -> &str {
        "list_teams"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_teams".to_string(),
            description: "List all configured teams and their agents. Shows team names, agent counts, and orchestrator info.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let names = team::list_teams(&self.home_dir);

        if names.is_empty() {
            return Ok(ToolOutput::success("No teams configured."));
        }

        let mut lines = Vec::new();
        for name in &names {
            match team::load_team(&self.home_dir, name) {
                Ok(def) => {
                    let mut block = format!("## {name}");
                    block.push_str(&format!(
                        "\nOrchestrator: {}\nMax iterations: {}\nAgents:",
                        def.team.orchestrator, def.flow.max_iterations,
                    ));
                    for agent in &def.agents {
                        let role = if agent.role.is_empty() {
                            "unspecified"
                        } else {
                            &agent.role
                        };
                        let mandate_part = if agent.mandate.is_empty() {
                            String::new()
                        } else {
                            format!(" — {}", agent.mandate)
                        };
                        block.push_str(&format!(
                            "\n  - {} (role: {}){mandate_part}",
                            agent.name, role,
                        ));
                    }
                    lines.push(block);
                }
                Err(e) => {
                    lines.push(format!("- {} (error loading: {})", name, e));
                }
            }
        }

        let count = names.len();
        let body = lines.join("\n");
        Ok(ToolOutput::success(format!(
            "Found {count} team{}:\n{body}",
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
    async fn test_list_teams_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = ListTeamsTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No teams configured"));
    }

    #[tokio::test]
    async fn test_list_teams_found() {
        let tmp = tempfile::tempdir().unwrap();
        let team_dir = tmp.path().join("teams").join("dev-team");
        fs::create_dir_all(&team_dir).unwrap();
        fs::write(
            team_dir.join("team.toml"),
            r#"
[team]
name = "dev-team"
orchestrator = "planner"

[[agents]]
name = "planner"
role = "orchestrator"
mandate = "Plan tasks"

[[agents]]
name = "coder"
role = "specialist"
mandate = "Write code"

[flow]
max_iterations = 3
"#,
        )
        .unwrap();

        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = ListTeamsTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("## dev-team"));
        assert!(result.content.contains("Orchestrator: planner"));
        assert!(result.content.contains("Max iterations: 3"));
        assert!(result.content.contains("planner (role: orchestrator) — Plan tasks"));
        assert!(result.content.contains("coder (role: specialist) — Write code"));
    }
}
