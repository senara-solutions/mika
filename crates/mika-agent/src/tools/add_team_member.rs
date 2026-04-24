use anyhow::Result;
use async_trait::async_trait;
use mika_common::agent;
use mika_common::claude::ToolDefinition;
use mika_common::team::{self, TeamAgent, normalize_team_name, validate_team_name};
use serde_json::Value;
use std::path::PathBuf;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct AddTeamMemberTool {
    pub home_dir: PathBuf,
}

#[async_trait]
impl Tool for AddTeamMemberTool {
    fn name(&self) -> &str {
        "add_team_member"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "add_team_member".to_string(),
            description: "Add a single agent to an existing team. For bulk roster changes, \
                use update_team instead."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "team_name": {
                        "type": "string",
                        "description": "Name of the team to add the member to"
                    },
                    "agent": {
                        "type": "object",
                        "description": "Agent to add to the team",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Existing agent name"
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
                "required": ["team_name", "agent"]
            }),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Validate and normalize team_name
        let raw_name = input["team_name"].as_str().unwrap_or("").trim().to_string();
        if raw_name.is_empty() {
            return Ok(ToolOutput::error(
                "'team_name' is required and cannot be empty.",
            ));
        }
        if raw_name.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'team_name' exceeds maximum length of {MAX_INPUT_LEN} characters."
            )));
        }

        let team_name = normalize_team_name(&raw_name);

        if let Err(e) = validate_team_name(&team_name) {
            return Ok(ToolOutput::error(format!("Invalid team name: {e}")));
        }

        if !team::team_exists(&self.home_dir, &team_name) {
            return Ok(ToolOutput::error(format!(
                "Team '{team_name}' does not exist."
            )));
        }

        // Validate agent fields
        let agent_val = &input["agent"];
        let agent_name = agent_val["name"].as_str().unwrap_or("").trim().to_string();
        if agent_name.is_empty() {
            return Ok(ToolOutput::error(
                "Agent 'name' is required and cannot be empty.",
            ));
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

        // Check agent exists globally
        if !agent::agent_exists(&self.home_dir, &agent_name) {
            return Ok(ToolOutput::error(format!(
                "Agent '{agent_name}' does not exist. Create it first with create_agent."
            )));
        }

        // Load existing team
        let mut def = match team::load_team(&self.home_dir, &team_name) {
            Ok(d) => d,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "Failed to load team definition: {e}"
                )));
            }
        };

        // Check agent is not already a member
        if def.agents.iter().any(|a| a.name == agent_name) {
            return Ok(ToolOutput::error(format!(
                "'{agent_name}' is already a member of '{team_name}'."
            )));
        }

        // Append new member
        def.agents.push(TeamAgent {
            name: agent_name.clone(),
            role: role.clone(),
            mandate,
        });

        // Final validation
        if let Err(e) = team::validate_team(&self.home_dir, &def) {
            return Ok(ToolOutput::error(format!(
                "Team '{team_name}' has an invalid roster after adding '{agent_name}': {e}. \
                 Fix the existing roster with `update_team`, then retry."
            )));
        }

        // Serialize and write
        let toml_content = match toml::to_string_pretty(&def) {
            Ok(content) => content,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "Failed to serialize team definition: {e}"
                )));
            }
        };

        let team_dir = team::team_dir(&self.home_dir, &team_name);
        if let Err(e) = std::fs::write(team_dir.join("team.toml"), &toml_content) {
            return Ok(ToolOutput::error(format!("Failed to write team.toml: {e}")));
        }

        Ok(ToolOutput::success(format!(
            "Added agent '{}' to team '{}' (role: {}). Team now has {} members.",
            agent_name,
            team_name,
            role,
            def.agents.len()
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
    fn create_team_fs(
        home_dir: &std::path::Path,
        name: &str,
        orchestrator: &str,
        agents: &[(&str, &str, &str)],
    ) {
        let dir = team::team_dir(home_dir, name);
        fs::create_dir_all(dir.join("workspace")).unwrap();

        let mut toml = format!("[team]\nname = \"{name}\"\norchestrator = \"{orchestrator}\"\n\n");
        for (aname, role, mandate) in agents {
            toml.push_str(&format!(
                "[[agents]]\nname = \"{aname}\"\nrole = \"{role}\"\nmandate = \"{mandate}\"\n\n"
            ));
        }
        toml.push_str("[flow]\nmax_iterations = 3\n");
        fs::write(dir.join("team.toml"), toml).unwrap();
    }

    #[tokio::test]
    async fn test_add_team_member_happy_path() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = AddTeamMemberTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_agent(tmp.path(), "c");
        create_team_fs(
            tmp.path(),
            "my-team",
            "a",
            &[("a", "lead", "Lead"), ("b", "worker", "Work")],
        );

        let result = tool
            .execute(
                serde_json::json!({
                    "team_name": "my-team",
                    "agent": {"name": "c", "role": "reviewer", "mandate": "Review code"}
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
        assert!(result.content.contains("Added agent 'c'"));
        assert!(result.content.contains("3 members"));

        let def = team::load_team(tmp.path(), "my-team").unwrap();
        assert_eq!(def.agents.len(), 3);
        assert_eq!(def.agents[2].name, "c");
        assert_eq!(def.agents[2].role, "reviewer");
        assert_eq!(def.agents[2].mandate, "Review code");
    }

    #[tokio::test]
    async fn test_add_team_member_already_a_member() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = AddTeamMemberTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_team_fs(
            tmp.path(),
            "my-team",
            "a",
            &[("a", "lead", "Lead"), ("b", "worker", "Work")],
        );

        let result = tool
            .execute(
                serde_json::json!({
                    "team_name": "my-team",
                    "agent": {"name": "b", "role": "reviewer", "mandate": "Review"}
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("already a member"));

        // File unchanged
        let def = team::load_team(tmp.path(), "my-team").unwrap();
        assert_eq!(def.agents.len(), 2);
    }

    #[tokio::test]
    async fn test_add_team_member_agent_not_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = AddTeamMemberTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_team_fs(
            tmp.path(),
            "my-team",
            "a",
            &[("a", "lead", "Lead"), ("b", "worker", "Work")],
        );

        let result = tool
            .execute(
                serde_json::json!({
                    "team_name": "my-team",
                    "agent": {"name": "ghost", "role": "reviewer", "mandate": "Review"}
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("does not exist"));
    }

    #[tokio::test]
    async fn test_add_team_member_team_not_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = AddTeamMemberTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "c");

        let result = tool
            .execute(
                serde_json::json!({
                    "team_name": "nonexistent",
                    "agent": {"name": "c", "role": "reviewer", "mandate": "Review"}
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("does not exist"));
    }

    #[tokio::test]
    async fn test_add_team_member_invalid_team_name() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = AddTeamMemberTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "team_name": "INVALID NAME!!!",
                    "agent": {"name": "c", "role": "reviewer", "mandate": "Review"}
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("Invalid team name"));
    }

    #[tokio::test]
    async fn test_add_team_member_empty_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = AddTeamMemberTool {
            home_dir: tmp.path().to_path_buf(),
        };

        // Set up team so we reach agent-field validation
        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_agent(tmp.path(), "c");
        create_team_fs(
            tmp.path(),
            "my-team",
            "a",
            &[("a", "lead", "Lead"), ("b", "worker", "Work")],
        );

        // Empty team_name
        let result = tool
            .execute(
                serde_json::json!({
                    "team_name": "",
                    "agent": {"name": "c", "role": "reviewer", "mandate": "Review"}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'team_name' is required"));

        // Empty agent name
        let result = tool
            .execute(
                serde_json::json!({
                    "team_name": "my-team",
                    "agent": {"name": "", "role": "reviewer", "mandate": "Review"}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'name' is required"));

        // Empty role
        let result = tool
            .execute(
                serde_json::json!({
                    "team_name": "my-team",
                    "agent": {"name": "c", "role": "", "mandate": "Review"}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'role' is required"));

        // Empty mandate
        let result = tool
            .execute(
                serde_json::json!({
                    "team_name": "my-team",
                    "agent": {"name": "c", "role": "reviewer", "mandate": ""}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'mandate' is required"));
    }
}
