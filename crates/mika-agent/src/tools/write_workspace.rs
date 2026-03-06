use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::path::PathBuf;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput, validate_and_resolve_path};

pub struct WriteWorkspaceTool {
    pub workspace_dir: PathBuf,
}

#[async_trait]
impl Tool for WriteWorkspaceTool {
    fn name(&self) -> &str {
        "write_workspace"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_workspace".to_string(),
            description: "Write a file to the team workspace. Use this to share results, research, and deliverables with other team members.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path for the file in the workspace (e.g., 'research.md')"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let path = input["path"].as_str().unwrap_or("");

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

        let full_path = match validate_and_resolve_path(path, &self.workspace_dir, true).await {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

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
        let workspace = tmp.path().join("workspace");

        let tool = WriteWorkspaceTool {
            workspace_dir: workspace.clone(),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "output.md", "content": "hello" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("5 bytes"));

        let written = fs::read_to_string(workspace.join("output.md")).unwrap();
        assert_eq!(written, "hello");
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");

        let tool = WriteWorkspaceTool {
            workspace_dir: workspace.clone(),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "sub/dir/file.md", "content": "nested" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);

        let written = fs::read_to_string(workspace.join("sub/dir/file.md")).unwrap();
        assert_eq!(written, "nested");
    }

    #[tokio::test]
    async fn test_write_path_traversal_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");

        let tool = WriteWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "../escape.txt", "content": "evil" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_write_empty_path() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");

        let tool = WriteWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "", "content": "data" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("required"));
    }

    #[tokio::test]
    async fn test_write_empty_content() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");

        let tool = WriteWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "file.md", "content": "" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("required"));
    }

    #[tokio::test]
    async fn test_write_overwrites_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("file.md"), "old content").unwrap();

        let tool = WriteWorkspaceTool {
            workspace_dir: workspace.clone(),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "file.md", "content": "new content" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);

        let written = fs::read_to_string(workspace.join("file.md")).unwrap();
        assert_eq!(written, "new content");
    }

    #[tokio::test]
    async fn test_write_absolute_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");

        let tool = WriteWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "/tmp/evil.txt", "content": "data" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Absolute paths are not allowed"));
    }

    #[tokio::test]
    async fn test_write_symlink_parent_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        // Create a symlink directory inside workspace pointing outside
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("linked_dir")).unwrap();

        let tool = WriteWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "linked_dir/file.md", "content": "data" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        // Should be caught by either symlink check or canonicalize containment check
        assert!(
            output.content.contains("Symbolic links")
                || output.content.contains("outside the workspace")
        );
    }

    #[tokio::test]
    async fn test_write_double_dot_in_filename_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");

        let tool = WriteWorkspaceTool {
            workspace_dir: workspace.clone(),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "file..v2.md", "content": "version 2" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("bytes"));

        let written = fs::read_to_string(workspace.join("file..v2.md")).unwrap();
        assert_eq!(written, "version 2");
    }

    #[tokio::test]
    async fn test_write_success_message_contains_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");

        let tool = WriteWorkspaceTool {
            workspace_dir: workspace.clone(),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "sub/file.md", "content": "data" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);

        let expected_path = workspace.join("sub/file.md");
        assert!(
            output
                .content
                .contains(&expected_path.display().to_string()),
            "expected absolute path '{}' in message: {}",
            expected_path.display(),
            output.content
        );
    }
}
