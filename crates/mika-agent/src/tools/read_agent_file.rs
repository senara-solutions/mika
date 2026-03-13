use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput, validate_and_resolve_path};

/// Maximum file size that can be read from the agent home directory (100 KB).
const MAX_READ_SIZE: u64 = 100 * 1024;

pub struct ReadAgentFileTool;

#[async_trait]
impl Tool for ReadAgentFileTool {
    fn name(&self) -> &str {
        "read_agent_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_agent_file".to_string(),
            description: "Read a file from the agent's home directory. Returns the file contents as text. Files larger than 100 KB are rejected.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path within the agent's home directory (e.g., 'notes/todo.md' or '~/notes/todo.md')"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let path = input["path"].as_str().unwrap_or("");

        let full_path = match validate_and_resolve_path(path, ctx.home_dir, false).await {
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
