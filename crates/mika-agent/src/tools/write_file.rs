use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::path::Component;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

/// Maximum file size for reading existing content during confirmation flow (100 KB).
const MAX_EXISTING_SIZE: u64 = 100 * 1024;

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Write content to a file in the agent's home directory. If the file already exists, the current content is returned and you MUST call again with confirm: true to overwrite.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path within the agent's home directory (e.g., 'notes/todo.md')"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    },
                    "confirm": {
                        "type": "boolean",
                        "description": "Set to true to confirm overwriting an existing file. Required when the file already exists."
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let path = input["path"].as_str().unwrap_or("");
        if path.is_empty() {
            return Ok(ToolOutput::error("'path' is required and cannot be empty."));
        }
        if path.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "Path exceeds maximum length of {MAX_INPUT_LEN} characters."
            )));
        }

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

        // Reject absolute paths
        if std::path::Path::new(path).is_absolute() {
            return Ok(ToolOutput::error(
                "Absolute paths are not allowed. Use a relative path within the home directory.",
            ));
        }

        // Prevent path traversal using component inspection
        for component in std::path::Path::new(path).components() {
            match component {
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Ok(ToolOutput::error(
                        "Path traversal components ('..', root, or prefix) are not allowed.",
                    ));
                }
                _ => {}
            }
        }

        let full_path = ctx.home_dir.join(path);

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolOutput::error(format!(
                    "Failed to create parent directories: {e}"
                )));
            }

            // Check for symlinks in the parent chain
            match tokio::fs::symlink_metadata(parent).await {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        return Ok(ToolOutput::error(
                            "Symbolic links are not allowed in the path.",
                        ));
                    }
                }
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to verify parent directory: {e}"
                    )));
                }
            }

            // Verify containment using canonicalize (parent now exists)
            let canonical_parent = match parent.canonicalize() {
                Ok(c) => c,
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to resolve parent directory: {e}"
                    )));
                }
            };
            let home_canonical = match ctx.home_dir.canonicalize() {
                Ok(c) => c,
                Err(_) => {
                    return Ok(ToolOutput::error("Home directory does not exist."));
                }
            };
            if !canonical_parent.starts_with(&home_canonical) {
                return Ok(ToolOutput::error(
                    "Path resolves outside the home directory.",
                ));
            }
        }

        // Check if file already exists
        let file_exists = tokio::fs::try_exists(&full_path).await.unwrap_or(false);

        if file_exists && !confirm {
            // Read existing content and return it for the agent to review
            match tokio::fs::metadata(&full_path).await {
                Ok(meta) => {
                    let size = meta.len();
                    if size > MAX_EXISTING_SIZE {
                        return Ok(ToolOutput::error(format!(
                            "File already exists at '{path}' ({size} bytes, too large to preview). \
                             Call again with confirm: true to overwrite."
                        )));
                    }

                    match tokio::fs::read_to_string(&full_path).await {
                        Ok(existing) => {
                            return Ok(ToolOutput::error(format!(
                                "File already exists at '{path}' ({size} bytes). \
                                 Current content is shown below. \
                                 Call again with confirm: true to overwrite.\n\n{existing}"
                            )));
                        }
                        Err(e) => {
                            return Ok(ToolOutput::error(format!(
                                "File already exists at '{path}' but could not be read: {e}. \
                                 Call again with confirm: true to overwrite."
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
                "Wrote {bytes_written} bytes to '{path}'."
            ))),
            Err(e) => Ok(ToolOutput::error(format!("Failed to write '{path}': {e}"))),
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

        let tool = WriteFileTool;
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

        let tool = WriteFileTool;
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

        let tool = WriteFileTool;
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

        let tool = WriteFileTool;
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

        let tool = WriteFileTool;
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

        let tool = WriteFileTool;
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

        let tool = WriteFileTool;
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

        let tool = WriteFileTool;
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

        let tool = WriteFileTool;
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

        let tool = WriteFileTool;
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
    async fn test_double_dot_in_filename_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = WriteFileTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "file..v2.md", "content": "version 2" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);

        let written = fs::read_to_string(home.join("file..v2.md")).unwrap();
        assert_eq!(written, "version 2");
    }
}
