use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::fmt::Write;
use std::path::PathBuf;

use super::{Tool, ToolContext, ToolOutput};

pub struct ListWorkspaceTool {
    pub workspace_dir: PathBuf,
    /// Optional read-only workspace from a previous run (via `--run-id`).
    pub reference_dir: Option<PathBuf>,
}

#[async_trait]
impl Tool for ListWorkspaceTool {
    fn name(&self) -> &str {
        "list_workspace"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_workspace".to_string(),
            description: "List all files in the team workspace with their sizes. Use this to see what shared artifacts are available.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let mut current_files = Vec::new();
        if self.workspace_dir.exists() {
            collect_files(
                &self.workspace_dir,
                &self.workspace_dir,
                &mut current_files,
                0,
            );
        }

        let mut ref_files = Vec::new();
        if let Some(ref ref_dir) = self.reference_dir
            && ref_dir.exists()
        {
            collect_files(ref_dir, ref_dir, &mut ref_files, 0);
        }

        if current_files.is_empty() && ref_files.is_empty() {
            return Ok(ToolOutput::success("Workspace is empty (no files yet)."));
        }

        current_files.sort_by(|a, b| a.0.cmp(&b.0));
        ref_files.sort_by(|a, b| a.0.cmp(&b.0));

        let mut output = String::new();

        if !current_files.is_empty() {
            let label = if self.reference_dir.is_some() {
                "Current run files:"
            } else {
                "Workspace files:"
            };
            writeln!(output, "{label}").unwrap();
            for (path, size) in &current_files {
                writeln!(output, "  {path} ({size})").unwrap();
            }
        }

        if !ref_files.is_empty() {
            if !current_files.is_empty() {
                writeln!(output).unwrap();
            }
            writeln!(output, "Previous run files (read-only):").unwrap();
            for (path, size) in &ref_files {
                writeln!(output, "  {path} ({size})").unwrap();
            }
        }

        Ok(ToolOutput::success(output))
    }
}

/// Maximum recursion depth for workspace listing.
const MAX_DEPTH: usize = 10;
/// Maximum number of files to collect.
const MAX_FILES: usize = 500;

/// Recursively collect files with their relative paths and human-readable sizes.
/// Skips symlinks, respects depth and file count limits.
fn collect_files(
    base: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<(String, String)>,
    depth: usize,
) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        // Skip symlinks entirely
        if ft.is_symlink() {
            continue;
        }

        let path = entry.path();
        if ft.is_dir() {
            collect_files(base, &path, out, depth + 1);
        } else if ft.is_file() {
            let relative = path
                .strip_prefix(base)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string_lossy().to_string());
            let size = entry
                .metadata()
                .map(|m| format_size(m.len()))
                .unwrap_or_else(|_| "?".to_string());
            out.push((relative, size));
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::fs;

    #[tokio::test]
    async fn test_list_empty_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");

        let tool = ListWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: None,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let output = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("empty"));
    }

    #[tokio::test]
    async fn test_list_with_files() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("research.md"), "some research content").unwrap();
        fs::write(workspace.join("summary.md"), "summary").unwrap();

        let tool = ListWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: None,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let output = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("research.md"));
        assert!(output.content.contains("summary.md"));
    }

    #[tokio::test]
    async fn test_list_with_nested_files() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join("sub")).unwrap();
        fs::write(workspace.join("top.md"), "top").unwrap();
        fs::write(workspace.join("sub").join("nested.md"), "nested").unwrap();

        let tool = ListWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: None,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let output = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("top.md"));
        assert!(output.content.contains("sub/nested.md"));
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
    }

    #[tokio::test]
    async fn test_list_skips_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("real.md"), "real file").unwrap();

        // Create a symlink to a file outside workspace
        let outside = tmp.path().join("secret.txt");
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("link.md")).unwrap();

        let tool = ListWorkspaceTool {
            workspace_dir: workspace,
            reference_dir: None,
        };
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let output = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("real.md"));
        assert!(!output.content.contains("link.md"));
    }

    #[test]
    fn test_collect_files_depth_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");

        // Create a directory tree deeper than MAX_DEPTH
        let mut deep_path = workspace.clone();
        for i in 0..=MAX_DEPTH + 2 {
            deep_path = deep_path.join(format!("level{i}"));
        }
        fs::create_dir_all(&deep_path).unwrap();
        fs::write(deep_path.join("deep_file.md"), "deep").unwrap();

        // Also create a file at a shallow level
        fs::write(workspace.join("shallow.md"), "shallow").unwrap();

        let mut files = Vec::new();
        collect_files(&workspace, &workspace, &mut files, 0);

        // Shallow file should be found
        assert!(files.iter().any(|(p, _)| p == "shallow.md"));
        // Deep file beyond MAX_DEPTH should NOT be found
        assert!(!files.iter().any(|(p, _)| p.contains("deep_file.md")));
    }

    #[test]
    fn test_collect_files_count_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        // Create more files than MAX_FILES
        for i in 0..MAX_FILES + 10 {
            fs::write(workspace.join(format!("file_{i:04}.txt")), "data").unwrap();
        }

        let mut files = Vec::new();
        collect_files(&workspace, &workspace, &mut files, 0);

        assert_eq!(files.len(), MAX_FILES);
    }
}
