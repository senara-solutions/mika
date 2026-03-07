use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::fmt::Write;

use super::{Tool, ToolContext, ToolOutput, validate_and_resolve_path};

/// Maximum number of entries returned by list_files.
const MAX_ENTRIES: usize = 500;
/// Maximum recursion depth when listing directories.
const MAX_DEPTH: usize = 10;

pub struct ListFilesTool;

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str {
        "list_files"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_files".to_string(),
            description: "List files and directories in the agent's home directory. Omit path or pass an empty string to list the home directory root. Returns up to 500 entries. Symlinks are skipped.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to a subdirectory within the agent's home directory. Omit or leave empty to list the home directory root."
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let path = input["path"].as_str().unwrap_or("").trim();

        let list_dir = if path.is_empty() {
            ctx.home_dir.to_path_buf()
        } else {
            match validate_and_resolve_path(path, ctx.home_dir).await {
                Ok(p) => p,
                Err(e) => return Ok(e),
            }
        };

        if !list_dir.exists() {
            return Ok(ToolOutput::error(format!(
                "Directory not found: '{}'",
                list_dir.display()
            )));
        }

        if !list_dir.is_dir() {
            return Ok(ToolOutput::error(format!(
                "'{}' is not a directory.",
                list_dir.display()
            )));
        }

        let mut entries: Vec<(String, bool)> = Vec::new();
        collect_entries(&list_dir, &list_dir, &mut entries, 0);

        if entries.is_empty() {
            return Ok(ToolOutput::success(format!(
                "Directory '{}' is empty.",
                list_dir.display()
            )));
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut output = String::new();
        writeln!(output, "Contents of '{}':", list_dir.display()).unwrap();
        for (name, is_dir) in &entries {
            let kind = if *is_dir { "dir" } else { "file" };
            writeln!(output, "  {name} ({kind})").unwrap();
        }
        if entries.len() >= MAX_ENTRIES {
            writeln!(output, "  [truncated at {MAX_ENTRIES} entries]").unwrap();
        }

        Ok(ToolOutput::success(output))
    }
}

/// Recursively collect entries with their relative paths.
/// Skips symlinks; respects depth and count limits.
fn collect_entries(
    base: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<(String, bool)>,
    depth: usize,
) {
    if depth > MAX_DEPTH || out.len() >= MAX_ENTRIES {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if out.len() >= MAX_ENTRIES {
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
        let relative = path
            .strip_prefix(base)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string_lossy().to_string());

        if ft.is_dir() {
            out.push((format!("{relative}/"), true));
            collect_entries(base, &path, out, depth + 1);
        } else if ft.is_file() {
            out.push((relative, false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::fs;

    #[tokio::test]
    async fn test_list_home_root() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("notes.md"), "hello").unwrap();
        fs::write(home.join("todo.md"), "world").unwrap();

        let tool = ListFilesTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({});

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("notes.md"));
        assert!(output.content.contains("todo.md"));
    }

    #[tokio::test]
    async fn test_list_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ListFilesTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({});

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("empty"));
    }

    #[tokio::test]
    async fn test_list_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join("notes")).unwrap();
        fs::write(home.join("notes").join("work.md"), "work notes").unwrap();

        let tool = ListFilesTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "notes" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("work.md"));
    }

    #[tokio::test]
    async fn test_list_shows_dirs_and_files() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join("subdir")).unwrap();
        fs::write(home.join("file.txt"), "data").unwrap();

        let tool = ListFilesTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({});

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("(dir)"));
        assert!(output.content.contains("(file)"));
    }

    #[tokio::test]
    async fn test_list_skips_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("real.md"), "real").unwrap();

        let outside = tmp.path().join("secret.txt");
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, home.join("link.md")).unwrap();

        let tool = ListFilesTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({});

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("real.md"));
        assert!(!output.content.contains("link.md"));
    }

    #[tokio::test]
    async fn test_list_path_traversal_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ListFilesTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "../../etc" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_list_absolute_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let tool = ListFilesTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({ "path": "/etc" });

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("Absolute paths are not allowed"));
    }

    #[tokio::test]
    async fn test_list_nested_files() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join("a").join("b")).unwrap();
        fs::write(home.join("a").join("b").join("deep.md"), "deep").unwrap();
        fs::write(home.join("top.md"), "top").unwrap();

        let tool = ListFilesTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({});

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("top.md"));
        assert!(output.content.contains("deep.md"));
    }

    #[test]
    fn test_collect_entries_depth_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");

        // Create a directory tree deeper than MAX_DEPTH
        let mut deep_path = home.clone();
        for i in 0..=MAX_DEPTH + 2 {
            deep_path = deep_path.join(format!("level{i}"));
        }
        fs::create_dir_all(&deep_path).unwrap();
        fs::write(deep_path.join("deep_file.md"), "deep").unwrap();
        fs::write(home.join("shallow.md"), "shallow").unwrap();

        let mut entries = Vec::new();
        collect_entries(&home, &home, &mut entries, 0);

        assert!(entries.iter().any(|(p, _)| p == "shallow.md"));
        assert!(!entries.iter().any(|(p, _)| p.contains("deep_file.md")));
    }

    #[test]
    fn test_collect_entries_count_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        for i in 0..MAX_ENTRIES + 10 {
            fs::write(home.join(format!("file_{i:04}.txt")), "data").unwrap();
        }

        let mut entries = Vec::new();
        collect_entries(&home, &home, &mut entries, 0);

        assert_eq!(entries.len(), MAX_ENTRIES);
    }
}
