use anyhow::Result;
use async_trait::async_trait;
use mika_common::agent;
use mika_common::claude::ToolDefinition;
use mika_common::team::{
    self, TeamAgent, TeamDefinition, TeamFlow, TeamMeta, normalize_team_name, validate_team_name,
};
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct CreateTeamTool {
    pub home_dir: PathBuf,
}

#[async_trait]
impl Tool for CreateTeamTool {
    fn name(&self) -> &str {
        "create_team"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_team".to_string(),
            description: "Create a new team definition with the specified agents and flow. \
                All referenced agents must already exist. The orchestrator must be one of the agents."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Team name (lowercase, hyphens allowed, e.g. 'inner-circle')"
                    },
                    "orchestrator": {
                        "type": "string",
                        "description": "Name of the agent that orchestrates the team (must be in the agents list)"
                    },
                    "agents": {
                        "type": "array",
                        "description": "List of agents in the team (minimum 2)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {
                                    "type": "string",
                                    "description": "Agent name (must already exist)"
                                },
                                "role": {
                                    "type": "string",
                                    "description": "Role of the agent in the team (e.g. 'orchestrator', 'specialist', 'critic')"
                                },
                                "mandate": {
                                    "type": "string",
                                    "description": "What this agent is responsible for in the team"
                                }
                            },
                            "required": ["name", "role", "mandate"]
                        }
                    },
                    "max_iterations": {
                        "type": "integer",
                        "description": "Maximum review iterations (1-10, default 3)"
                    }
                },
                "required": ["name", "orchestrator", "agents"]
            }),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Validate name
        let raw_name = input["name"].as_str().unwrap_or("").trim().to_string();
        if raw_name.is_empty() {
            return Ok(ToolOutput::error("'name' is required and cannot be empty."));
        }
        if raw_name.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'name' exceeds maximum length of {MAX_INPUT_LEN} characters."
            )));
        }

        let name = normalize_team_name(&raw_name);

        if let Err(e) = validate_team_name(&name) {
            return Ok(ToolOutput::error(format!("Invalid team name: {e}")));
        }

        if team::team_exists(&self.home_dir, &name) {
            return Ok(ToolOutput::error(format!("Team '{name}' already exists.")));
        }

        // Validate orchestrator
        let orchestrator = input["orchestrator"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if orchestrator.is_empty() {
            return Ok(ToolOutput::error(
                "'orchestrator' is required and cannot be empty.",
            ));
        }
        if orchestrator.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'orchestrator' exceeds maximum length of {MAX_INPUT_LEN} characters."
            )));
        }

        // Validate agents array
        let agents_val = match input["agents"].as_array() {
            Some(arr) => arr,
            None => {
                return Ok(ToolOutput::error(
                    "'agents' is required and must be an array.",
                ));
            }
        };

        if agents_val.len() < 2 {
            return Ok(ToolOutput::error("A team requires at least 2 agents."));
        }

        // Parse and validate each agent entry
        let mut agents = Vec::with_capacity(agents_val.len());
        let mut seen_names = HashSet::new();

        for (i, agent_val) in agents_val.iter().enumerate() {
            let agent_name = agent_val["name"].as_str().unwrap_or("").trim().to_string();
            if agent_name.is_empty() {
                return Ok(ToolOutput::error(format!(
                    "Agent at index {i}: 'name' is required and cannot be empty."
                )));
            }
            if agent_name.len() > MAX_INPUT_LEN {
                return Ok(ToolOutput::error(format!(
                    "Agent '{agent_name}': name exceeds maximum length."
                )));
            }

            let role = agent_val["role"].as_str().unwrap_or("").trim().to_string();
            if role.is_empty() {
                return Ok(ToolOutput::error(format!(
                    "Agent '{agent_name}': 'role' is required and cannot be empty."
                )));
            }
            if role.len() > MAX_INPUT_LEN {
                return Ok(ToolOutput::error(format!(
                    "Agent '{agent_name}': role exceeds maximum length."
                )));
            }

            let mandate = agent_val["mandate"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            if mandate.is_empty() {
                return Ok(ToolOutput::error(format!(
                    "Agent '{agent_name}': 'mandate' is required and cannot be empty."
                )));
            }
            if mandate.len() > MAX_INPUT_LEN {
                return Ok(ToolOutput::error(format!(
                    "Agent '{agent_name}': mandate exceeds maximum length."
                )));
            }

            if !seen_names.insert(agent_name.clone()) {
                return Ok(ToolOutput::error(format!(
                    "Duplicate agent name '{agent_name}' in agents list."
                )));
            }

            if !agent::agent_exists(&self.home_dir, &agent_name) {
                return Ok(ToolOutput::error(format!(
                    "Agent '{agent_name}' does not exist. Create it first with create_agent."
                )));
            }

            agents.push(TeamAgent {
                name: agent_name,
                role,
                mandate,
            });
        }

        // Orchestrator must be in the agents list
        if !agents.iter().any(|a| a.name == orchestrator) {
            return Ok(ToolOutput::error(format!(
                "Orchestrator '{orchestrator}' must be listed in the agents array."
            )));
        }

        // Orchestrator must exist as an agent
        if !agent::agent_exists(&self.home_dir, &orchestrator) {
            return Ok(ToolOutput::error(format!(
                "Orchestrator agent '{orchestrator}' does not exist."
            )));
        }

        // Validate max_iterations
        let max_iterations = match input.get("max_iterations") {
            Some(Value::Number(n)) => {
                let val = n.as_u64().unwrap_or(0) as u32;
                if !(1..=10).contains(&val) {
                    return Ok(ToolOutput::error(
                        "'max_iterations' must be between 1 and 10.",
                    ));
                }
                val
            }
            Some(Value::Null) | None => 3,
            _ => {
                return Ok(ToolOutput::error("'max_iterations' must be an integer."));
            }
        };

        // Build the team definition
        let def = TeamDefinition {
            team: TeamMeta {
                name: name.clone(),
                orchestrator,
            },
            agents,
            flow: TeamFlow { max_iterations },
        };

        // Create team directory and workspace
        let team_dir = team::team_dir(&self.home_dir, &name);
        if let Err(e) = std::fs::create_dir_all(&team_dir) {
            return Ok(ToolOutput::error(format!(
                "Failed to create team directory: {e}"
            )));
        }

        let workspace_dir = team::workspace_base_dir(&self.home_dir, &name);
        if let Err(e) = std::fs::create_dir_all(&workspace_dir) {
            return Ok(ToolOutput::error(format!(
                "Failed to create workspace directory: {e}"
            )));
        }

        // Serialize and write team.toml
        let toml_content = match toml::to_string_pretty(&def) {
            Ok(content) => content,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "Failed to serialize team definition: {e}"
                )));
            }
        };

        if let Err(e) = std::fs::write(team_dir.join("team.toml"), &toml_content) {
            return Ok(ToolOutput::error(format!("Failed to write team.toml: {e}")));
        }

        let agent_count = def.agents.len();
        Ok(ToolOutput::success(format!(
            "Team '{name}' created successfully with {agent_count} agents. \
             The team is now available for execution via `run_team`."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::fs;

    /// Helper to create a fake agent directory so agent_exists() returns true.
    fn create_agent(home_dir: &std::path::Path, name: &str) {
        let agent_dir = home_dir.join("agents").join(name);
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("config.toml"), "# config").unwrap();
    }

    #[tokio::test]
    async fn test_create_team_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "planner");
        create_agent(tmp.path(), "researcher");

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "dev-team",
                    "orchestrator": "planner",
                    "agents": [
                        {"name": "planner", "role": "orchestrator", "mandate": "Decompose goals"},
                        {"name": "researcher", "role": "specialist", "mandate": "Research topics"}
                    ]
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
        assert!(result.content.contains("dev-team"));
        assert!(result.content.contains("2 agents"));
        assert!(team::team_exists(tmp.path(), "dev-team"));
    }

    #[tokio::test]
    async fn test_create_team_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "main");
        create_agent(tmp.path(), "writer");

        tool.execute(
            serde_json::json!({
                "name": "content-team",
                "orchestrator": "main",
                "agents": [
                    {"name": "main", "role": "orchestrator", "mandate": "Coordinate"},
                    {"name": "writer", "role": "writer", "mandate": "Write content"}
                ],
                "max_iterations": 5
            }),
            &ctx,
        )
        .await
        .unwrap();

        // Verify we can load_team and get the same data back
        let def = team::load_team(tmp.path(), "content-team").unwrap();
        assert_eq!(def.team.name, "content-team");
        assert_eq!(def.team.orchestrator, "main");
        assert_eq!(def.agents.len(), 2);
        assert_eq!(def.agents[0].name, "main");
        assert_eq!(def.agents[0].role, "orchestrator");
        assert_eq!(def.agents[1].name, "writer");
        assert_eq!(def.agents[1].mandate, "Write content");
        assert_eq!(def.flow.max_iterations, 5);
    }

    #[tokio::test]
    async fn test_create_team_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");

        let input = serde_json::json!({
            "name": "my-team",
            "orchestrator": "a",
            "agents": [
                {"name": "a", "role": "lead", "mandate": "Lead"},
                {"name": "b", "role": "worker", "mandate": "Work"}
            ]
        });

        tool.execute(input.clone(), &ctx).await.unwrap();

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("already exists"));
    }

    #[tokio::test]
    async fn test_create_team_too_few_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "solo");

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "small-team",
                    "orchestrator": "solo",
                    "agents": [
                        {"name": "solo", "role": "all", "mandate": "Everything"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("at least 2"));
    }

    #[tokio::test]
    async fn test_create_team_duplicate_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "agent1");

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "dup-team",
                    "orchestrator": "agent1",
                    "agents": [
                        {"name": "agent1", "role": "lead", "mandate": "Lead"},
                        {"name": "agent1", "role": "worker", "mandate": "Work"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Duplicate"));
    }

    #[tokio::test]
    async fn test_create_team_nonexistent_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "real-agent");

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "bad-team",
                    "orchestrator": "real-agent",
                    "agents": [
                        {"name": "real-agent", "role": "lead", "mandate": "Lead"},
                        {"name": "ghost", "role": "worker", "mandate": "Work"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("does not exist"));
        assert!(result.content.contains("ghost"));
    }

    #[tokio::test]
    async fn test_create_team_orchestrator_not_in_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_agent(tmp.path(), "c");

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "mismatched",
                    "orchestrator": "c",
                    "agents": [
                        {"name": "a", "role": "lead", "mandate": "Lead"},
                        {"name": "b", "role": "worker", "mandate": "Work"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("must be listed"));
    }

    #[tokio::test]
    async fn test_create_team_missing_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        // Empty name
        let result = tool
            .execute(
                serde_json::json!({"name": "", "orchestrator": "a", "agents": []}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);

        // Empty orchestrator
        let result = tool
            .execute(
                serde_json::json!({"name": "team", "orchestrator": "", "agents": []}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);

        // Missing agent role
        create_agent(tmp.path(), "x");
        create_agent(tmp.path(), "y");
        let result = tool
            .execute(
                serde_json::json!({
                    "name": "team",
                    "orchestrator": "x",
                    "agents": [
                        {"name": "x", "role": "", "mandate": "Lead"},
                        {"name": "y", "role": "worker", "mandate": "Work"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("role"));

        // Missing agent mandate
        let result = tool
            .execute(
                serde_json::json!({
                    "name": "team",
                    "orchestrator": "x",
                    "agents": [
                        {"name": "x", "role": "lead", "mandate": "Lead"},
                        {"name": "y", "role": "worker", "mandate": ""}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("mandate"));
    }

    #[tokio::test]
    async fn test_create_team_max_iterations_bounds() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");

        // max_iterations = 0 (too low)
        let result = tool
            .execute(
                serde_json::json!({
                    "name": "iter-team",
                    "orchestrator": "a",
                    "agents": [
                        {"name": "a", "role": "lead", "mandate": "Lead"},
                        {"name": "b", "role": "worker", "mandate": "Work"}
                    ],
                    "max_iterations": 0
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("between 1 and 10"));

        // max_iterations = 11 (too high)
        let result = tool
            .execute(
                serde_json::json!({
                    "name": "iter-team",
                    "orchestrator": "a",
                    "agents": [
                        {"name": "a", "role": "lead", "mandate": "Lead"},
                        {"name": "b", "role": "worker", "mandate": "Work"}
                    ],
                    "max_iterations": 11
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("between 1 and 10"));
    }

    #[tokio::test]
    async fn test_create_team_workspace_created() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");

        tool.execute(
            serde_json::json!({
                "name": "ws-team",
                "orchestrator": "a",
                "agents": [
                    {"name": "a", "role": "lead", "mandate": "Lead"},
                    {"name": "b", "role": "worker", "mandate": "Work"}
                ]
            }),
            &ctx,
        )
        .await
        .unwrap();

        assert!(team::workspace_base_dir(tmp.path(), "ws-team").exists());
    }

    #[tokio::test]
    async fn test_create_team_normalizes_name() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "  My-Team  ",
                    "orchestrator": "a",
                    "agents": [
                        {"name": "a", "role": "lead", "mandate": "Lead"},
                        {"name": "b", "role": "worker", "mandate": "Work"}
                    ]
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
        assert!(team::team_exists(tmp.path(), "my-team"));
    }
}
