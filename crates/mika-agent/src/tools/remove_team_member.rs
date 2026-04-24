use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::team::{self, normalize_team_name, validate_team_name};
use serde_json::Value;
use std::path::PathBuf;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct RemoveTeamMemberTool {
    pub home_dir: PathBuf,
}

#[async_trait]
impl Tool for RemoveTeamMemberTool {
    fn name(&self) -> &str {
        "remove_team_member"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "remove_team_member".to_string(),
            description: "Remove a single agent from an existing team. Cannot remove the \
                orchestrator — reassign the orchestrator via update_team first."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "team_name": {
                        "type": "string",
                        "description": "Name of the team to remove the member from"
                    },
                    "agent_name": {
                        "type": "string",
                        "description": "Name of the agent to remove"
                    }
                },
                "required": ["team_name", "agent_name"]
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

        // Validate agent_name
        let agent_name = input["agent_name"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if agent_name.is_empty() {
            return Ok(ToolOutput::error(
                "'agent_name' is required and cannot be empty.",
            ));
        }
        if agent_name.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'agent_name' exceeds maximum length of {MAX_INPUT_LEN} characters."
            )));
        }

        if !team::team_exists(&self.home_dir, &team_name) {
            return Ok(ToolOutput::error(format!(
                "Team '{team_name}' does not exist."
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

        // Check agent is a member
        if !def.agents.iter().any(|a| a.name == agent_name) {
            return Ok(ToolOutput::error(format!(
                "'{agent_name}' is not a member of '{team_name}'."
            )));
        }

        // Orchestrator guard
        if def.team.orchestrator == agent_name {
            return Ok(ToolOutput::error(format!(
                "'{agent_name}' is the orchestrator of '{team_name}'. \
                 Reassign the orchestrator via `update_team` before removing this agent."
            )));
        }

        // Min-size check
        if def.agents.len() - 1 < 2 {
            return Ok(ToolOutput::error(format!(
                "Removing '{agent_name}' would leave '{team_name}' with fewer than 2 members."
            )));
        }

        // Remove the agent
        def.agents.retain(|a| a.name != agent_name);

        // Final validation
        if let Err(e) = team::validate_team(&self.home_dir, &def) {
            return Ok(ToolOutput::error(format!(
                "Team '{team_name}' has an invalid roster — {e}. \
                 Fix this first with `update_team`, then retry the removal."
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
            "Removed agent '{}' from team '{}' ({} members remaining).",
            agent_name,
            team_name,
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
    async fn test_remove_team_member_happy_path() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RemoveTeamMemberTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_agent(tmp.path(), "c");
        create_team_fs(
            tmp.path(),
            "my-team",
            "a",
            &[
                ("a", "lead", "Lead"),
                ("b", "worker", "Work"),
                ("c", "reviewer", "Review"),
            ],
        );

        let result = tool
            .execute(
                serde_json::json!({"team_name": "my-team", "agent_name": "c"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(
            !result.is_error,
            "Expected success, got: {}",
            result.content
        );
        assert!(result.content.contains("Removed agent 'c'"));
        assert!(result.content.contains("2 members remaining"));

        let def = team::load_team(tmp.path(), "my-team").unwrap();
        assert_eq!(def.agents.len(), 2);
        assert!(!def.agents.iter().any(|a| a.name == "c"));
    }

    #[tokio::test]
    async fn test_remove_team_member_not_a_member() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RemoveTeamMemberTool {
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
                serde_json::json!({"team_name": "my-team", "agent_name": "ghost"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("is not a member"));
    }

    #[tokio::test]
    async fn test_remove_team_member_is_orchestrator() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RemoveTeamMemberTool {
            home_dir: tmp.path().to_path_buf(),
        };

        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_agent(tmp.path(), "c");
        create_team_fs(
            tmp.path(),
            "my-team",
            "a",
            &[
                ("a", "lead", "Lead"),
                ("b", "worker", "Work"),
                ("c", "reviewer", "Review"),
            ],
        );

        let result = tool
            .execute(
                serde_json::json!({"team_name": "my-team", "agent_name": "a"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("orchestrator"));
        assert!(result.content.contains("update_team"));

        // File unchanged
        let def = team::load_team(tmp.path(), "my-team").unwrap();
        assert_eq!(def.agents.len(), 3);
    }

    #[tokio::test]
    async fn test_remove_team_member_would_drop_below_minimum() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RemoveTeamMemberTool {
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
                serde_json::json!({"team_name": "my-team", "agent_name": "b"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("fewer than 2 members"));

        // File unchanged
        let def = team::load_team(tmp.path(), "my-team").unwrap();
        assert_eq!(def.agents.len(), 2);
    }

    #[tokio::test]
    async fn test_remove_team_member_team_not_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RemoveTeamMemberTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(
                serde_json::json!({"team_name": "nonexistent", "agent_name": "a"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("does not exist"));
    }

    #[tokio::test]
    async fn test_remove_team_member_preexisting_orphan() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RemoveTeamMemberTool {
            home_dir: tmp.path().to_path_buf(),
        };

        // Create agents a, b, c — but NOT d
        create_agent(tmp.path(), "a");
        create_agent(tmp.path(), "b");
        create_agent(tmp.path(), "c");

        // Manually write a team.toml with an orphan (d doesn't exist globally)
        let dir = team::team_dir(tmp.path(), "my-team");
        fs::create_dir_all(dir.join("workspace")).unwrap();
        fs::write(
            dir.join("team.toml"),
            r#"[team]
name = "my-team"
orchestrator = "a"

[[agents]]
name = "a"
role = "lead"
mandate = "Lead"

[[agents]]
name = "b"
role = "worker"
mandate = "Work"

[[agents]]
name = "c"
role = "reviewer"
mandate = "Review"

[[agents]]
name = "d"
role = "ghost"
mandate = "Gone"

[flow]
max_iterations = 3
"#,
        )
        .unwrap();

        // Try to remove "c" — should fail because validate_team catches orphan "d"
        let result = tool
            .execute(
                serde_json::json!({"team_name": "my-team", "agent_name": "c"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("invalid roster"));
        assert!(result.content.contains("update_team"));

        // File unchanged — orphan is surfaced, not silently tolerated
        let def = team::load_team(tmp.path(), "my-team").unwrap();
        assert_eq!(def.agents.len(), 4);
    }
}
