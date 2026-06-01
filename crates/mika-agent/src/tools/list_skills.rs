use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::bundled_skills::is_bundled_skill;
use crate::skills::SkillRegistry;
use crate::skills::marketplace::get_marketplace_entry;

pub struct ListSkillsTool;

const SKIPPED_WARNING: &str =
    "Warning: {} skill(s) skipped due to errors. Run 'mika skills validate' for details.";

/// Format the skipped-skills warning line, or empty string if none skipped.
fn skipped_warning(count: usize) -> String {
    if count > 0 {
        SKIPPED_WARNING.replace("{}", &count.to_string())
    } else {
        String::new()
    }
}

#[async_trait]
impl Tool for ListSkillsTool {
    fn name(&self) -> &str {
        "list_skills"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_skills".to_string(),
            description: "List all installed skills with their status, keywords, and tool counts."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let skills_dir = ctx.home_dir.join("skills");
        let mut registry = SkillRegistry::from_dir(&skills_dir);

        // Apply identity allowlist then DB overrides so the listing reflects effective values
        let identity = crate::prompt::load_identity(ctx.home_dir);
        if let Some(ref allowlist) = identity.skills.allowlist {
            registry.apply_identity_allowlist(allowlist);
        }
        if let Ok(overrides) = ctx.db.get_skill_overrides(ctx.db.agent_id()).await {
            registry.apply_overrides(&overrides);
        }

        // Run load-time crash-protection to promote broken-handler/tools.json skills to skipped,
        // matching the startup paths (chat.rs, ask.rs, server/mod.rs).
        registry.apply_load_safety_check();
        registry.log_summary();

        let entries = registry.skills();

        if entries.is_empty() {
            let warning = skipped_warning(registry.skipped_count());
            if warning.is_empty() {
                return Ok(ToolOutput::success("No skills installed."));
            }
            return Ok(ToolOutput::success(format!(
                "No skills installed.\n\n{}\n",
                warning
            )));
        }

        let mut output = format!("Installed skills ({}):\n", entries.len());
        for entry in entries {
            let status = if entry.enabled { "enabled" } else { "disabled" };
            let keywords = if entry.manifest.triggers.keywords.is_empty() {
                "none".to_string()
            } else {
                entry.manifest.triggers.keywords.join(", ")
            };
            let always_on = if entry.manifest.skill.always_on {
                " [always-on]"
            } else {
                ""
            };
            let override_badge = if entry.has_override {
                " [override]"
            } else {
                ""
            };
            let tools_count = entry.skill_tools.len();
            let name = &entry.manifest.skill.name;
            let origin = if is_bundled_skill(name) {
                " [built-in]"
            } else if let Some(mp_entry) = get_marketplace_entry(ctx.home_dir, name) {
                if mp_entry.linked {
                    " [marketplace/linked]"
                } else {
                    " [marketplace]"
                }
            } else {
                " [custom]"
            };

            output.push_str(&format!(
                "- {} ({}){} — {}{}{}\n  Keywords: {}\n  Tools: {}\n",
                entry.manifest.skill.name,
                status,
                origin,
                entry.manifest.skill.description,
                always_on,
                override_badge,
                keywords,
                tools_count,
            ));
        }

        let warning = skipped_warning(registry.skipped_count());
        if !warning.is_empty() {
            output.push_str(&format!("\n{}\n", warning));
        }

        Ok(ToolOutput::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use tempfile::TempDir;

    fn setup() -> (TempDir, TestHarness) {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("skills")).unwrap();
        let harness = TestHarness::new();
        (tmp, harness)
    }

    #[tokio::test]
    async fn test_list_skills_empty() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = ListSkillsTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No skills installed"));
    }

    #[tokio::test]
    async fn test_list_skills_shows_entries() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());

        // Create a skill
        let skill_dir = tmp.path().join("skills/web-search");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            [triggers]
            keywords = ["search", "find"]
            "#,
        )
        .unwrap();

        let tool = ListSkillsTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("web-search"));
        assert!(result.content.contains("enabled"));
        assert!(result.content.contains("search, find"));
    }

    #[tokio::test]
    async fn test_list_skills_tags_builtin() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());

        // Create a skill with a bundled name
        let skill_dir = tmp.path().join("skills/tmux");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "tmux"
            description = "Terminal multiplexer"
            [triggers]
            keywords = ["tmux"]
            "#,
        )
        .unwrap();

        let tool = ListSkillsTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("[built-in]"),
            "expected [built-in] tag in: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_skills_tags_custom() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());

        // Create a skill with a non-bundled name
        let skill_dir = tmp.path().join("skills/my-custom-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "my-custom-skill"
            description = "A custom skill"
            [triggers]
            keywords = ["custom"]
            "#,
        )
        .unwrap();

        let tool = ListSkillsTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("[custom]"),
            "expected [custom] tag in: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_skills_no_warning_when_no_skipped() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());

        // Create a valid skill — no skipped entries
        let skill_dir = tmp.path().join("skills/valid-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "valid-skill"
            description = "A valid skill"
            [triggers]
            keywords = ["valid"]
            "#,
        )
        .unwrap();

        let tool = ListSkillsTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("valid-skill"));
        assert!(
            !result.content.contains("Warning"),
            "should not contain warning when no skills skipped: {}",
            result.content
        );
        assert!(
            !result.content.contains("skipped"),
            "should not mention skipped when none skipped: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_skills_shows_skipped_warning() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());

        // Create a valid skill
        let valid_dir = tmp.path().join("skills/valid-skill");
        std::fs::create_dir_all(&valid_dir).unwrap();
        std::fs::write(
            valid_dir.join("skill.toml"),
            r#"
            [skill]
            name = "valid-skill"
            description = "A valid skill"
            [triggers]
            keywords = ["valid"]
            "#,
        )
        .unwrap();

        // Create an invalid skill (malformed TOML triggers skip)
        let invalid_dir = tmp.path().join("skills/broken-skill");
        std::fs::create_dir_all(&invalid_dir).unwrap();
        std::fs::write(
            invalid_dir.join("skill.toml"),
            "this is not valid toml {{{{",
        )
        .unwrap();

        let tool = ListSkillsTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        // Valid skill still listed
        assert!(result.content.contains("valid-skill"));
        // Warning footer present
        assert!(
            result
                .content
                .contains("Warning: 1 skill(s) skipped due to errors"),
            "expected skipped warning in: {}",
            result.content
        );
        assert!(
            result.content.contains("mika skills validate"),
            "expected validate hint in: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_skills_all_skipped_shows_warning() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());

        // Only invalid skills — no valid ones
        let invalid_dir = tmp.path().join("skills/broken-skill");
        std::fs::create_dir_all(&invalid_dir).unwrap();
        std::fs::write(invalid_dir.join("skill.toml"), "not valid toml {{{{").unwrap();

        let tool = ListSkillsTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("No skills installed."),
            "expected 'No skills installed.' in: {}",
            result.content
        );
        assert!(
            result
                .content
                .contains("Warning: 1 skill(s) skipped due to errors"),
            "expected skipped warning in: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_skills_shows_disabled() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());

        let skill_dir = tmp.path().join("skills/disabled-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "disabled-skill"
            description = "A disabled skill"
            [triggers]
            keywords = ["test"]
            "#,
        )
        .unwrap();
        std::fs::write(skill_dir.join(".disabled"), "").unwrap();

        let tool = ListSkillsTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("disabled-skill"));
        assert!(result.content.contains("disabled"));
    }
}
