use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::sync::atomic::Ordering;

use super::create_skill::{validate_skill_name, verify_skill_path};
use super::{Tool, ToolContext, ToolOutput};
use crate::bundled_skills::is_bundled_skill;
use crate::skills::marketplace;

pub struct DeleteSkillTool;

#[async_trait]
impl Tool for DeleteSkillTool {
    fn name(&self) -> &str {
        "delete_skill"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "delete_skill".to_string(),
            description: "Permanently delete a custom or marketplace-installed skill. Removes \
                the skill directory and all its files (manifest, prompt, tools, handlers). \
                Built-in skills cannot be deleted — use toggle_skill to disable them instead. \
                Changes take effect immediately on the next turn."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to delete"
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let name = input["name"].as_str().unwrap_or("").trim();

        // Validate skill name
        if let Err(e) = validate_skill_name(name) {
            return Ok(ToolOutput::error(e));
        }

        let skills_dir = ctx.home_dir.join("skills");
        let skill_dir = skills_dir.join(name);

        // Check skill exists
        if !skill_dir.exists() {
            return Ok(ToolOutput::error(format!("Skill '{name}' not found.")));
        }

        // Prevent deletion of built-in skills
        if is_bundled_skill(name) {
            return Ok(ToolOutput::error(format!(
                "Cannot delete built-in skill '{name}'. Use toggle_skill to disable it instead."
            )));
        }

        // Verify path safety (symlink escape prevention)
        if let Err(e) = verify_skill_path(&skills_dir, &skill_dir) {
            return Ok(ToolOutput::error(e));
        }

        // Delete the skill directory
        if let Err(e) = std::fs::remove_dir_all(&skill_dir) {
            return Ok(ToolOutput::error(format!("Failed to delete skill: {e}")));
        }

        // Clean up marketplace lock entry if this was a marketplace skill
        let mut lock = marketplace::read_lock(ctx.home_dir);
        if lock.skills.remove(name).is_some()
            && let Err(e) = marketplace::write_lock(ctx.home_dir, &lock)
        {
            tracing::warn!(skill = name, error = %e, "failed to update marketplace lock after delete");
        }

        // Clean up any DB override for this skill
        if let Err(e) = ctx.db.delete_skill_override(ctx.db.agent_id(), name).await {
            tracing::warn!(skill = name, error = %e, "failed to delete skill override after delete");
        }

        ctx.skills_dirty.store(true, Ordering::Release);

        Ok(ToolOutput::success(format!(
            "Deleted skill '{name}'.\nChanges take effect immediately on the next turn."
        )))
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
                r#"[skill]
name = "{name}"
description = "A test skill"

[triggers]
keywords = ["test"]
"#
            ),
        )
        .unwrap();
        std::fs::write(skill_dir.join("system_prompt.md"), "Test prompt.").unwrap();
        let harness = TestHarness::new();
        (tmp, harness)
    }

    #[tokio::test]
    async fn test_delete_skill_success() {
        let (tmp, harness) = setup_with_skill("my-skill");
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = DeleteSkillTool;

        let result = tool
            .execute(serde_json::json!({"name": "my-skill"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "Got: {}", result.content);
        assert!(result.content.contains("Deleted skill 'my-skill'"));
        assert!(result.content.contains("immediately"));

        // Verify directory is gone
        assert!(!tmp.path().join("skills/my-skill").exists());
    }

    #[tokio::test]
    async fn test_delete_skill_not_found() {
        let (tmp, harness) = setup_with_skill("other-skill");
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = DeleteSkillTool;

        let result = tool
            .execute(serde_json::json!({"name": "nonexistent"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
        // No path leakage
        assert!(!result.content.contains("skills/"));
    }

    #[tokio::test]
    async fn test_delete_skill_bundled_rejected() {
        let (tmp, harness) = setup_with_skill("tmux");
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = DeleteSkillTool;

        let result = tool
            .execute(serde_json::json!({"name": "tmux"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Cannot delete built-in skill"));
        assert!(result.content.contains("toggle_skill"));

        // Verify directory still exists
        assert!(tmp.path().join("skills/tmux").exists());
    }

    #[tokio::test]
    async fn test_delete_skill_invalid_name_empty() {
        let (tmp, harness) = setup_with_skill("some-skill");
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = DeleteSkillTool;

        let result = tool
            .execute(serde_json::json!({"name": ""}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("empty"));
    }

    #[tokio::test]
    async fn test_delete_skill_invalid_name_path_traversal() {
        let (tmp, harness) = setup_with_skill("some-skill");
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = DeleteSkillTool;

        let result = tool
            .execute(serde_json::json!({"name": "../evil"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("path separators"));
    }

    #[tokio::test]
    async fn test_delete_skill_invalid_name_special_chars() {
        let (tmp, harness) = setup_with_skill("some-skill");
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = DeleteSkillTool;

        let result = tool
            .execute(serde_json::json!({"name": "sk!ll"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("alphanumeric"));
    }

    #[tokio::test]
    async fn test_delete_skill_disabled_skill() {
        let (tmp, harness) = setup_with_skill("disabled-skill");
        // Add .disabled marker
        std::fs::write(tmp.path().join("skills/disabled-skill/.disabled"), "").unwrap();

        let ctx = harness.ctx_with_home(tmp.path());
        let tool = DeleteSkillTool;

        let result = tool
            .execute(serde_json::json!({"name": "disabled-skill"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "Got: {}", result.content);
        assert!(result.content.contains("Deleted"));

        // Verify directory is gone (including .disabled marker)
        assert!(!tmp.path().join("skills/disabled-skill").exists());
    }

    #[tokio::test]
    async fn test_delete_skill_with_subdirectories() {
        let (tmp, harness) = setup_with_skill("complex-skill");
        // Add handlers subdirectory with files
        let handlers_dir = tmp.path().join("skills/complex-skill/handlers");
        std::fs::create_dir_all(&handlers_dir).unwrap();
        std::fs::write(handlers_dir.join("run.sh"), "#!/bin/bash\necho hello").unwrap();
        std::fs::write(tmp.path().join("skills/complex-skill/tools.json"), "[]").unwrap();

        let ctx = harness.ctx_with_home(tmp.path());
        let tool = DeleteSkillTool;

        let result = tool
            .execute(serde_json::json!({"name": "complex-skill"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "Got: {}", result.content);
        assert!(!tmp.path().join("skills/complex-skill").exists());
    }

    #[tokio::test]
    async fn test_delete_skill_cleans_marketplace_lock() {
        use crate::skills::marketplace::{
            MarketplaceEntry, MarketplaceLock, read_lock, write_lock,
        };
        use std::collections::BTreeMap;

        let (tmp, harness) = setup_with_skill("market-skill");

        // Write a marketplace lock entry for this skill
        let mut skills = BTreeMap::new();
        skills.insert(
            "market-skill".to_string(),
            MarketplaceEntry {
                url: "https://github.com/user/mika-skill-market.git".to_string(),
                path: ".".to_string(),
                commit: "abc123".to_string(),
                linked: false,
                installed_as_dependency_of: vec![],
                installed_at: "2026-03-02T10:00:00Z".to_string(),
                updated_at: "2026-03-02T10:00:00Z".to_string(),
            },
        );
        let lock = MarketplaceLock { skills };
        write_lock(tmp.path(), &lock).unwrap();

        // Verify lock entry exists before delete
        let lock_before = read_lock(tmp.path());
        assert!(lock_before.skills.contains_key("market-skill"));

        // Delete the skill
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = DeleteSkillTool;
        let result = tool
            .execute(serde_json::json!({"name": "market-skill"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "Got: {}", result.content);
        assert!(result.content.contains("Deleted skill 'market-skill'"));

        // Verify skill directory is gone
        assert!(!tmp.path().join("skills/market-skill").exists());

        // Verify marketplace lock entry is removed
        let lock_after = read_lock(tmp.path());
        assert!(
            !lock_after.skills.contains_key("market-skill"),
            "marketplace lock entry should be removed after delete"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_delete_skill_symlink_escape() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Create an external target directory
        let external = TempDir::new().unwrap();
        std::fs::write(external.path().join("secret.txt"), "sensitive data").unwrap();

        // Create a symlink inside skills/ that points to the external directory
        std::os::unix::fs::symlink(external.path(), skills_dir.join("evil-link")).unwrap();

        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = DeleteSkillTool;

        let result = tool
            .execute(serde_json::json!({"name": "evil-link"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);

        // External directory must still exist with its contents
        assert!(external.path().join("secret.txt").exists());
    }
}
