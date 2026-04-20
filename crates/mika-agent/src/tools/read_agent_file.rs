use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{
    SkillPathInfo, Tool, ToolContext, ToolOutput, resolve_agent_home, validate_and_resolve_path,
};
use crate::db::core_memory_section_names;

/// Maximum file size that can be read from the agent home directory (100 KB).
const MAX_READ_SIZE: u64 = 100 * 1024;

/// Check whether a path targets a core_memory section.
///
/// Returns `Some(section_name)` if the path matches a core_memory section,
/// `None` otherwise. Core memory is DB-backed and auto-injected into the
/// system prompt — it has no filesystem representation.
fn is_core_memory_path(path: &str) -> Option<&'static str> {
    // Normalize: strip leading ./ and ~/
    let normalized = path
        .strip_prefix("./")
        .or_else(|| path.strip_prefix("~/"))
        .unwrap_or(path);

    // Check for core_memory/ or core-memory/ prefix
    let after_prefix = normalized
        .strip_prefix("core_memory/")
        .or_else(|| normalized.strip_prefix("core-memory/"));

    if let Some(remainder) = after_prefix {
        // Extract the file stem (without .md extension)
        let stem = Path::new(remainder)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(remainder);
        for section in core_memory_section_names() {
            if stem == section {
                return Some(section);
            }
        }
        return None;
    }

    // Check for exact directory match: "core_memory" or "core-memory"
    if normalized == "core_memory" || normalized == "core-memory" {
        // Return a generic indicator — the path targets the core_memory directory itself
        return Some("core_memory");
    }

    // Check for bare section name (with or without .md extension)
    let stem = Path::new(normalized)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(normalized);
    // Only match if the full path IS the section name (not a substring or nested path)
    if normalized == stem || normalized == format!("{stem}.md") {
        for section in core_memory_section_names() {
            if stem == section {
                return Some(section);
            }
        }
    }

    None
}

/// Check whether a path targets a skill prompt file that is already loaded in context.
///
/// Returns `Some(skill_name)` if the path matches an active skill's `system_prompt.md`,
/// `None` otherwise. Skill prompts are pre-injected into the system prompt by the engine
/// — re-reading them wastes tokens.
fn is_active_skill_prompt<'a>(
    path: &str,
    active_skill_paths: &'a [SkillPathInfo],
) -> Option<&'a str> {
    if active_skill_paths.is_empty() {
        return None;
    }

    // Normalize: strip leading ./ and ~/
    let normalized = path
        .strip_prefix("./")
        .or_else(|| path.strip_prefix("~/"))
        .unwrap_or(path);

    for info in active_skill_paths {
        if normalized == info.prompt_relative_path {
            return Some(&info.skill_name);
        }
    }

    None
}

pub struct ReadAgentFileTool;

#[async_trait]
impl Tool for ReadAgentFileTool {
    fn name(&self) -> &str {
        "read_agent_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_agent_file".to_string(),
            description: "Read a file from the agent's home directory. Returns the file contents as text. Files larger than 100 KB are rejected. Does NOT access core_memory — core_memory sections (self_model, user_summary, etc.) are auto-injected into your system prompt and cannot be read as files. Active skill prompts (system_prompt.md) are also already in your context — do not re-read them. To modify core_memory, use update_core_memory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path within the agent's home directory (e.g., 'notes/todo.md' or '~/notes/todo.md')"
                    },
                    "agent": {
                        "type": "string",
                        "description": "Target agent name (orchestrator-only). Omit to use your own home directory."
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let path = input["path"].as_str().unwrap_or("");
        let agent_param = input["agent"].as_str();

        // Reject core_memory paths with a domain-specific error
        if let Some(section) = is_core_memory_path(path) {
            return Ok(ToolOutput::error(format!(
                "Path '{}' is not filesystem-accessible. core_memory sections ({}) \
                 are auto-injected into your system prompt on every turn — the content \
                 is already available in the 'Core Memory' block above. \
                 To modify core_memory, use the update_core_memory tool.",
                path,
                if section == "core_memory" {
                    core_memory_section_names().join(", ")
                } else {
                    format!("including '{section}'")
                }
            )));
        }

        // Reject paths targeting active skill prompts already in context
        if let Some(skill_name) = is_active_skill_prompt(path, ctx.active_skill_paths) {
            tracing::info!(
                tool = "read_agent_file",
                redirect_reason = "active_skill_prompt",
                matched_source = skill_name,
                "context redundancy redirect: skill prompt already in system prompt"
            );
            return Ok(ToolOutput::error(format!(
                "The file '{path}' is already loaded as the '{skill_name}' skill prompt \
                 in your system prompt context. Read it from the '<context type=\"skill\">' \
                 block above instead of re-fetching."
            )));
        }

        // Resolve base directory (own home or target agent's home)
        let base_dir = match resolve_agent_home(agent_param, ctx).await {
            Ok(dir) => dir,
            Err(e) => return Ok(e),
        };

        let full_path = match validate_and_resolve_path(path, &base_dir, false).await {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        // Reject symlinks at the target file (defense-in-depth)
        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Ok(ToolOutput::error("Symbolic links are not allowed."));
                }
            }
            Err(_) => {
                return Ok(ToolOutput::error(format!(
                    "File not found: '{}'",
                    full_path.display()
                )));
            }
        }

        // Check file size before reading
        match tokio::fs::metadata(&full_path).await {
            Ok(meta) => {
                if meta.len() > MAX_READ_SIZE {
                    return Ok(ToolOutput::error(format!(
                        "File too large ({} bytes). Maximum is {} bytes (100 KB).",
                        meta.len(),
                        MAX_READ_SIZE
                    )));
                }
            }
            Err(_) => {
                return Ok(ToolOutput::error(format!(
                    "File not found: '{}'",
                    full_path.display()
                )));
            }
        }

        match tokio::fs::read_to_string(&full_path).await {
            Ok(content) => Ok(ToolOutput::success(format!(
                "Contents of '{}':\n\n{content}",
                full_path.display()
            ))),
            Err(e) => Ok(ToolOutput::error(format!(
                "Failed to read '{}': {e}",
                full_path.display()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use crate::tools::MAX_INPUT_LEN;
    use std::fs;

    #[tokio::test]
    async fn test_read_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("notes.md"), "hello world").unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "notes.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("hello world"));
    }

    #[tokio::test]
    async fn test_read_nested_file() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join("sub")).unwrap();
        fs::write(home.join("sub").join("notes.md"), "nested content").unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "sub/notes.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("nested content"));
    }

    #[tokio::test]
    async fn test_read_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "missing.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_read_empty_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("required"));
    }

    #[tokio::test]
    async fn test_read_path_traversal_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "../../../etc/passwd" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_read_absolute_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "/etc/passwd" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Absolute paths are not allowed"));
    }

    #[tokio::test]
    async fn test_read_symlink_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let secret = tmp.path().join("secret.txt");
        fs::write(&secret, "top secret").unwrap();
        std::os::unix::fs::symlink(&secret, home.join("link.md")).unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "link.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Symbolic links are not allowed"));
    }

    #[tokio::test]
    async fn test_read_path_length_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let long_path = "a".repeat(MAX_INPUT_LEN + 1);
        let input = serde_json::json!({ "path": long_path });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("exceeds maximum length"));
    }

    #[tokio::test]
    async fn test_read_success_contains_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("file.md"), "data").unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "file.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        let expected = home.join("file.md");
        assert!(
            output.content.contains(&expected.display().to_string()),
            "expected absolute path '{}' in output: {}",
            expected.display(),
            output.content
        );
    }

    #[tokio::test]
    async fn test_read_tilde_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("notes.md"), "tilde content").unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "~/notes.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("tilde content"));
    }

    // --- Cross-agent file access tests ---

    /// Helper: create a global home with the default agent ("mika") and a target agent.
    fn setup_cross_agent_dirs(tmp: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
        let global = tmp.path().join("mika-home");
        let mika_home = global.join("agents").join("mika");
        let other_home = global.join("agents").join("work");
        fs::create_dir_all(&mika_home).unwrap();
        fs::create_dir_all(&other_home).unwrap();
        // agent_exists checks for config.toml
        fs::write(mika_home.join("config.toml"), "[config]").unwrap();
        fs::write(other_home.join("config.toml"), "model = \"sonnet\"").unwrap();
        fs::write(other_home.join("notes.md"), "work notes").unwrap();
        (global, mika_home)
    }

    #[tokio::test]
    async fn test_cross_agent_read() {
        let tmp = tempfile::tempdir().unwrap();
        let (global, mika_home) = setup_cross_agent_dirs(&tmp);

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new(); // default agent is "mika"
        let ctx = harness.ctx_with_home_and_global(&mika_home, &global);
        let input = serde_json::json!({ "path": "config.toml", "agent": "work" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("model = \"sonnet\""));
    }

    #[tokio::test]
    async fn test_cross_agent_read_blocked_non_orchestrator() {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().join("mika-home");
        let other_home = global.join("agents").join("work");
        let reader_home = global.join("agents").join("reader");
        fs::create_dir_all(&other_home).unwrap();
        fs::create_dir_all(&reader_home).unwrap();
        fs::write(other_home.join("config.toml"), "data").unwrap();
        fs::write(reader_home.join("config.toml"), "data").unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::with_agent("reader");
        let ctx = harness.ctx_with_home_and_global(&reader_home, &global);
        let input = serde_json::json!({ "path": "config.toml", "agent": "work" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("orchestrator"));
    }

    #[tokio::test]
    async fn test_cross_agent_read_agent_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let (global, mika_home) = setup_cross_agent_dirs(&tmp);

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home_and_global(&mika_home, &global);
        let input = serde_json::json!({ "path": "config.toml", "agent": "nonexistent" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("not found"));
        assert!(output.content.contains("work")); // lists available agents
    }

    #[tokio::test]
    async fn test_cross_agent_read_global_home_none() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home); // global_home_dir is None
        let input = serde_json::json!({ "path": "config.toml", "agent": "work" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("not available"));
    }

    #[tokio::test]
    async fn test_cross_agent_read_self_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let (global, mika_home) = setup_cross_agent_dirs(&tmp);
        fs::write(mika_home.join("notes.md"), "my notes").unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home_and_global(&mika_home, &global);
        let input = serde_json::json!({ "path": "notes.md", "agent": "mika" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("my notes"));
    }

    // --- Active skill prompt path rejection tests ---

    #[test]
    fn test_is_active_skill_prompt_match() {
        let paths = vec![SkillPathInfo {
            skill_name: "self-dev".to_string(),
            prompt_relative_path: "skills/self-dev/system_prompt.md".to_string(),
        }];
        assert_eq!(
            is_active_skill_prompt("skills/self-dev/system_prompt.md", &paths),
            Some("self-dev")
        );
    }

    #[test]
    fn test_is_active_skill_prompt_tilde_prefix() {
        let paths = vec![SkillPathInfo {
            skill_name: "self-dev".to_string(),
            prompt_relative_path: "skills/self-dev/system_prompt.md".to_string(),
        }];
        assert_eq!(
            is_active_skill_prompt("~/skills/self-dev/system_prompt.md", &paths),
            Some("self-dev")
        );
    }

    #[test]
    fn test_is_active_skill_prompt_dot_prefix() {
        let paths = vec![SkillPathInfo {
            skill_name: "self-dev".to_string(),
            prompt_relative_path: "skills/self-dev/system_prompt.md".to_string(),
        }];
        assert_eq!(
            is_active_skill_prompt("./skills/self-dev/system_prompt.md", &paths),
            Some("self-dev")
        );
    }

    #[test]
    fn test_is_active_skill_prompt_no_match() {
        let paths = vec![SkillPathInfo {
            skill_name: "self-dev".to_string(),
            prompt_relative_path: "skills/self-dev/system_prompt.md".to_string(),
        }];
        assert_eq!(is_active_skill_prompt("notes.md", &paths), None);
        assert_eq!(
            is_active_skill_prompt("skills/self-dev/handlers/run.sh", &paths),
            None
        );
        assert_eq!(
            is_active_skill_prompt("skills/other/system_prompt.md", &paths),
            None
        );
    }

    #[test]
    fn test_is_active_skill_prompt_empty_paths() {
        assert_eq!(
            is_active_skill_prompt("skills/self-dev/system_prompt.md", &[]),
            None
        );
    }

    #[tokio::test]
    async fn test_read_active_skill_prompt_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let skill_dir = home.join("skills").join("self-dev");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "skill prompt content").unwrap();

        let skill_paths = vec![SkillPathInfo {
            skill_name: "self-dev".to_string(),
            prompt_relative_path: "skills/self-dev/system_prompt.md".to_string(),
        }];

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let mut ctx = harness.ctx_with_home(&home);
        ctx.active_skill_paths = &skill_paths;
        let input = serde_json::json!({ "path": "skills/self-dev/system_prompt.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(
            output.content.contains("already loaded"),
            "expected skill prompt redirect, got: {}",
            output.content
        );
        assert!(
            output.content.contains("self-dev"),
            "expected skill name in error, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_read_non_skill_file_allowed_with_active_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("notes.md"), "my notes").unwrap();

        let skill_paths = vec![SkillPathInfo {
            skill_name: "self-dev".to_string(),
            prompt_relative_path: "skills/self-dev/system_prompt.md".to_string(),
        }];

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let mut ctx = harness.ctx_with_home(&home);
        ctx.active_skill_paths = &skill_paths;
        let input = serde_json::json!({ "path": "notes.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "should NOT reject: {}", output.content);
        assert!(output.content.contains("my notes"));
    }

    #[tokio::test]
    async fn test_core_memory_guard_fires_before_skill_prompt_guard() {
        // Core memory paths should be rejected by the core_memory guard,
        // not the skill prompt guard, even if active skills are loaded.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let skill_paths = vec![SkillPathInfo {
            skill_name: "self-dev".to_string(),
            prompt_relative_path: "skills/self-dev/system_prompt.md".to_string(),
        }];

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let mut ctx = harness.ctx_with_home(&home);
        ctx.active_skill_paths = &skill_paths;
        let input = serde_json::json!({ "path": "core_memory/self_model.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(
            output.content.contains("not filesystem-accessible"),
            "expected core_memory guard error, got: {}",
            output.content
        );
    }

    // --- Core memory path rejection tests ---

    #[test]
    fn test_is_core_memory_path_prefixed() {
        assert_eq!(
            is_core_memory_path("core_memory/self_model.md"),
            Some("self_model")
        );
        assert_eq!(
            is_core_memory_path("core_memory/user_summary"),
            Some("user_summary")
        );
        assert_eq!(
            is_core_memory_path("core-memory/self_model.md"),
            Some("self_model")
        );
        assert_eq!(
            is_core_memory_path("core_memory/current_priorities.md"),
            Some("current_priorities")
        );
        assert_eq!(
            is_core_memory_path("core_memory/key_people"),
            Some("key_people")
        );
        assert_eq!(
            is_core_memory_path("core_memory/workflows.md"),
            Some("workflows")
        );
    }

    #[test]
    fn test_is_core_memory_path_bare_names() {
        assert_eq!(is_core_memory_path("self_model"), Some("self_model"));
        assert_eq!(is_core_memory_path("self_model.md"), Some("self_model"));
        assert_eq!(is_core_memory_path("user_summary"), Some("user_summary"));
        assert_eq!(is_core_memory_path("workflows.md"), Some("workflows"));
    }

    #[test]
    fn test_is_core_memory_path_tilde_prefix() {
        assert_eq!(
            is_core_memory_path("~/core_memory/workflows.md"),
            Some("workflows")
        );
        assert_eq!(is_core_memory_path("~/self_model.md"), Some("self_model"));
    }

    #[test]
    fn test_is_core_memory_path_dot_prefix() {
        assert_eq!(
            is_core_memory_path("./core_memory/self_model.md"),
            Some("self_model")
        );
    }

    #[test]
    fn test_is_core_memory_path_directory() {
        assert_eq!(is_core_memory_path("core_memory"), Some("core_memory"));
        assert_eq!(is_core_memory_path("core-memory"), Some("core_memory"));
    }

    #[test]
    fn test_is_core_memory_path_non_matching() {
        // Substring of "core_memory" in a different path should not match
        assert_eq!(is_core_memory_path("notes/core_memory_backup.md"), None);
        // File containing section name as substring should not match
        assert_eq!(is_core_memory_path("my_self_model.md"), None);
        // Normal files should not match
        assert_eq!(is_core_memory_path("notes.md"), None);
        assert_eq!(is_core_memory_path("sub/notes.md"), None);
        assert_eq!(is_core_memory_path("config.toml"), None);
        // Non-existent section under core_memory/ prefix
        assert_eq!(is_core_memory_path("core_memory/nonexistent.md"), None);
    }

    #[tokio::test]
    async fn test_read_core_memory_section_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "core_memory/self_model.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(
            output.content.contains("not filesystem-accessible"),
            "expected domain-specific error, got: {}",
            output.content
        );
        assert!(
            output
                .content
                .contains("auto-injected into your system prompt"),
            "expected system prompt guidance, got: {}",
            output.content
        );
        assert!(
            output.content.contains("update_core_memory"),
            "expected update_core_memory redirect, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_read_core_memory_bare_name_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "self_model" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("not filesystem-accessible"));
    }

    #[tokio::test]
    async fn test_read_core_memory_non_matching_path_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join("notes")).unwrap();
        fs::write(home.join("notes/core_memory_backup.md"), "backup data").unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "notes/core_memory_backup.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "should NOT reject: {}", output.content);
        assert!(output.content.contains("backup data"));
    }

    #[tokio::test]
    async fn test_read_double_dot_in_filename_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("file..v2.md"), "version 2 content").unwrap();

        let tool = ReadAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "file..v2.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("version 2 content"));
    }
}
