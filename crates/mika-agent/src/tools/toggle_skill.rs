use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::sync::atomic::Ordering;

use super::create_skill::{validate_skill_name, verify_skill_path};
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
                Changes take effect immediately on the next turn."
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

        let skills_dir = ctx.home_dir.join("skills");
        let skill_dir = skills_dir.join(name);
        if !skill_dir.exists() {
            return Ok(ToolOutput::error(format!("Skill '{name}' not found.")));
        }

        // Symlink guard: verify skill dir is actually inside skills_dir
        if let Err(e) = verify_skill_path(&skills_dir, &skill_dir) {
            return Ok(ToolOutput::error(e));
        }

        // Check current enabled state from DB overrides.
        let agent_id = ctx.db.agent_id();
        let overrides = ctx.db.get_skill_overrides(agent_id).await?;
        let current_override = overrides
            .iter()
            .find(|o| o.skill_name.eq_ignore_ascii_case(name));
        // Default is enabled (None or Some(true)).
        let currently_enabled = current_override.and_then(|o| o.enabled).unwrap_or(true);

        if enabled == currently_enabled {
            let state = if enabled { "enabled" } else { "disabled" };
            return Ok(ToolOutput::success(format!(
                "Skill '{name}' is already {state}."
            )));
        }

        // Write enabled state to DB.
        if let Err(e) = ctx.db.set_skill_enabled(agent_id, name, enabled).await {
            return Ok(ToolOutput::error(format!(
                "Failed to {} skill '{name}': {e}",
                if enabled { "enable" } else { "disable" }
            )));
        }

        ctx.skills_dirty.store(true, Ordering::Release);
        let state = if enabled { "enabled" } else { "disabled" };
        Ok(ToolOutput::success(format!(
            "Skill '{name}' {state}. Changes take effect immediately on the next turn."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::sync::atomic::AtomicBool;
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

    #[tokio::test]
    async fn test_disable_skill() {
        let (tmp, harness) = setup_with_skill("my-skill");
        let ctx = harness.ctx_with_home(tmp.path());
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
        // Verify DB state: override should exist with enabled = false.
        let overrides = ctx.db.get_skill_overrides(ctx.db.agent_id()).await.unwrap();
        let ov = overrides
            .iter()
            .find(|o| o.skill_name == "my-skill")
            .unwrap();
        assert_eq!(ov.enabled, Some(false));
    }

    #[tokio::test]
    async fn test_enable_disabled_skill() {
        let (tmp, harness) = setup_with_skill("my-skill");
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = ToggleSkillTool;
        // Pre-disable via DB
        ctx.db
            .set_skill_enabled(ctx.db.agent_id(), "my-skill", false)
            .await
            .unwrap();

        let result = tool
            .execute(
                serde_json::json!({"name": "my-skill", "enabled": true}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "Got: {}", result.content);
        assert!(result.content.contains("enabled"));
        // Verify DB state: override row should be deleted (default-equals-delete).
        let overrides = ctx.db.get_skill_overrides(ctx.db.agent_id()).await.unwrap();
        assert!(overrides.iter().all(|o| o.skill_name != "my-skill"));
    }

    #[tokio::test]
    async fn test_already_enabled() {
        let (tmp, harness) = setup_with_skill("my-skill");
        let ctx = harness.ctx_with_home(tmp.path());
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
        let ctx = harness.ctx_with_home(tmp.path());
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
        let ctx = harness.ctx_with_home(tmp.path());
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

    #[tokio::test]
    async fn test_disable_with_existing_always_on_preserves_it() {
        let (tmp, harness) = setup_with_skill("my-skill");
        let ctx = harness.ctx_with_home(tmp.path());
        let agent_id = ctx.db.agent_id();
        // Set always_on override first
        ctx.db
            .set_skill_override(agent_id, "my-skill", true)
            .await
            .unwrap();

        let tool = ToggleSkillTool;
        let result = tool
            .execute(
                serde_json::json!({"name": "my-skill", "enabled": false}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "Got: {}", result.content);
        // Both overrides should coexist
        let overrides = ctx.db.get_skill_overrides(agent_id).await.unwrap();
        let ov = overrides
            .iter()
            .find(|o| o.skill_name == "my-skill")
            .unwrap();
        assert_eq!(ov.enabled, Some(false));
        assert_eq!(ov.always_on, Some(true));
    }

    #[tokio::test]
    async fn test_already_disabled() {
        let (tmp, harness) = setup_with_skill("my-skill");
        // Use local AtomicBools to avoid race with concurrent tests (the shared
        // static in ctx_with_home is process-wide and set by other toggle_skill tests).
        let skills_dirty = AtomicBool::new(false);
        let pr_review_posted = AtomicBool::new(false);
        let ctx = ToolContext {
            db: &harness.db,
            session_id: "test-session",
            trace_id: "00000000000000000000000000000000",
            home_dir: tmp.path(),
            global_home_dir: None,
            core_memory_edit_count: &harness.counter,
            is_onboarding: false,
            message_sender: None,
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            skills_dirty: &skills_dirty,
            is_reflection: false,
            is_task_context: false,
            is_callback_turn: false,
            provider_name: "anthropic",
            model_name: "claude-sonnet-4-6",
            active_skill_paths: &[],
            max_tasks_per_session: 25,
            pr_review_posted: &pr_review_posted,
        };
        // Pre-disable via DB
        ctx.db
            .set_skill_enabled(ctx.db.agent_id(), "my-skill", false)
            .await
            .unwrap();

        let tool = ToggleSkillTool;
        let result = tool
            .execute(
                serde_json::json!({"name": "my-skill", "enabled": false}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("already disabled"));
        // skills_dirty should NOT be set for a no-op
        assert!(!skills_dirty.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn test_disable_sets_skills_dirty() {
        let (tmp, harness) = setup_with_skill("my-skill");
        // Use local AtomicBools to avoid race with concurrent tests.
        let skills_dirty = AtomicBool::new(false);
        let pr_review_posted = AtomicBool::new(false);
        let ctx = ToolContext {
            db: &harness.db,
            session_id: "test-session",
            trace_id: "00000000000000000000000000000000",
            home_dir: tmp.path(),
            global_home_dir: None,
            core_memory_edit_count: &harness.counter,
            is_onboarding: false,
            message_sender: None,
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            skills_dirty: &skills_dirty,
            is_reflection: false,
            is_task_context: false,
            is_callback_turn: false,
            provider_name: "anthropic",
            model_name: "claude-sonnet-4-6",
            active_skill_paths: &[],
            max_tasks_per_session: 25,
            pr_review_posted: &pr_review_posted,
        };
        let tool = ToggleSkillTool;

        tool.execute(
            serde_json::json!({"name": "my-skill", "enabled": false}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(skills_dirty.load(Ordering::Acquire));
    }
}
