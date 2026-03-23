use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

use std::path::Path;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::skills::manifest::{SkillInfo, SkillManifest, Triggers};

pub struct CreateSkillTool;

/// Maximum allowed length for skill name.
pub(crate) const MAX_SKILL_NAME_LEN: usize = 50;

/// Maximum number of keywords per skill.
pub(super) const MAX_KEYWORDS: usize = 50;

/// Maximum length of a single keyword.
pub(super) const MAX_KEYWORD_LEN: usize = 100;

/// Maximum number of dependencies per skill.
pub(super) const MAX_DEPENDENCIES: usize = 20;

/// Maximum length of a single dependency name.
pub(super) const MAX_DEPENDENCY_LEN: usize = 128;

/// Verify that a skill directory is actually inside the skills root.
/// Guards against symlink attacks where a skill name resolves outside the expected directory.
pub(crate) fn verify_skill_path(skills_dir: &Path, skill_dir: &Path) -> Result<(), String> {
    match (skills_dir.canonicalize(), skill_dir.canonicalize()) {
        (Ok(parent), Ok(child)) if child.starts_with(&parent) => Ok(()),
        (Ok(_), Ok(_)) => {
            Err("Skill directory escaped skills root (possible symlink attack).".into())
        }
        _ => Err("Failed to verify skill directory location.".into()),
    }
}

/// Validate a skill description: non-empty and within length limit.
pub(super) fn validate_description(desc: &str) -> Result<(), String> {
    if desc.is_empty() {
        return Err("Description cannot be empty.".to_string());
    }
    if desc.len() > MAX_INPUT_LEN {
        return Err(format!(
            "Description too long ({} chars, max {MAX_INPUT_LEN}).",
            desc.len()
        ));
    }
    Ok(())
}

/// Validate a system prompt: non-empty and within length limit.
pub(super) fn validate_system_prompt(prompt: &str) -> Result<(), String> {
    if prompt.is_empty() {
        return Err("System prompt cannot be empty.".to_string());
    }
    if prompt.len() > MAX_INPUT_LEN {
        return Err(format!(
            "System prompt too long ({} chars, max {MAX_INPUT_LEN}).",
            prompt.len()
        ));
    }
    Ok(())
}

/// Validate keywords: count and individual length limits.
pub(super) fn validate_keywords(keywords: &[String]) -> Result<(), String> {
    if keywords.len() > MAX_KEYWORDS {
        return Err(format!(
            "Too many keywords ({}, max {MAX_KEYWORDS}).",
            keywords.len()
        ));
    }
    if let Some(long) = keywords.iter().find(|k| k.len() > MAX_KEYWORD_LEN) {
        return Err(format!(
            "Keyword too long ({} chars, max {MAX_KEYWORD_LEN}).",
            long.len()
        ));
    }
    Ok(())
}

/// Validate dependencies: count, individual length, and non-empty entries.
pub(super) fn validate_dependencies(deps: &[String]) -> Result<(), String> {
    if deps.len() > MAX_DEPENDENCIES {
        return Err(format!(
            "Too many dependencies ({}, max {MAX_DEPENDENCIES}).",
            deps.len()
        ));
    }
    if let Some(empty) = deps.iter().find(|d| d.is_empty()) {
        let _ = empty;
        return Err("Dependency name cannot be empty.".to_string());
    }
    if let Some(long) = deps.iter().find(|d| d.len() > MAX_DEPENDENCY_LEN) {
        return Err(format!(
            "Dependency name too long ({} chars, max {MAX_DEPENDENCY_LEN}).",
            long.len()
        ));
    }
    Ok(())
}

/// Validate a skill name: non-empty, alphanumeric + hyphens only, no path traversal.
pub fn validate_skill_name(name: &str) -> Result<(), String> {
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
                when triggered by keywords. The skill will be available immediately on \
                the next turn. Note: this creates the skill scaffold only (manifest + prompt). \
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
                    },
                    "dependencies": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Other skill names this skill depends on (loaded when this skill is active, max 20)"
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
        let dependencies: Vec<String> = input["dependencies"]
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

        // Validate inputs
        if let Err(e) = validate_description(description) {
            return Ok(ToolOutput::error(e));
        }
        if let Err(e) = validate_system_prompt(system_prompt) {
            return Ok(ToolOutput::error(e));
        }
        if keywords.is_empty() && !always_on {
            return Ok(ToolOutput::error(
                "At least one keyword is required (or set always_on to true).",
            ));
        }
        if let Err(e) = validate_keywords(&keywords) {
            return Ok(ToolOutput::error(e));
        }
        if let Err(e) = validate_dependencies(&dependencies) {
            return Ok(ToolOutput::error(e));
        }

        let skills_dir = ctx.home_dir.join("skills");
        let skill_dir = skills_dir.join(name);

        // Check for existing skill
        if skill_dir.exists() {
            return Ok(ToolOutput::error(format!(
                "Skill '{name}' already exists. Choose a different name.",
            )));
        }

        // Create skill directory (only the leaf; skills_dir should already exist)
        if let Err(e) = std::fs::create_dir(&skill_dir) {
            return Ok(ToolOutput::error(format!(
                "Failed to create skill directory: {e}"
            )));
        }

        // Symlink guard: verify the created directory is actually inside skills_dir
        if let Err(e) = verify_skill_path(&skills_dir, &skill_dir) {
            let _ = std::fs::remove_dir_all(&skill_dir);
            return Ok(ToolOutput::error(e));
        }

        // Build manifest struct and serialize to TOML (avoids string interpolation injection)
        let manifest = SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: description.to_string(),
                version: "0.1.0".to_string(),
                always_on,
                timeout_secs: 30,
                dependencies,
                max_prompt_size: None,
            },
            triggers: Triggers { keywords },
            llm: Default::default(),
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

        ctx.skills_dirty.store(true, Ordering::Release);

        Ok(ToolOutput::success(format!(
            "Created skill '{name}'.\n\
             Files: skill.toml, system_prompt.md\n\
             The skill will be available immediately on the next turn.\n\
             To add custom tools, create a tools.json file in the skill directory.",
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
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = CreateSkillTool;
        let output = tool.execute(valid_input(), &ctx).await.unwrap();

        assert!(
            !output.is_error,
            "Expected success, got: {}",
            output.content
        );
        assert!(
            output.content.contains("Created skill 'test-skill'"),
            "Got: {}",
            output.content
        );

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
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = CreateSkillTool;
        tool.execute(valid_input(), &ctx).await.unwrap();

        // Verify the created skill is loadable by scan_skills_dir
        let scan = crate::skills::index::scan_skills_dir(&tmp.path().join("skills"));
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].manifest.skill.name, "test-skill");
        assert_eq!(scan.entries[0].keywords_lower, vec!["test", "example"]);
        assert_eq!(
            scan.entries[0].prompt_snippet,
            "You are testing the skill system."
        );
    }

    #[tokio::test]
    async fn test_create_skill_duplicate_name() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());
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
        let ctx = harness.ctx_with_home(tmp.path());
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
        let ctx = harness.ctx_with_home(tmp.path());
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
        let ctx = harness.ctx_with_home(tmp.path());
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
        let ctx = harness.ctx_with_home(tmp.path());
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

    #[tokio::test]
    async fn test_create_skill_with_dependencies() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = CreateSkillTool;

        let input = json!({
            "name": "dep-skill",
            "description": "Skill with deps",
            "keywords": ["test"],
            "system_prompt": "Has dependencies.",
            "dependencies": ["shell-exec", "web-search"]
        });
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(
            !output.is_error,
            "Expected success, got: {}",
            output.content
        );

        // Verify dependencies in manifest
        let skill_dir = tmp.path().join("skills/dep-skill");
        let toml_content = std::fs::read_to_string(skill_dir.join("skill.toml")).unwrap();
        let manifest: SkillManifest = toml::from_str(&toml_content).unwrap();
        assert_eq!(
            manifest.skill.dependencies,
            vec!["shell-exec", "web-search"]
        );
    }

    #[tokio::test]
    async fn test_create_skill_without_dependencies_defaults_empty() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = CreateSkillTool;

        let output = tool.execute(valid_input(), &ctx).await.unwrap();
        assert!(!output.is_error);

        let skill_dir = tmp.path().join("skills/test-skill");
        let toml_content = std::fs::read_to_string(skill_dir.join("skill.toml")).unwrap();
        let manifest: SkillManifest = toml::from_str(&toml_content).unwrap();
        assert!(manifest.skill.dependencies.is_empty());
    }

    #[tokio::test]
    async fn test_create_skill_too_many_dependencies() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = CreateSkillTool;

        let deps: Vec<String> = (0..21).map(|i| format!("dep-{i}")).collect();
        let input = json!({
            "name": "many-deps",
            "description": "Too many deps",
            "keywords": ["test"],
            "system_prompt": "prompt",
            "dependencies": deps
        });
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Too many dependencies"));
    }

    #[tokio::test]
    async fn test_create_skill_dependency_name_too_long() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = CreateSkillTool;

        let long_dep = "a".repeat(129);
        let input = json!({
            "name": "long-dep",
            "description": "Long dep name",
            "keywords": ["test"],
            "system_prompt": "prompt",
            "dependencies": [long_dep]
        });
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Dependency name too long"));
    }

    #[tokio::test]
    async fn test_create_skill_empty_dependency_name() {
        let (tmp, harness) = setup();
        let ctx = harness.ctx_with_home(tmp.path());
        let tool = CreateSkillTool;

        let input = json!({
            "name": "empty-dep",
            "description": "Empty dep",
            "keywords": ["test"],
            "system_prompt": "prompt",
            "dependencies": ["valid", ""]
        });
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Dependency name cannot be empty"));
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

    #[test]
    fn test_validate_dependencies() {
        // Valid
        assert!(validate_dependencies(&[]).is_ok());
        assert!(validate_dependencies(&["shell-exec".to_string()]).is_ok());

        // Too many
        let many: Vec<String> = (0..21).map(|i| format!("dep-{i}")).collect();
        assert!(validate_dependencies(&many).is_err());

        // Empty entry
        assert!(validate_dependencies(&["".to_string()]).is_err());

        // Too long entry
        assert!(validate_dependencies(&["a".repeat(129)]).is_err());

        // At limit (20 entries, 128 chars each)
        let at_limit: Vec<String> = (0..20).map(|i| format!("dep-{i}")).collect();
        assert!(validate_dependencies(&at_limit).is_ok());
        assert!(validate_dependencies(&["a".repeat(128)]).is_ok());
    }
}
