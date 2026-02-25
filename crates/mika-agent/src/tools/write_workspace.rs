use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::path::{Component, PathBuf};

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

        // Reject absolute paths
        if std::path::Path::new(path).is_absolute() {
            return Ok(ToolOutput::error(
                "Absolute paths are not allowed. Use a relative path within the workspace.",
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

        let full_path = self.workspace_dir.join(path);

        // Ensure workspace dir exists
        if let Err(e) = tokio::fs::create_dir_all(&self.workspace_dir).await {
            return Ok(ToolOutput::error(format!(
                "Failed to create workspace directory: {e}"
            )));
        }

        // Create parent directories if needed, then verify containment via canonicalize
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
                            "Symbolic links are not allowed in the workspace.",
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
            let workspace_canonical = match self.workspace_dir.canonicalize() {
                Ok(c) => c,
                Err(_) => {
                    return Ok(ToolOutput::error("Workspace directory does not exist."));
                }
            };
            if !canonical_parent.starts_with(&workspace_canonical) {
                return Ok(ToolOutput::error(
                    "Path resolves outside the workspace directory.",
                ));
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
}
