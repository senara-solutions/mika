use anyhow::Result;
use async_trait::async_trait;
use mika_common::agent;
use mika_common::claude::ToolDefinition;
use mika_common::team::{
    self, TeamAgent, TeamFlow, TeamMeta, normalize_team_name, validate_team_name,
};
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct UpdateTeamTool {
    pub home_dir: PathBuf,
}

#[async_trait]
impl Tool for UpdateTeamTool {
    fn name(&self) -> &str {
        "update_team"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_team".to_string(),
            description: "Update an existing team definition. Only provided fields are changed; \
                omitted fields remain unchanged."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the team to update"
                    },
                    "orchestrator": {
                        "type": "string",
                        "description": "New orchestrator agent name (must be in the agents list)"
                    },
                    "agents": {
                        "type": "array",
                        "description": "New list of agents (replaces existing, minimum 2)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {
                                    "type": "string",
                                    "description": "Agent name (must already exist)"
                                },
                                "role": {
                                    "type": "string",
                                    "description": "Role of the agent in the team"
                                },
                                "mandate": {
                                    "type": "string",
                                    "description": "What this agent is responsible for"
                                }
                            },
                            "required": ["name", "role", "mandate"]
                        }
                    },
                    "max_iterations": {
                        "type": "integer",
                        "description": "Maximum review iterations (1-10)"
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Validate and normalize name
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

        if !team::team_exists(&self.home_dir, &name) {
            return Ok(ToolOutput::error(format!("Team '{name}' does not exist.")));
        }

        // Load existing definition
        let mut def = match team::load_team(&self.home_dir, &name) {
            Ok(d) => d,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "Failed to load team definition: {e}"
                )));
            }
        };

        let mut changes = Vec::new();

        // Check if anything was actually provided to update
        let has_orchestrator = input
            .get("orchestrator")
            .is_some_and(|v| !v.is_null());
        let has_agents = input.get("agents").is_some_and(|v| !v.is_null());
        let has_max_iterations = input
            .get("max_iterations")
            .is_some_and(|v| !v.is_null());

        if !has_orchestrator && !has_agents && !has_max_iterations {
            return Ok(ToolOutput::error(
                "No fields to update. Provide at least one of: orchestrator, agents, max_iterations.",
            ));
        }

        // Update agents first (before orchestrator validation, since orchestrator must be in agents list)
        if has_agents {
            let agents_val = match input["agents"].as_array() {
                Some(arr) => arr,
                None => {
                    return Ok(ToolOutput::error("'agents' must be an array."));
                }
            };

            if agents_val.len() < 2 {
                return Ok(ToolOutput::error(
                    "A team requires at least 2 agents.",
                ));
            }

            let mut agents = Vec::with_capacity(agents_val.len());
            let mut seen_names = HashSet::new();

            for (i, agent_val) in agents_val.iter().enumerate() {
                let agent_name = agent_val["name"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .to_string();
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

                let role = agent_val["role"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .to_string();
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

            def.agents = agents;
            changes.push(format!("{} agents", agents_val.len()));
        }

        // Update orchestrator
        if has_orchestrator {
            let orchestrator = input["orchestrator"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            if orchestrator.is_empty() {
                return Ok(ToolOutput::error(
                    "'orchestrator' cannot be empty.",
                ));
            }
            if orchestrator.len() > MAX_INPUT_LEN {
                return Ok(ToolOutput::error(format!(
                    "'orchestrator' exceeds maximum length of {MAX_INPUT_LEN} characters."
                )));
            }

            if !agent::agent_exists(&self.home_dir, &orchestrator) {
                return Ok(ToolOutput::error(format!(
                    "Orchestrator agent '{orchestrator}' does not exist."
                )));
            }

            if !def.agents.iter().any(|a| a.name == orchestrator) {
                return Ok(ToolOutput::error(format!(
                    "Orchestrator '{orchestrator}' must be listed in the agents array."
                )));
            }

            def.team = TeamMeta {
                name: name.clone(),
                orchestrator: orchestrator.clone(),
            };
            changes.push(format!("orchestrator → {orchestrator}"));
        }

        // Update max_iterations
        if has_max_iterations {
            match &input["max_iterations"] {
                Value::Number(n) => {
                    let val = n.as_u64().unwrap_or(0) as u32;
                    if !(1..=10).contains(&val) {
                        return Ok(ToolOutput::error(
                            "'max_iterations' must be between 1 and 10.",
                        ));
                    }
                    def.flow = TeamFlow {
                        max_iterations: val,
                    };
                    changes.push(format!("max_iterations → {val}"));
                }
                _ => {
                    return Ok(ToolOutput::error(
                        "'max_iterations' must be an integer.",
                    ));
                }
            }
        }

        // Final validation via validate_team
        if let Err(e) = team::validate_team(&self.home_dir, &def) {
            return Ok(ToolOutput::error(format!(
                "Invalid team definition after update: {e}"
            )));
        }

        // Serialize and write back
        let toml_content = match toml::to_string_pretty(&def) {
            Ok(content) => content,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "Failed to serialize team definition: {e}"
                )));
            }
        };

        let team_dir = team::team_dir(&self.home_dir, &name);
        if let Err(e) = std::fs::write(team_dir.join("team.toml"), &toml_content) {
            return Ok(ToolOutput::error(format!(
                "Failed to write team.toml: {e}"
            )));
        }

        Ok(ToolOutput::success(format!(
            "Updated team '{name}': {}.",
            changes.join(", ")
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

    /// Helper to create a team via filesystem.
    fn create_team_fs(home_dir: &std::path::Path, name: &str, orchestrator: &str, agents: &[(&str, &str, &str)]) {
        let dir = team::team_dir(home_dir, name);
        fs::create_dir_all(dir.join("workspace")).unwrap();

        let mut toml = format!(
            "[team]\nname = \"{name}\"\norchestrator = \"{orchestrator}\"\n\n"
        );
        for (aname, role, mandate) in agents {
            toml.push_str(&format!(
                "[[agents]]\nname = \"{aname}\"\nrole = \"{role}\"\nmandate = \"{mandate}\"\n\n"
            ));
        }
        toml.push_str("[flow]\nmax_iterations = 3\n");
        fs::write(dir.join("team.toml"), toml).unwrap();
    }

    #[tokio::test]
    async fn test_update_team_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(
                serde_json::json!({"name": "nonexistent", "max_iterations": 5}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("does not exist"));
    }

    #[tokio::test]
    async fn test_update_team_no_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_team_fs(tmp.path(), "my-team", "a", &[("a", "lead", "Lead"), ("b", "worker", "Work")]);

        let result = tool
            .execute(serde_json::json!({"name": "my-team"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("No fields to update"));
    }

    #[tokio::test]
    async fn test_update_team_orchestrator_only() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_team_fs(tmp.path(), "my-team", "a", &[("a", "lead", "Lead"), ("b", "worker", "Work")]);

        let result = tool
            .execute(
                serde_json::json!({"name": "my-team", "orchestrator": "b"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "Expected success, got: {}", result.content);
        assert!(result.content.contains("orchestrator → b"));

        let def = team::load_team(tmp.path(), "my-team").unwrap();
        assert_eq!(def.team.orchestrator, "b");
        // Agents unchanged
        assert_eq!(def.agents.len(), 2);
    }

    #[tokio::test]
    async fn test_update_team_agents_only() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_agent(tmp.path(), "c");
        create_team_fs(tmp.path(), "my-team", "a", &[("a", "lead", "Lead"), ("b", "worker", "Work")]);

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "my-team",
                    "agents": [
                        {"name": "a", "role": "lead", "mandate": "Lead"},
                        {"name": "b", "role": "worker", "mandate": "Work"},
                        {"name": "c", "role": "reviewer", "mandate": "Review"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "Expected success, got: {}", result.content);
        assert!(result.content.contains("3 agents"));

        let def = team::load_team(tmp.path(), "my-team").unwrap();
        assert_eq!(def.agents.len(), 3);
        assert_eq!(def.team.orchestrator, "a"); // unchanged
    }

    #[tokio::test]
    async fn test_update_team_max_iterations_only() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_team_fs(tmp.path(), "my-team", "a", &[("a", "lead", "Lead"), ("b", "worker", "Work")]);

        let result = tool
            .execute(
                serde_json::json!({"name": "my-team", "max_iterations": 7}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "Expected success, got: {}", result.content);
        assert!(result.content.contains("max_iterations → 7"));

        let def = team::load_team(tmp.path(), "my-team").unwrap();
        assert_eq!(def.flow.max_iterations, 7);
    }

    #[tokio::test]
    async fn test_update_team_all_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_agent(tmp.path(), "c");
        create_team_fs(tmp.path(), "my-team", "a", &[("a", "lead", "Lead"), ("b", "worker", "Work")]);

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "my-team",
                    "orchestrator": "c",
                    "agents": [
                        {"name": "b", "role": "specialist", "mandate": "Specialize"},
                        {"name": "c", "role": "orchestrator", "mandate": "Coordinate"}
                    ],
                    "max_iterations": 5
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "Expected success, got: {}", result.content);

        let def = team::load_team(tmp.path(), "my-team").unwrap();
        assert_eq!(def.team.orchestrator, "c");
        assert_eq!(def.agents.len(), 2);
        assert_eq!(def.agents[0].name, "b");
        assert_eq!(def.agents[1].name, "c");
        assert_eq!(def.flow.max_iterations, 5);
    }

    #[tokio::test]
    async fn test_update_team_invalid_orchestrator() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_team_fs(tmp.path(), "my-team", "a", &[("a", "lead", "Lead"), ("b", "worker", "Work")]);

        // Orchestrator not in agents list
        let result = tool
            .execute(
                serde_json::json!({"name": "my-team", "orchestrator": "b"}),
                &ctx,
            )
            .await
            .unwrap();
        // "b" IS in the agents list, so this should succeed
        assert!(!result.is_error);

        // Nonexistent orchestrator
        let result = tool
            .execute(
                serde_json::json!({"name": "my-team", "orchestrator": "ghost"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("does not exist"));
    }

    #[tokio::test]
    async fn test_update_team_orchestrator_not_in_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_agent(tmp.path(), "c");
        create_team_fs(tmp.path(), "my-team", "a", &[("a", "lead", "Lead"), ("b", "worker", "Work")]);

        // c exists as agent but is not in the team's agents list
        let result = tool
            .execute(
                serde_json::json!({"name": "my-team", "orchestrator": "c"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("must be listed"));
    }

    #[tokio::test]
    async fn test_update_team_max_iterations_bounds() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_team_fs(tmp.path(), "my-team", "a", &[("a", "lead", "Lead"), ("b", "worker", "Work")]);

        let result = tool
            .execute(
                serde_json::json!({"name": "my-team", "max_iterations": 0}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("between 1 and 10"));

        let result = tool
            .execute(
                serde_json::json!({"name": "my-team", "max_iterations": 11}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("between 1 and 10"));
    }

    #[tokio::test]
    async fn test_update_team_agent_validation() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_team_fs(tmp.path(), "my-team", "a", &[("a", "lead", "Lead"), ("b", "worker", "Work")]);

        // Too few agents
        let result = tool
            .execute(
                serde_json::json!({
                    "name": "my-team",
                    "agents": [
                        {"name": "a", "role": "lead", "mandate": "Lead"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("at least 2"));

        // Nonexistent agent
        let result = tool
            .execute(
                serde_json::json!({
                    "name": "my-team",
                    "agents": [
                        {"name": "a", "role": "lead", "mandate": "Lead"},
                        {"name": "ghost", "role": "worker", "mandate": "Work"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("does not exist"));

        // Duplicate agent
        let result = tool
            .execute(
                serde_json::json!({
                    "name": "my-team",
                    "agents": [
                        {"name": "a", "role": "lead", "mandate": "Lead"},
                        {"name": "a", "role": "worker", "mandate": "Work"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Duplicate"));
    }
}
