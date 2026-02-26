use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::{Value, json};

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::skills::manifest::{SkillInfo, SkillManifest, Triggers};

pub struct CreateSkillTool;

/// Maximum allowed length for skill name.
const MAX_SKILL_NAME_LEN: usize = 50;

/// Maximum number of keywords per skill.
const MAX_KEYWORDS: usize = 50;

/// Maximum length of a single keyword.
const MAX_KEYWORD_LEN: usize = 100;

/// Validate a skill name: non-empty, alphanumeric + hyphens only, no path traversal.
pub(super) fn validate_skill_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Skill name cannot be empty.".to_string());
    }
    if name.len() > MAX_SKILL_NAME_LEN {
        return Err(format!(
            "Skill name too long ({} chars, max {MAX_SKILL_NAME_LEN}).",
            name.len()
        ));
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("Skill name cannot contain path separators or '..'".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Skill name must contain only alphanumeric characters, hyphens, or underscores."
                .to_string(),
        );
    }
    Ok(())
}

#[async_trait]
impl Tool for CreateSkillTool {
    fn name(&self) -> &str {
        "create_skill"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_skill".to_string(),
            description: "Create a new skill with a manifest and system prompt snippet. \
                Skills extend your capabilities by injecting context into your system prompt \
                when triggered by keywords. The skill will be available after restarting \
                the conversation. Note: this creates the skill scaffold only (manifest + prompt). \
                To add custom tools with executable handlers, the user must edit the files manually."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Skill name (alphanumeric + hyphens/underscores, max 50 chars). Example: 'claude-relay'"
                    },
                    "description": {
                        "type": "string",
                        "description": "Brief description of what the skill does"
                    },
                    "keywords": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Keywords that trigger this skill when they appear in the user's message"
                    },
                    "system_prompt": {
                        "type": "string",
                        "description": "The prompt snippet injected into the system prompt when this skill is active. This is the core of the skill — it tells you how to behave when the skill triggers."
                    },
                    "always_on": {
                        "type": "boolean",
                        "description": "If true, this skill is always active regardless of keywords. Default: false"
                    }
                },
                "required": ["name", "description", "keywords", "system_prompt"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let name = input["name"].as_str().unwrap_or("").trim();
        let description = input["description"].as_str().unwrap_or("").trim();
        let system_prompt = input["system_prompt"].as_str().unwrap_or("").trim();
        let always_on = input["always_on"].as_bool().unwrap_or(false);
        let keywords: Vec<String> = input["keywords"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();

        // Validate name
        if let Err(e) = validate_skill_name(name) {
            return Ok(ToolOutput::error(e));
        }

        // Validate inputs not empty
        if description.is_empty() {
            return Ok(ToolOutput::error("Description cannot be empty."));
        }
        if system_prompt.is_empty() {
            return Ok(ToolOutput::error("System prompt cannot be empty."));
        }
        if keywords.is_empty() && !always_on {
            return Ok(ToolOutput::error(
                "At least one keyword is required (or set always_on to true).",
            ));
        }

        // Validate input lengths
        if description.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "Description too long ({} chars, max {MAX_INPUT_LEN}).",
                description.len()
            )));
        }
        if system_prompt.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "System prompt too long ({} chars, max {MAX_INPUT_LEN}).",
                system_prompt.len()
            )));
        }

        // Validate keyword count and individual lengths
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

        let skills_dir = ctx.home_dir.join("skills");
        let skill_dir = skills_dir.join(name);

        // Check for existing skill
        if skill_dir.exists() {
            return Ok(ToolOutput::error(format!(
                "Skill '{name}' already exists at {}. Choose a different name.",
                skill_dir.display()
            )));
        }

        // Create skill directory (only the leaf; skills_dir should already exist)
        if let Err(e) = std::fs::create_dir(&skill_dir) {
            return Ok(ToolOutput::error(format!(
                "Failed to create skill directory: {e}"
            )));
        }

        // Symlink guard: verify the created directory is actually inside skills_dir
        match (skills_dir.canonicalize(), skill_dir.canonicalize()) {
            (Ok(canonical_parent), Ok(canonical_child)) => {
                if !canonical_child.starts_with(&canonical_parent) {
                    let _ = std::fs::remove_dir_all(&skill_dir);
                    return Ok(ToolOutput::error(
                        "Skill directory escaped skills root (possible symlink attack).",
                    ));
                }
            }
            _ => {
                // If canonicalize fails, the directory doesn't exist as expected
                let _ = std::fs::remove_dir_all(&skill_dir);
                return Ok(ToolOutput::error(
                    "Failed to verify skill directory location.",
                ));
            }
        }

        // Build manifest struct and serialize to TOML (avoids string interpolation injection)
        let manifest = SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: description.to_string(),
                version: "0.1.0".to_string(),
                always_on,
                timeout_secs: 30,
            },
            triggers: Triggers { keywords },
        };

        let skill_toml = match toml::to_string_pretty(&manifest) {
            Ok(s) => s,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&skill_dir);
                return Ok(ToolOutput::error(format!(
                    "Failed to serialize skill manifest: {e}"
                )));
            }
        };

        if let Err(e) = std::fs::write(skill_dir.join("skill.toml"), &skill_toml) {
            // Clean up on failure
            let _ = std::fs::remove_dir_all(&skill_dir);
            return Ok(ToolOutput::error(format!(
                "Failed to write skill.toml: {e}"
            )));
        }

        // Write system_prompt.md
        if let Err(e) = std::fs::write(skill_dir.join("system_prompt.md"), system_prompt) {
            let _ = std::fs::remove_dir_all(&skill_dir);
            return Ok(ToolOutput::error(format!(
                "Failed to write system_prompt.md: {e}"
            )));
        }

        Ok(ToolOutput::success(format!(
            "Created skill '{name}' at {}\n\
             Files: skill.toml, system_prompt.md\n\
             The skill will be available after restarting the conversation.\n\
             To add custom tools, create a tools.json file in the skill directory.",
            skill_dir.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use tempfile::TempDir;

    fn setup() -> (TempDir, TestHarness) {
        let tmp = TempDir::new().unwrap();
        // Create skills directory (parent must exist for create_dir in the tool)
        std::fs::create_dir_all(tmp.path().join("skills")).unwrap();
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

    fn valid_input() -> Value {
        json!({
            "name": "test-skill",
            "description": "A test skill",
            "keywords": ["test", "example"],
            "system_prompt": "You are testing the skill system."
        })
    }

    #[tokio::test]
    async fn test_create_skill_success() {
        let (tmp, harness) = setup();
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = CreateSkillTool;
        let output = tool.execute(valid_input(), &ctx).await.unwrap();

        assert!(
            !output.is_error,
            "Expected success, got: {}",
            output.content
        );
        assert!(output.content.contains("Created skill 'test-skill'"));

        // Verify files exist
        let skill_dir = tmp.path().join("skills/test-skill");
        assert!(skill_dir.join("skill.toml").exists());
        assert!(skill_dir.join("system_prompt.md").exists());

        // Verify skill.toml is valid TOML and roundtrips correctly
        let toml_content = std::fs::read_to_string(skill_dir.join("skill.toml")).unwrap();
        let manifest: SkillManifest = toml::from_str(&toml_content).unwrap();
        assert_eq!(manifest.skill.name, "test-skill");
        assert_eq!(manifest.skill.description, "A test skill");

        // Verify system_prompt.md content
        let prompt = std::fs::read_to_string(skill_dir.join("system_prompt.md")).unwrap();
        assert_eq!(prompt, "You are testing the skill system.");
    }

    #[tokio::test]
    async fn test_create_skill_loadable_by_scanner() {
        let (tmp, harness) = setup();
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = CreateSkillTool;
        tool.execute(valid_input(), &ctx).await.unwrap();

        // Verify the created skill is loadable by scan_skills_dir
        let entries = crate::skills::index::scan_skills_dir(&tmp.path().join("skills"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.skill.name, "test-skill");
        assert_eq!(entries[0].keywords_lower, vec!["test", "example"]);
        assert_eq!(
            entries[0].prompt_snippet,
            "You are testing the skill system."
        );
    }

    #[tokio::test]
    async fn test_create_skill_duplicate_name() {
        let (tmp, harness) = setup();
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = CreateSkillTool;

        // Create once
        let output = tool.execute(valid_input(), &ctx).await.unwrap();
        assert!(!output.is_error);

        // Create again — should fail
        let output = tool.execute(valid_input(), &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("already exists"));
    }

    #[tokio::test]
    async fn test_create_skill_invalid_names() {
        let (tmp, harness) = setup();
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = CreateSkillTool;

        // Empty name
        let input =
            json!({"name": "", "description": "d", "keywords": ["k"], "system_prompt": "p"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("empty"));

        // Path traversal
        let input =
            json!({"name": "../evil", "description": "d", "keywords": ["k"], "system_prompt": "p"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("path separators"));

        // Special characters
        let input =
            json!({"name": "sk!ll", "description": "d", "keywords": ["k"], "system_prompt": "p"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("alphanumeric"));

        // Too long
        let long_name = "a".repeat(51);
        let input =
            json!({"name": long_name, "description": "d", "keywords": ["k"], "system_prompt": "p"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("too long"));
    }

    #[tokio::test]
    async fn test_create_skill_empty_inputs() {
        let (tmp, harness) = setup();
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = CreateSkillTool;

        // Empty description
        let input =
            json!({"name": "sk", "description": "", "keywords": ["k"], "system_prompt": "p"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Description cannot be empty"));

        // Empty system_prompt
        let input =
            json!({"name": "sk", "description": "d", "keywords": ["k"], "system_prompt": ""});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("System prompt cannot be empty"));

        // Empty keywords (and not always_on)
        let input = json!({"name": "sk", "description": "d", "keywords": [], "system_prompt": "p"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("keyword"));
    }

    #[tokio::test]
    async fn test_create_skill_always_on_no_keywords() {
        let (tmp, harness) = setup();
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = CreateSkillTool;

        // always_on=true should work with empty keywords
        let input = json!({
            "name": "always-skill",
            "description": "Always active",
            "keywords": [],
            "system_prompt": "Always present.",
            "always_on": true
        });
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(
            !output.is_error,
            "Expected success, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_create_skill_description_with_quotes() {
        let (tmp, harness) = setup();
        let ctx = ctx_with_home(&harness, tmp.path());
        let tool = CreateSkillTool;

        // Description with TOML-special characters should not break serialization
        let input = json!({
            "name": "quote-skill",
            "description": "She said \"hello\" and it's fine",
            "keywords": ["test"],
            "system_prompt": "Handle quotes."
        });
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(
            !output.is_error,
            "Expected success, got: {}",
            output.content
        );

        // Verify roundtrip: the TOML should parse back correctly
        let skill_dir = tmp.path().join("skills/quote-skill");
        let toml_content = std::fs::read_to_string(skill_dir.join("skill.toml")).unwrap();
        let manifest: SkillManifest = toml::from_str(&toml_content).unwrap();
        assert_eq!(
            manifest.skill.description,
            "She said \"hello\" and it's fine"
        );
    }

    #[test]
    fn test_validate_skill_name() {
        assert!(validate_skill_name("web-search").is_ok());
        assert!(validate_skill_name("my_skill").is_ok());
        assert!(validate_skill_name("skill123").is_ok());
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("../evil").is_err());
        assert!(validate_skill_name("path/sep").is_err());
        assert!(validate_skill_name("sp ace").is_err());
        assert!(validate_skill_name("sp!cial").is_err());
        assert!(validate_skill_name(&"a".repeat(51)).is_err());
    }
}
