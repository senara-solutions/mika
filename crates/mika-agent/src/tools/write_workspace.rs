use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::path::PathBuf;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

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
        if path.is_empty() {
            return Ok(ToolOutput::error("'path' is required and cannot be empty."));
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

        // Prevent path traversal
        if path.contains("..") {
            return Ok(ToolOutput::error("Path traversal ('..') is not allowed."));
        }

        let full_path = self.workspace_dir.join(path);

        // Ensure workspace dir exists
        if let Err(e) = tokio::fs::create_dir_all(&self.workspace_dir).await {
            return Ok(ToolOutput::error(format!(
                "Failed to create workspace directory: {e}"
            )));
        }

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            // Verify parent is within workspace (using string prefix check before dirs exist)
            let workspace_str = self.workspace_dir.to_string_lossy();
            let parent_str = parent.to_string_lossy();
            if !parent_str.starts_with(workspace_str.as_ref()) {
                return Ok(ToolOutput::error(
                    "Path resolves outside the workspace directory.",
                ));
            }

            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolOutput::error(format!(
                    "Failed to create parent directories: {e}"
                )));
            }
        }

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
}
