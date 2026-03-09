use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::team::{self, normalize_team_name, validate_team_name};
use serde_json::Value;
use std::path::PathBuf;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct DeleteTeamTool {
    pub home_dir: PathBuf,
}

#[async_trait]
impl Tool for DeleteTeamTool {
    fn name(&self) -> &str {
        "delete_team"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "delete_team".to_string(),
            description: "Delete a team definition and all its data (workspace, config). \
                This cannot be undone."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the team to delete"
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

        let name = normalize_team_name(&raw_name);

        if let Err(e) = validate_team_name(&name) {
            return Ok(ToolOutput::error(format!("Invalid team name: {e}")));
        }

        if !team::team_exists(&self.home_dir, &name) {
            return Ok(ToolOutput::error(format!("Team '{name}' does not exist.")));
        }

        let team_dir = team::team_dir(&self.home_dir, &name);
        if let Err(e) = std::fs::remove_dir_all(&team_dir) {
            return Ok(ToolOutput::error(format!(
                "Failed to delete team directory: {e}"
            )));
        }

        Ok(ToolOutput::success(format!(
            "Deleted team '{name}' and all its data."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::fs;

    #[tokio::test]
    async fn test_delete_team_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DeleteTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(serde_json::json!({"name": "nonexistent"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("does not exist"));
    }

    #[tokio::test]
    async fn test_delete_team_success() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DeleteTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        // Create a team directory with team.toml and workspace
        let dir = team::team_dir(tmp.path(), "my-team");
        fs::create_dir_all(dir.join("workspace")).unwrap();
        fs::write(dir.join("team.toml"), "[team]\nname = \"my-team\"\norchestrator = \"a\"\n[[agents]]\nname = \"a\"\n[[agents]]\nname = \"b\"\n").unwrap();
        assert!(team::team_exists(tmp.path(), "my-team"));

        let result = tool
            .execute(serde_json::json!({"name": "my-team"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "Expected success, got: {}", result.content);
        assert!(result.content.contains("Deleted team 'my-team'"));
        assert!(!team::team_exists(tmp.path(), "my-team"));
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn test_delete_team_normalizes_name() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DeleteTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        // Create team with lowercase name
        let dir = team::team_dir(tmp.path(), "my-team");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("team.toml"), "[team]\nname = \"my-team\"\norchestrator = \"a\"\n[[agents]]\nname = \"a\"\n[[agents]]\nname = \"b\"\n").unwrap();

        let result = tool
            .execute(serde_json::json!({"name": "  My-Team  "}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "Expected success, got: {}", result.content);
        assert!(!team::team_exists(tmp.path(), "my-team"));
    }

    #[tokio::test]
    async fn test_delete_team_empty_name() {
        let tmp = tempfile::tempdir().unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = DeleteTeamTool {
            home_dir: tmp.path().to_path_buf(),
        };

        let result = tool
            .execute(serde_json::json!({"name": ""}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("cannot be empty"));
    }
}
