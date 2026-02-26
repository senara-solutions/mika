use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::create_skill::validate_skill_name;
use super::{Tool, ToolContext, ToolOutput};

pub struct ToggleSkillTool;

#[async_trait]
impl Tool for ToggleSkillTool {
    fn name(&self) -> &str {
        "toggle_skill"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "toggle_skill".to_string(),
            description: "Enable or disable a skill. Disabled skills are not loaded on startup. \
                Changes take effect after restarting the conversation."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to enable or disable"
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Set to true to enable, false to disable"
                    }
                },
                "required": ["name", "enabled"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let name = input["name"].as_str().unwrap_or("").trim();
        let enabled = match input["enabled"].as_bool() {
            Some(v) => v,
            None => return Ok(ToolOutput::error("'enabled' must be a boolean.")),
        };

        // Validate skill name
        if let Err(e) = validate_skill_name(name) {
            return Ok(ToolOutput::error(e));
        }

        let skill_dir = ctx.home_dir.join("skills").join(name);
        if !skill_dir.exists() {
            return Ok(ToolOutput::error(format!(
                "Skill '{name}' not found at {}.",
                skill_dir.display()
            )));
        }

        let disabled_marker = skill_dir.join(".disabled");
        let currently_enabled = !disabled_marker.exists();

        if enabled == currently_enabled {
            let state = if enabled { "enabled" } else { "disabled" };
            return Ok(ToolOutput::success(format!(
                "Skill '{name}' is already {state}."
            )));
        }

        if enabled {
            // Remove .disabled marker
            if let Err(e) = std::fs::remove_file(&disabled_marker) {
                return Ok(ToolOutput::error(format!(
                    "Failed to enable skill '{name}': {e}"
                )));
            }
            Ok(ToolOutput::success(format!(
                "Skill '{name}' enabled. Changes take effect after restarting the conversation."
            )))
        } else {
            // Create .disabled marker
            if let Err(e) = std::fs::write(&disabled_marker, "") {
                return Ok(ToolOutput::error(format!(
                    "Failed to disable skill '{name}': {e}"
                )));
            }
            Ok(ToolOutput::success(format!(
                "Skill '{name}' disabled. Changes take effect after restarting the conversation."
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use tempfile::TempDir;

    fn setup_with_skill(name: &str) -> (TempDir, TestHarness) {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            format!(
                r#"
            [skill]
            name = "{name}"
            description = "A test skill"
            [triggers]
            keywords = ["test"]
            "#
            ),
        )
        .unwrap();
        let harness = TestHarness::new();
        (tmp, harness)
    }

    fn ctx_with_home<'a>(harness: &'a TestHarness, home: &'a std::path::Path) -> ToolContext<'a> {
        ToolContext {
            db: &harness.db,
            session_id: "test-session",
            home_dir: home,
            core_memory_edit_count: &harness.counter,
            is_onboarding: false,
            message_sender: None,
            embedding_client: None,
        }
    }

    #[tokio::test]
    async fn test_disable_skill() {
        let (tmp, harness) = setup_with_skill("my-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = ToggleSkillTool;

        let result = tool
            .execute(
                serde_json::json!({"name": "my-skill", "enabled": false}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "Got: {}", result.content);
        assert!(result.content.contains("disabled"));
        assert!(tmp.path().join("skills/my-skill/.disabled").exists());
    }

    #[tokio::test]
    async fn test_enable_disabled_skill() {
        let (tmp, harness) = setup_with_skill("my-skill");
        // Pre-disable the skill
        std::fs::write(tmp.path().join("skills/my-skill/.disabled"), "").unwrap();

        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = ToggleSkillTool;

        let result = tool
            .execute(
                serde_json::json!({"name": "my-skill", "enabled": true}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "Got: {}", result.content);
        assert!(result.content.contains("enabled"));
        assert!(!tmp.path().join("skills/my-skill/.disabled").exists());
    }

    #[tokio::test]
    async fn test_already_enabled() {
        let (tmp, harness) = setup_with_skill("my-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = ToggleSkillTool;

        let result = tool
            .execute(
                serde_json::json!({"name": "my-skill", "enabled": true}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("already enabled"));
    }

    #[tokio::test]
    async fn test_nonexistent_skill() {
        let (tmp, harness) = setup_with_skill("my-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = ToggleSkillTool;

        let result = tool
            .execute(serde_json::json!({"name": "nope", "enabled": false}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_invalid_name() {
        let (tmp, harness) = setup_with_skill("my-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = ToggleSkillTool;

        let result = tool
            .execute(
                serde_json::json!({"name": "../evil", "enabled": false}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("path separators"));
    }
}
