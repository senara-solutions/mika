use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::path::PathBuf;

use super::{Tool, ToolContext, ToolOutput};

/// Maximum file size that can be read from the workspace (100 KB).
const MAX_READ_SIZE: u64 = 100 * 1024;

pub struct ReadWorkspaceTool {
    pub workspace_dir: PathBuf,
}

#[async_trait]
impl Tool for ReadWorkspaceTool {
    fn name(&self) -> &str {
        "read_workspace"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_workspace".to_string(),
            description: "Read a file from the team workspace. Use this to access shared context and outputs from other team members.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to the file in the workspace (e.g., 'research.md')"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let path = input["path"].as_str().unwrap_or("");
        if path.is_empty() {
            return Ok(ToolOutput::error("'path' is required and cannot be empty."));
        }

        // Prevent path traversal
        if path.contains("..") {
            return Ok(ToolOutput::error("Path traversal ('..') is not allowed."));
        }

        let full_path = self.workspace_dir.join(path);

        // Verify the resolved path is still within the workspace
        match full_path.canonicalize() {
            Ok(canonical) => {
                let workspace_canonical = match self.workspace_dir.canonicalize() {
                    Ok(c) => c,
                    Err(_) => {
                        return Ok(ToolOutput::error("Workspace directory does not exist."));
                    }
                };
                if !canonical.starts_with(&workspace_canonical) {
                    return Ok(ToolOutput::error(
                        "Path resolves outside the workspace directory.",
                    ));
                }
            }
            Err(_) => {
                return Ok(ToolOutput::error(format!("File not found: {path}")));
            }
        }

        // Check file size
        match tokio::fs::metadata(&full_path).await {
            Ok(meta) => {
                if meta.len() > MAX_READ_SIZE {
                    return Ok(ToolOutput::error(format!(
                        "File too large ({} bytes). Maximum is {} bytes.",
                        meta.len(),
                        MAX_READ_SIZE
                    )));
                }
            }
            Err(_) => {
                return Ok(ToolOutput::error(format!("File not found: {path}")));
            }
        }

        match tokio::fs::read_to_string(&full_path).await {
            Ok(content) => Ok(ToolOutput::success(content)),
            Err(e) => Ok(ToolOutput::error(format!("Failed to read '{path}': {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::fs;

    #[tokio::test]
    async fn test_read_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("test.md"), "hello world").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "test.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content, "hello world");
    }

    #[tokio::test]
    async fn test_read_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "missing.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_read_path_traversal_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "../../../etc/passwd" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_read_empty_path() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("required"));
    }

    #[tokio::test]
    async fn test_read_nested_file() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join("subdir")).unwrap();
        fs::write(workspace.join("subdir").join("notes.md"), "nested content").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "subdir/notes.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content, "nested content");
    }
}
