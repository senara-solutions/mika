use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{
    MAX_INPUT_LEN, Tool, ToolContext, ToolOutput, resolve_agent_home, validate_and_resolve_path,
};

/// Maximum file size for reading existing content during confirmation flow (100 KB).
const MAX_EXISTING_SIZE: u64 = 100 * 1024;

pub struct WriteAgentFileTool;

#[async_trait]
impl Tool for WriteAgentFileTool {
    fn name(&self) -> &str {
        "write_agent_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_agent_file".to_string(),
            description: "Write content to a file in the agent's home directory. If the file already exists, the current content is returned and you MUST call again with confirm: true to overwrite.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path within the agent's home directory (e.g., 'notes/todo.md' or '~/notes/todo.md')"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    },
                    "confirm": {
                        "type": "boolean",
                        "description": "Set to true to confirm overwriting an existing file. Required when the file already exists."
                    },
                    "agent": {
                        "type": "string",
                        "description": "Target agent name (orchestrator-only). Omit to use your own home directory."
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let path = input["path"].as_str().unwrap_or("");
        let agent_param = input["agent"].as_str();

        let content = input["content"].as_str().unwrap_or("");
        if content.is_empty() {
            return Ok(ToolOutput::error(
                "'content' is required and cannot be empty.",
            ));
        }
        if content.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "Content exceeds maximum length of {MAX_INPUT_LEN} characters."
            )));
        }

        let confirm = input["confirm"].as_bool().unwrap_or(false);

        // Resolve base directory (own home or target agent's home)
        let base_dir = match resolve_agent_home(agent_param, ctx).await {
            Ok(dir) => dir,
            Err(e) => return Ok(e),
        };

        let full_path = match validate_and_resolve_path(path, &base_dir, true).await {
            Ok(p) => p,
            Err(e) => {
                // If the path was absolute, hint about agent parameter or run_shell
                if Path::new(path).is_absolute() {
                    return Ok(ToolOutput::error(format!(
                        "{} For other agents' files, use the `agent` parameter. \
                         For paths outside any agent's home directory, use run_shell.",
                        e.content
                    )));
                }
                return Ok(e);
            }
        };

        // Check if file already exists
        let file_exists = tokio::fs::try_exists(&full_path).await.unwrap_or(false);

        // Reject symlinks at the target file itself (defense-in-depth beyond parent check)
        if file_exists {
            match tokio::fs::symlink_metadata(&full_path).await {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Ok(ToolOutput::error(
                        "Target file is a symbolic link. Symbolic links are not allowed.",
                    ));
                }
                _ => {}
            }
        }

        if file_exists && !confirm {
            // Build the retry hint including agent param if cross-agent
            let agent_hint = match agent_param {
                Some(name) if !name.is_empty() => format!(" and agent: \"{}\"", name.trim()),
                _ => String::new(),
            };

            // Read existing content and return it for the agent to review
            match tokio::fs::metadata(&full_path).await {
                Ok(meta) => {
                    let size = meta.len();
                    if size > MAX_EXISTING_SIZE {
                        return Ok(ToolOutput::error(format!(
                            "File already exists at '{}' ({size} bytes, too large to preview). \
                             Call again with confirm: true{agent_hint} to overwrite.",
                            full_path.display()
                        )));
                    }

                    match tokio::fs::read_to_string(&full_path).await {
                        Ok(existing) => {
                            return Ok(ToolOutput::error(format!(
                                "File already exists at '{}' ({size} bytes). \
                                 Current content is shown below. \
                                 Call again with confirm: true{agent_hint} to overwrite.\n\n{existing}",
                                full_path.display()
                            )));
                        }
                        Err(e) => {
                            return Ok(ToolOutput::error(format!(
                                "File already exists at '{}' but could not be read: {e}. \
                                 Call again with confirm: true{agent_hint} to overwrite.",
                                full_path.display()
                            )));
                        }
                    }
                }
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "File appears to exist but metadata could not be read: {e}"
                    )));
                }
            }
        }

        // Write the file
        let bytes_written = content.len();
        match tokio::fs::write(&full_path, content).await {
            Ok(()) => Ok(ToolOutput::success(format!(
                "Wrote {bytes_written} bytes to '{}'.",
                full_path.display()
            ))),
            Err(e) => Ok(ToolOutput::error(format!(
                "Failed to write '{}': {e}",
                full_path.display()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::fs;

    #[tokio::test]
    async fn test_write_new_file() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "notes.md", "content": "hello world" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("11 bytes"));

        let written = fs::read_to_string(home.join("notes.md")).unwrap();
        assert_eq!(written, "hello world");
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "sub/dir/file.md", "content": "nested" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);

        let written = fs::read_to_string(home.join("sub/dir/file.md")).unwrap();
        assert_eq!(written, "nested");
    }

    #[tokio::test]
    async fn test_overwrite_without_confirm_returns_content() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("existing.md"), "old content").unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "existing.md", "content": "new content" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("File already exists"));
        assert!(output.content.contains("old content"));
        assert!(output.content.contains("confirm: true"));

        // Verify the file was NOT overwritten
        let still_old = fs::read_to_string(home.join("existing.md")).unwrap();
        assert_eq!(still_old, "old content");
    }

    #[tokio::test]
    async fn test_overwrite_with_confirm_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("existing.md"), "old content").unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({
            "path": "existing.md",
            "content": "new content",
            "confirm": true
        });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("11 bytes"));

        let written = fs::read_to_string(home.join("existing.md")).unwrap();
        assert_eq!(written, "new content");
    }

    #[tokio::test]
    async fn test_path_traversal_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "../escape.txt", "content": "evil" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_absolute_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "/tmp/evil.txt", "content": "data" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Absolute paths are not allowed"));
    }

    #[tokio::test]
    async fn test_empty_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "", "content": "data" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("required"));
    }

    #[tokio::test]
    async fn test_empty_content() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "file.md", "content": "" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("required"));
    }

    #[tokio::test]
    async fn test_symlink_parent_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        // Create a symlink directory inside home pointing outside
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("linked_dir")).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "linked_dir/file.md", "content": "data" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        // Caught by either symlink check or canonicalize containment check
        assert!(
            output.content.contains("Symbolic links")
                || output.content.contains("outside the home")
        );
    }

    #[tokio::test]
    async fn test_confirm_false_explicit_still_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("file.md"), "original").unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({
            "path": "file.md",
            "content": "replacement",
            "confirm": false
        });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("File already exists"));
    }

    #[tokio::test]
    async fn test_symlink_target_file_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        // Create a real file outside home and symlink to it from within home
        let outside = tmp.path().join("secret.txt");
        fs::write(&outside, "secret data").unwrap();
        std::os::unix::fs::symlink(&outside, home.join("link.md")).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);

        // Even with confirm: true, symlink target should be rejected
        let input = serde_json::json!({
            "path": "link.md",
            "content": "overwrite attempt",
            "confirm": true
        });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("symbolic link"));
    }

    #[tokio::test]
    async fn test_success_message_contains_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "sub/file.md", "content": "data" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);

        // Must contain the absolute path, not just the relative input
        let expected_path = home.join("sub/file.md");
        assert!(
            output
                .content
                .contains(&expected_path.display().to_string()),
            "expected absolute path '{}' in message: {}",
            expected_path.display(),
            output.content
        );
    }

    #[tokio::test]
    async fn test_confirmation_message_contains_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("existing.md"), "old").unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "existing.md", "content": "new" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);

        // Confirmation message must show absolute path
        let expected_path = home.join("existing.md");
        assert!(
            output
                .content
                .contains(&expected_path.display().to_string()),
            "expected absolute path '{}' in confirmation: {}",
            expected_path.display(),
            output.content
        );
    }

    #[tokio::test]
    async fn test_absolute_path_hints_agent_param_and_run_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "/tmp/outside.txt", "content": "data" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(
            output.content.contains("agent") && output.content.contains("run_shell"),
            "expected agent param and run_shell hints, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_traversal_path_does_not_hint_run_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "../escape.txt", "content": "data" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(
            !output.content.contains("run_shell"),
            "traversal error should not mention run_shell, got: {}",
            output.content
        );
    }

    // --- Cross-agent file access tests ---

    fn setup_cross_agent_dirs(tmp: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
        let global = tmp.path().join("mika-home");
        let mika_home = global.join("agents").join("mika");
        let other_home = global.join("agents").join("work");
        fs::create_dir_all(&mika_home).unwrap();
        fs::create_dir_all(&other_home).unwrap();
        fs::write(mika_home.join("config.toml"), "[config]").unwrap();
        fs::write(other_home.join("config.toml"), "model = \"old\"").unwrap();
        (global, mika_home)
    }

    #[tokio::test]
    async fn test_cross_agent_write_confirmation_includes_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let (global, mika_home) = setup_cross_agent_dirs(&tmp);

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home_and_global(&mika_home, &global);
        let input = serde_json::json!({
            "path": "config.toml",
            "content": "model = \"new\"",
            "agent": "work"
        });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("File already exists"));
        assert!(
            output.content.contains("agent: \"work\""),
            "confirmation should include agent hint, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_cross_agent_write_with_confirm() {
        let tmp = tempfile::tempdir().unwrap();
        let (global, mika_home) = setup_cross_agent_dirs(&tmp);

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home_and_global(&mika_home, &global);
        let input = serde_json::json!({
            "path": "config.toml",
            "content": "model = \"new\"",
            "confirm": true,
            "agent": "work"
        });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);

        let other_home = global.join("agents").join("work");
        let written = fs::read_to_string(other_home.join("config.toml")).unwrap();
        assert_eq!(written, "model = \"new\"");
    }

    #[tokio::test]
    async fn test_cross_agent_write_blocked_non_orchestrator() {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().join("mika-home");
        let other_home = global.join("agents").join("work");
        let writer_home = global.join("agents").join("writer");
        fs::create_dir_all(&other_home).unwrap();
        fs::create_dir_all(&writer_home).unwrap();
        fs::write(other_home.join("config.toml"), "data").unwrap();
        fs::write(writer_home.join("config.toml"), "data").unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::with_agent("writer");
        let ctx = harness.ctx_with_home_and_global(&writer_home, &global);
        let input = serde_json::json!({
            "path": "config.toml",
            "content": "evil",
            "agent": "work"
        });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("orchestrator"));
    }

    #[tokio::test]
    async fn test_double_dot_in_filename_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteAgentFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "file..v2.md", "content": "version 2" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);

        let written = fs::read_to_string(home.join("file..v2.md")).unwrap();
        assert_eq!(written, "version 2");
    }
}
