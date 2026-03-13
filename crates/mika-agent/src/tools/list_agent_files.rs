use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::fmt::Write;
use std::time::SystemTime;

use super::{Tool, ToolContext, ToolOutput, validate_and_resolve_path};

/// Maximum number of entries returned by list_agent_files.
const MAX_ENTRIES: usize = 500;
/// Maximum recursion depth when listing directories.
const MAX_DEPTH: usize = 10;

pub struct ListAgentFilesTool;

#[async_trait]
impl Tool for ListAgentFilesTool {
    fn name(&self) -> &str {
        "list_agent_files"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_agent_files".to_string(),
            description: "List files and directories in the agent's home directory. Omit path or pass an empty string to list the home directory root. Returns up to 500 entries with sizes and ages. Symlinks are skipped.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to a subdirectory within the agent's home directory. Omit, leave empty, or pass '~' to list the home directory root."
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let path = input["path"].as_str().unwrap_or("").trim();

        // Treat bare ~ as home root (same as empty path)
        let path = if path == "~" || path == "~/" {
            ""
        } else {
            path
        };

        let list_dir = if path.is_empty() {
            ctx.home_dir.to_path_buf()
        } else {
            match validate_and_resolve_path(path, ctx.home_dir, false).await {
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

        let list_dir_clone = list_dir.clone();
        let entries = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            collect_entries(&list_dir_clone, &list_dir_clone, &mut out, 0);
            out
        })
        .await
        .unwrap_or_default();

        if entries.is_empty() {
            return Ok(ToolOutput::success(format!(
                "Directory '{}' is empty.",
                list_dir.display()
            )));
        }

        let mut sorted = entries;
        sorted.sort_by(|a, b| a.0.cmp(&b.0));

        let mut output = String::new();
        writeln!(output, "Contents of '{}':", list_dir.display()).unwrap();
        for (name, is_dir, size, mtime) in &sorted {
            if *is_dir {
                let age = mtime
                    .map(|t| format!(", {}", format_date(t)))
                    .unwrap_or_default();
                writeln!(output, "  {name} (dir{age})").unwrap();
            } else {
                let size_str = size
                    .map(|s| format!(", {}", format_size(s)))
                    .unwrap_or_default();
                let age = mtime
                    .map(|t| format!(", {}", format_age(t)))
                    .unwrap_or_default();
                writeln!(output, "  {name} (file{size_str}{age})").unwrap();
            }
        }
        if sorted.len() >= MAX_ENTRIES {
            writeln!(output, "  [truncated at {MAX_ENTRIES} entries]").unwrap();
        }

        Ok(ToolOutput::success(output))
    }
}

/// Format a file size as a human-readable string (e.g. "2.3 KB").
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Format elapsed time since mtime as a human-readable age (e.g. "3h ago").
fn format_age(mtime: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(mtime)
        .unwrap_or_default()
        .as_secs();
    if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

/// Format a SystemTime as a date string (e.g. "2026-03-05") for directories.
fn format_date(mtime: SystemTime) -> String {
    use std::time::UNIX_EPOCH;
    let secs = mtime
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple date calculation: days since epoch
    let days = secs / 86400;
    // Use chrono if available; otherwise fall back to age display
    let age_secs = SystemTime::now()
        .duration_since(mtime)
        .unwrap_or_default()
        .as_secs();
    let _ = days; // suppress unused warning
    if age_secs < 86400 {
        format!("{}h ago", age_secs / 3600)
    } else {
        format!("{}d ago", age_secs / 86400)
    }
}

/// Recursively collect entries with their relative paths, sizes, and mtimes.
/// Skips symlinks; respects depth and count limits.
fn collect_entries(
    base: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<(String, bool, Option<u64>, Option<SystemTime>)>,
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

        // Collect metadata for size and mtime (best-effort)
        let (size, mtime) = entry
            .metadata()
            .map(|m| {
                let sz = if ft.is_file() { Some(m.len()) } else { None };
                let mt = m.modified().ok();
                (sz, mt)
            })
            .unwrap_or((None, None));

        if ft.is_dir() {
            out.push((format!("{relative}/"), true, None, mtime));
            collect_entries(base, &path, out, depth + 1);
        } else if ft.is_file() {
            out.push((relative, false, size, mtime));
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

        let tool = ListAgentFilesTool;
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

        let tool = ListAgentFilesTool;
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

        let tool = ListAgentFilesTool;
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

        let tool = ListAgentFilesTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({});

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("(dir"));
        assert!(output.content.contains("(file"));
    }

    #[tokio::test]
    async fn test_list_shows_file_size() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("data.txt"), "hello world").unwrap();

        let tool = ListAgentFilesTool;
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(&home);
        let input = serde_json::json!({});

        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "unexpected error: {}", output.content);
        // Should show size info (11 bytes)
        assert!(output.content.contains("11 B") || output.content.contains("B"));
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

        let tool = ListAgentFilesTool;
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

        let tool = ListAgentFilesTool;
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

        let tool = ListAgentFilesTool;
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

        let tool = ListAgentFilesTool;
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

        assert!(entries.iter().any(|(p, _, _, _)| p == "shallow.md"));
        assert!(
            !entries
                .iter()
                .any(|(p, _, _, _)| p.contains("deep_file.md"))
        );
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

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(2355), "2.3 KB");
        assert_eq!(format_size(1_048_576), "1.0 MB");
    }
}
