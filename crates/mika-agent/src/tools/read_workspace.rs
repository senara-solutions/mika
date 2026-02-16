use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::path::{Component, PathBuf};

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

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

        // Validate path length
        if path.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "Path exceeds maximum length of {MAX_INPUT_LEN} characters."
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

        // Reject symlinks (check before canonicalize to prevent TOCTOU via symlink)
        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Ok(ToolOutput::error(
                        "Symbolic links are not allowed in the workspace.",
                    ));
                }
            }
            Err(_) => {
                return Ok(ToolOutput::error(format!("File not found: {path}")));
            }
        }

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

    #[tokio::test]
    async fn test_read_absolute_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "/etc/passwd" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Absolute paths are not allowed"));
    }

    #[tokio::test]
    async fn test_read_symlink_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        // Create a real file outside workspace and symlink to it
        let secret = tmp.path().join("secret.txt");
        fs::write(&secret, "top secret").unwrap();
        std::os::unix::fs::symlink(&secret, workspace.join("link.md")).unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "link.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Symbolic links are not allowed"));
    }

    #[tokio::test]
    async fn test_read_path_length_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let long_path = "a".repeat(MAX_INPUT_LEN + 1);
        let input = serde_json::json!({ "path": long_path });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("exceeds maximum length"));
    }

    #[tokio::test]
    async fn test_read_double_dot_in_filename_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("file..v2.md"), "version 2 content").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "file..v2.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content, "version 2 content");
    }
}
