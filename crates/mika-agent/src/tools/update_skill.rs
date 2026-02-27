use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::{Value, json};

use super::create_skill::{MAX_KEYWORDS, MAX_KEYWORD_LEN, validate_skill_name};
use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::skills::manifest::SkillManifest;

pub struct UpdateSkillTool;

#[async_trait]
impl Tool for UpdateSkillTool {
    fn name(&self) -> &str {
        "update_skill"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_skill".to_string(),
            description: "Update an existing skill's manifest or system prompt. \
                At least one optional field must be provided. \
                Changes take effect after restarting the conversation."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the existing skill to update"
                    },
                    "description": {
                        "type": "string",
                        "description": "New description for the skill"
                    },
                    "keywords": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "New keywords that trigger this skill"
                    },
                    "system_prompt": {
                        "type": "string",
                        "description": "New content for the skill's system prompt (system_prompt.md)"
                    },
                    "always_on": {
                        "type": "boolean",
                        "description": "New always_on value (true = always active regardless of keywords)"
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

        // Check that at least one update field is provided
        let has_description = input.get("description").is_some_and(|v| !v.is_null());
        let has_keywords = input.get("keywords").is_some_and(|v| !v.is_null());
        let has_system_prompt = input.get("system_prompt").is_some_and(|v| !v.is_null());
        let has_always_on = input.get("always_on").is_some_and(|v| !v.is_null());

        if !has_description && !has_keywords && !has_system_prompt && !has_always_on {
            return Ok(ToolOutput::error(
                "At least one field to update must be provided \
                 (description, keywords, system_prompt, or always_on).",
            ));
        }

        let skills_dir = ctx.home_dir.join("skills");
        let skill_dir = skills_dir.join(name);

        // Check skill exists
        if !skill_dir.exists() {
            return Ok(ToolOutput::error(format!(
                "Skill '{name}' not found at {}.",
                skill_dir.display()
            )));
        }

        // Symlink guard: verify skill dir is actually inside skills_dir
        match (skills_dir.canonicalize(), skill_dir.canonicalize()) {
            (Ok(canonical_parent), Ok(canonical_child)) => {
                if !canonical_child.starts_with(&canonical_parent) {
                    return Ok(ToolOutput::error(
                        "Skill directory escaped skills root (possible symlink attack).",
                    ));
                }
            }
            _ => {
                return Ok(ToolOutput::error(
                    "Failed to verify skill directory location.",
                ));
            }
        }

        let mut updated = Vec::new();

        // Update skill.toml fields if any manifest fields changed
        if has_description || has_keywords || has_always_on {
            let toml_path = skill_dir.join("skill.toml");
            let toml_content = match std::fs::read_to_string(&toml_path) {
                Ok(s) => s,
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to read skill.toml: {e}"
                    )));
                }
            };
            let mut manifest: SkillManifest = match toml::from_str(&toml_content) {
                Ok(m) => m,
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to parse skill.toml: {e}"
                    )));
                }
            };

            if has_description {
                let desc = input["description"].as_str().unwrap_or("").trim();
                if desc.is_empty() {
                    return Ok(ToolOutput::error("Description cannot be empty."));
                }
                if desc.len() > MAX_INPUT_LEN {
                    return Ok(ToolOutput::error(format!(
                        "Description too long ({} chars, max {MAX_INPUT_LEN}).",
                        desc.len()
                    )));
                }
                manifest.skill.description = desc.to_string();
                updated.push("description");
            }

            if has_keywords {
                let keywords: Vec<String> = input["keywords"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();

                if keywords.len() > MAX_KEYWORDS {
                    return Ok(ToolOutput::error(format!(
                        "Too many keywords ({}, max {MAX_KEYWORDS}).",
                        keywords.len()
                    )));
                }
                if let Some(long) = keywords.iter().find(|k| k.len() > MAX_KEYWORD_LEN) {
                    return Ok(ToolOutput::error(format!(
                        "Keyword too long ({} chars, max {MAX_KEYWORD_LEN}).",
                        long.len()
                    )));
                }

                manifest.triggers.keywords = keywords;
                updated.push("keywords");
            }

            if has_always_on {
                let always_on = match input["always_on"].as_bool() {
                    Some(v) => v,
                    None => {
                        return Ok(ToolOutput::error("'always_on' must be a boolean."));
                    }
                };
                manifest.skill.always_on = always_on;
                updated.push("always_on");
            }

            let new_toml = match toml::to_string_pretty(&manifest) {
                Ok(s) => s,
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to serialize skill manifest: {e}"
                    )));
                }
            };
            if let Err(e) = std::fs::write(&toml_path, &new_toml) {
                return Ok(ToolOutput::error(format!(
                    "Failed to write skill.toml: {e}"
                )));
            }
        }

        // Update system_prompt.md if provided
        if has_system_prompt {
            let prompt = input["system_prompt"].as_str().unwrap_or("").trim();
            if prompt.is_empty() {
                return Ok(ToolOutput::error("System prompt cannot be empty."));
            }
            if prompt.len() > MAX_INPUT_LEN {
                return Ok(ToolOutput::error(format!(
                    "System prompt too long ({} chars, max {MAX_INPUT_LEN}).",
                    prompt.len()
                )));
            }
            if let Err(e) = std::fs::write(skill_dir.join("system_prompt.md"), prompt) {
                return Ok(ToolOutput::error(format!(
                    "Failed to write system_prompt.md: {e}"
                )));
            }
            updated.push("system_prompt");
        }

        Ok(ToolOutput::success(format!(
            "Updated skill '{name}': changed {fields}.\n\
             Changes take effect after restarting the conversation.",
            fields = updated.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::SkillManifest;
    use crate::test_utils::test_helpers::TestHarness;
    use tempfile::TempDir;

    /// Create a temp dir with a skill already present.
    fn setup_with_skill(name: &str) -> (TempDir, TestHarness) {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"[skill]
name = "test-skill"
description = "Original description"
version = "0.1.0"
always_on = false
timeout_secs = 30

[triggers]
keywords = ["original"]
"#,
        )
        .unwrap();
        std::fs::write(
            skill_dir.join("system_prompt.md"),
            "Original system prompt.",
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

    fn read_manifest(tmp: &TempDir, name: &str) -> SkillManifest {
        let toml_content =
            std::fs::read_to_string(tmp.path().join("skills").join(name).join("skill.toml"))
                .unwrap();
        toml::from_str(&toml_content).unwrap()
    }

    fn read_prompt(tmp: &TempDir, name: &str) -> String {
        std::fs::read_to_string(
            tmp.path()
                .join("skills")
                .join(name)
                .join("system_prompt.md"),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn test_update_system_prompt_only() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        let output = tool
            .execute(
                json!({
                    "name": "test-skill",
                    "system_prompt": "New system prompt content."
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!output.is_error, "Got: {}", output.content);
        assert!(output.content.contains("system_prompt"));
        assert_eq!(read_prompt(&tmp, "test-skill"), "New system prompt content.");
        // Manifest should be unchanged
        let manifest = read_manifest(&tmp, "test-skill");
        assert_eq!(manifest.skill.description, "Original description");
    }

    #[tokio::test]
    async fn test_update_description_only() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        let output = tool
            .execute(
                json!({
                    "name": "test-skill",
                    "description": "Updated description"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!output.is_error, "Got: {}", output.content);
        assert!(output.content.contains("description"));
        let manifest = read_manifest(&tmp, "test-skill");
        assert_eq!(manifest.skill.description, "Updated description");
        // Prompt should be unchanged
        assert_eq!(read_prompt(&tmp, "test-skill"), "Original system prompt.");
    }

    #[tokio::test]
    async fn test_update_keywords_only() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        let output = tool
            .execute(
                json!({
                    "name": "test-skill",
                    "keywords": ["new-kw1", "new-kw2"]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!output.is_error, "Got: {}", output.content);
        assert!(output.content.contains("keywords"));
        let manifest = read_manifest(&tmp, "test-skill");
        assert_eq!(manifest.triggers.keywords, vec!["new-kw1", "new-kw2"]);
    }

    #[tokio::test]
    async fn test_update_always_on() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        let output = tool
            .execute(
                json!({
                    "name": "test-skill",
                    "always_on": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!output.is_error, "Got: {}", output.content);
        let manifest = read_manifest(&tmp, "test-skill");
        assert!(manifest.skill.always_on);
    }

    #[tokio::test]
    async fn test_update_multiple_fields() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        let output = tool
            .execute(
                json!({
                    "name": "test-skill",
                    "description": "Multi-update desc",
                    "keywords": ["alpha", "beta"],
                    "system_prompt": "Multi-update prompt.",
                    "always_on": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!output.is_error, "Got: {}", output.content);
        assert!(output.content.contains("description"));
        assert!(output.content.contains("keywords"));
        assert!(output.content.contains("system_prompt"));
        assert!(output.content.contains("always_on"));

        let manifest = read_manifest(&tmp, "test-skill");
        assert_eq!(manifest.skill.description, "Multi-update desc");
        assert_eq!(manifest.triggers.keywords, vec!["alpha", "beta"]);
        assert!(manifest.skill.always_on);
        assert_eq!(read_prompt(&tmp, "test-skill"), "Multi-update prompt.");
    }

    #[tokio::test]
    async fn test_error_skill_not_found() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        let output = tool
            .execute(
                json!({
                    "name": "nonexistent",
                    "description": "nope"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_error_no_fields_provided() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        let output = tool
            .execute(json!({"name": "test-skill"}), &ctx)
            .await
            .unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("At least one field"));
    }

    #[tokio::test]
    async fn test_error_invalid_skill_name() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        let output = tool
            .execute(
                json!({"name": "../evil", "description": "nope"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("path separators"));
    }

    #[tokio::test]
    async fn test_error_empty_system_prompt() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        let output = tool
            .execute(
                json!({"name": "test-skill", "system_prompt": ""}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("System prompt cannot be empty"));
    }

    #[tokio::test]
    async fn test_error_empty_description() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        let output = tool
            .execute(
                json!({"name": "test-skill", "description": "  "}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Description cannot be empty"));
    }

    #[tokio::test]
    async fn test_toml_roundtrip_preserves_fields() {
        let (tmp, harness) = setup_with_skill("test-skill");
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = UpdateSkillTool;

        // Update only description — version, timeout_secs, name should survive
        tool.execute(
            json!({"name": "test-skill", "description": "Roundtrip test"}),
            &ctx,
        )
        .await
        .unwrap();

        let manifest = read_manifest(&tmp, "test-skill");
        assert_eq!(manifest.skill.name, "test-skill");
        assert_eq!(manifest.skill.description, "Roundtrip test");
        assert_eq!(manifest.skill.version, "0.1.0");
        assert_eq!(manifest.skill.timeout_secs, 30);
        assert!(!manifest.skill.always_on);
        assert_eq!(manifest.triggers.keywords, vec!["original"]);
    }
}
