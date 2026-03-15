use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::{Tool, ToolContext, ToolOutput, validate_and_resolve_path};

/// Maximum file size that can be read from the workspace (100 KB).
const MAX_READ_SIZE: u64 = 100 * 1024;

pub struct ReadWorkspaceTool {
    pub workspace_dir: PathBuf,
    /// Optional read-only workspace from a previous run (via `--run-id`).
    pub reference_dir: Option<PathBuf>,
}

impl ReadWorkspaceTool {
    /// Read a file from a specific workspace directory with full security checks.
    async fn read_from_dir(&self, path: &str, base_dir: &Path) -> Result<ToolOutput> {
        let full_path = match validate_and_resolve_path(path, base_dir, false).await {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        // Reject symlinks at the target file
        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Ok(ToolOutput::error(
                        "Symbolic links are not allowed in the workspace.",
                    ));
                }
            }
            Err(_) => {
                return Ok(ToolOutput::error(format!(
                    "File not found: {}",
                    full_path.display()
                )));
            }
        }

        // Verify the resolved path is still within the workspace
        match full_path.canonicalize() {
            Ok(canonical) => {
                let workspace_canonical = match base_dir.canonicalize() {
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
                return Ok(ToolOutput::error(format!(
                    "File not found: {}",
                    full_path.display()
                )));
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
                return Ok(ToolOutput::error(format!(
                    "File not found: {}",
                    full_path.display()
                )));
            }
        }

        match tokio::fs::read_to_string(&full_path).await {
            Ok(content) => Ok(ToolOutput::success(content)),
            Err(e) => Ok(ToolOutput::error(format!(
                "Failed to read '{}': {e}",
                full_path.display()
            ))),
        }
    }
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

        // Reject paths whose first component is a dotfile/dotdir (e.g. .meta/)
        if path
            .split('/')
            .next()
            .is_some_and(|first| first.starts_with('.') && !first.starts_with(".."))
        {
            return Ok(ToolOutput::error(
                "Cannot access hidden files in workspace.",
            ));
        }

        // Try current workspace first, then fall back to reference workspace
        match self.read_from_dir(path, &self.workspace_dir).await {
            Ok(output) if !output.is_error => Ok(output),
            Ok(not_found) => {
                // Only fall back to reference workspace for genuine "not found" errors.
                // Security errors (symlinks, traversal) should not be masked.
                let is_not_found = not_found.content.contains("not found")
                    || not_found.content.contains("does not exist");
                if is_not_found {
                    if let Some(ref ref_dir) = self.reference_dir {
                        match self.read_from_dir(path, ref_dir).await {
                            Ok(output) if !output.is_error => Ok(output),
                            _ => Ok(not_found), // Return original error from current workspace
                        }
                    } else {
                        Ok(not_found)
                    }
                } else {
                    Ok(not_found)
                }
            }
            Err(e) => Err(e),
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
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("test.md"), "hello world").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: None,
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
            reference_dir: None,
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
            reference_dir: None,
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
            reference_dir: None,
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
            reference_dir: None,
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
            reference_dir: None,
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
            reference_dir: None,
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
            reference_dir: None,
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
    async fn test_read_dotdir_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join(".meta")).unwrap();
        fs::write(workspace.join(".meta").join("goal.md"), "secret goal").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: None,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        // .meta/goal.md should be rejected
        let input = serde_json::json!({ "path": ".meta/goal.md" });
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("hidden"));

        // .meta alone should be rejected
        let input = serde_json::json!({ "path": ".meta" });
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("hidden"));

        // .hidden file should be rejected
        let input = serde_json::json!({ "path": ".hidden" });
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("hidden"));
    }

    #[tokio::test]
    async fn test_read_double_dot_in_filename_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("file..v2.md"), "version 2 content").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: None,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "file..v2.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content, "version 2 content");
    }

    #[tokio::test]
    async fn test_reference_fallback_on_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let reference = tmp.path().join("reference");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&reference).unwrap();
        fs::write(reference.join("notes.md"), "from reference").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: Some(reference),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "notes.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content, "from reference");
    }

    #[tokio::test]
    async fn test_reference_fallback_file_in_current_only() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let reference = tmp.path().join("reference");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&reference).unwrap();
        fs::write(workspace.join("current_only.md"), "from current").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: Some(reference),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "current_only.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content, "from current");
    }

    #[tokio::test]
    async fn test_reference_fallback_file_in_reference_only() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let reference = tmp.path().join("reference");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&reference).unwrap();
        fs::write(reference.join("ref_only.md"), "from reference").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: Some(reference),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "ref_only.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content, "from reference");
    }

    #[tokio::test]
    async fn test_reference_fallback_current_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let reference = tmp.path().join("reference");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&reference).unwrap();
        fs::write(workspace.join("shared.md"), "current version").unwrap();
        fs::write(reference.join("shared.md"), "reference version").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: Some(reference),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "shared.md" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content, "current version");
    }

    #[tokio::test]
    async fn test_reference_fallback_traversal_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let reference = tmp.path().join("reference");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&reference).unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: Some(reference),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "../../etc/passwd" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_no_reference_fallback_on_symlink_error() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let reference = tmp.path().join("reference");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&reference).unwrap();

        // Create a symlink in the current workspace
        let secret = tmp.path().join("secret.txt");
        fs::write(&secret, "top secret").unwrap();
        std::os::unix::fs::symlink(&secret, workspace.join("link.md")).unwrap();

        // Put a legitimate file in reference with the same name
        fs::write(reference.join("link.md"), "from reference").unwrap();

        let tool = ReadWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: Some(reference),
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({ "path": "link.md" });

        // Should return the symlink error, NOT fall back to reference
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Symbolic links are not allowed"));
    }
}
