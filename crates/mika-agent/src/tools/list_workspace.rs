use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::fmt::Write;
use std::path::PathBuf;

use super::{Tool, ToolContext, ToolOutput};

pub struct ListWorkspaceTool {
    pub workspace_dir: PathBuf,
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
        if !self.workspace_dir.exists() {
            return Ok(ToolOutput::success("Workspace is empty (no files yet)."));
        }

        let mut files = Vec::new();
        collect_files(&self.workspace_dir, &self.workspace_dir, &mut files);

        if files.is_empty() {
            return Ok(ToolOutput::success("Workspace is empty (no files yet)."));
        }

        files.sort_by(|a, b| a.0.cmp(&b.0));

        let mut output = String::new();
        writeln!(output, "Workspace files:").unwrap();
        for (path, size) in &files {
            writeln!(output, "  {path} ({size})").unwrap();
        }
        Ok(ToolOutput::success(output))
    }
}

/// Recursively collect files with their relative paths and human-readable sizes.
fn collect_files(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, out);
        } else if path.is_file() {
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
}
