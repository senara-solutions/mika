use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::bundled_skills::is_bundled_skill;
use crate::skills::marketplace::is_marketplace_skill;

pub struct ListSkillsTool;

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
        let entries = crate::skills::index::scan_skills_dir(&skills_dir);

        if entries.is_empty() {
            return Ok(ToolOutput::success("No skills installed."));
        }

        let mut output = format!("Installed skills ({}):\n", entries.len());
        for entry in &entries {
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
            let tools_count = entry.skill_tools.len();
            let name = &entry.manifest.skill.name;
            let origin = if is_bundled_skill(name) {
                " [built-in]"
            } else if is_marketplace_skill(ctx.home_dir, name) {
                " [marketplace]"
            } else {
                " [custom]"
            };

            output.push_str(&format!(
                "- {} ({}){} — {}{}\n  Keywords: {}\n  Tools: {}\n",
                entry.manifest.skill.name,
                status,
                origin,
                entry.manifest.skill.description,
                always_on,
                keywords,
                tools_count,
            ));
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
