//! Argument completer functions for slash commands.
//!
//! Each completer returns `(Vec<CompletionItem>, title)` where title is
//! the popup header (e.g., " Models ").

use std::path::Path;

use mika_agent::config_keys::SETTABLE_CONFIG_KEYS;
use mika_common::agent::list_agents;
use mika_common::team::list_teams;

use super::autocomplete::{CompletionContext, CompletionItem};

/// Filter items by prefix (case-insensitive).
fn filter_by_prefix(items: Vec<CompletionItem>, prefix: &str) -> Vec<CompletionItem> {
    if prefix.is_empty() {
        return items;
    }
    let lower = prefix.to_lowercase();
    items
        .into_iter()
        .filter(|item| item.value.to_lowercase().starts_with(&lower))
        .collect()
}

// === Static completers ===

/// `/model <tab>` — model aliases.
pub fn complete_model(
    arg_text: &str,
    _arg_index: usize,
    _ctx: &CompletionContext,
) -> (Vec<CompletionItem>, &'static str) {
    let items = vec![
        CompletionItem {
            value: "sonnet".to_string(),
            description: Some("Claude Sonnet 4.6".to_string()),
        },
        CompletionItem {
            value: "opus".to_string(),
            description: Some("Claude Opus 4.6".to_string()),
        },
        CompletionItem {
            value: "haiku".to_string(),
            description: Some("Claude Haiku 4.5".to_string()),
        },
    ];
    (filter_by_prefix(items, arg_text), " Models ")
}

/// `/think <tab>` — thinking levels (only for arg_index 0).
pub fn complete_think(
    arg_text: &str,
    arg_index: usize,
    _ctx: &CompletionContext,
) -> (Vec<CompletionItem>, &'static str) {
    if arg_index > 0 {
        return (vec![], " Think ");
    }
    let items = vec![
        CompletionItem {
            value: "off".to_string(),
            description: Some("Disable thinking".to_string()),
        },
        CompletionItem {
            value: "low".to_string(),
            description: Some("5K token budget".to_string()),
        },
        CompletionItem {
            value: "medium".to_string(),
            description: Some("10K token budget".to_string()),
        },
        CompletionItem {
            value: "high".to_string(),
            description: Some("50K token budget".to_string()),
        },
    ];
    (filter_by_prefix(items, arg_text), " Think ")
}

/// `/memory <tab>` — subcommands.
pub fn complete_memory(
    arg_text: &str,
    arg_index: usize,
    _ctx: &CompletionContext,
) -> (Vec<CompletionItem>, &'static str) {
    if arg_index > 0 {
        return (vec![], " Memory ");
    }
    let items = vec![CompletionItem {
        value: "search".to_string(),
        description: Some("Search memory for a query".to_string()),
    }];
    (filter_by_prefix(items, arg_text), " Memory ")
}

/// `/config <tab>` — subcommands and config keys.
pub fn complete_config(
    arg_text: &str,
    arg_index: usize,
    _ctx: &CompletionContext,
) -> (Vec<CompletionItem>, &'static str) {
    match arg_index {
        0 => {
            // Subcommands
            let items = vec![
                CompletionItem {
                    value: "set".to_string(),
                    description: Some("Set a config value".to_string()),
                },
                CompletionItem {
                    value: "get".to_string(),
                    description: Some("Get a config value".to_string()),
                },
            ];
            (filter_by_prefix(items, arg_text), " Config ")
        }
        1 => {
            // Config keys (only after "set")
            let items: Vec<CompletionItem> = SETTABLE_CONFIG_KEYS
                .iter()
                .map(|key| CompletionItem {
                    value: key.to_string(),
                    description: None,
                })
                .collect();
            (filter_by_prefix(items, arg_text), " Config Keys ")
        }
        2 => {
            // Value completions for specific keys — only thinking_level has enumerable values
            let items = vec![
                CompletionItem {
                    value: "low".to_string(),
                    description: None,
                },
                CompletionItem {
                    value: "medium".to_string(),
                    description: None,
                },
                CompletionItem {
                    value: "high".to_string(),
                    description: None,
                },
                CompletionItem {
                    value: "off".to_string(),
                    description: None,
                },
            ];
            (filter_by_prefix(items, arg_text), " Values ")
        }
        _ => (vec![], " Config "),
    }
}

// === Dynamic completers ===

/// `/switch <tab>` — agent names (excluding current agent).
pub fn complete_switch(
    arg_text: &str,
    arg_index: usize,
    ctx: &CompletionContext,
) -> (Vec<CompletionItem>, &'static str) {
    if arg_index > 0 {
        return (vec![], " Agents ");
    }
    let agents = list_agents(ctx.global_home);
    let items: Vec<CompletionItem> = agents
        .into_iter()
        .filter(|name| name != ctx.current_agent)
        .map(|name| CompletionItem {
            value: name,
            description: None,
        })
        .collect();
    (filter_by_prefix(items, arg_text), " Agents ")
}

/// `/team <tab>` — team names (only for arg_index 0).
pub fn complete_team(
    arg_text: &str,
    arg_index: usize,
    ctx: &CompletionContext,
) -> (Vec<CompletionItem>, &'static str) {
    if arg_index > 0 {
        return (vec![], " Teams ");
    }
    let teams = list_teams(ctx.home_dir);
    let items: Vec<CompletionItem> = teams
        .into_iter()
        .map(|name| CompletionItem {
            value: name,
            description: None,
        })
        .collect();
    (filter_by_prefix(items, arg_text), " Teams ")
}

/// `/skill <tab>` — skill names.
pub fn complete_skill(
    arg_text: &str,
    arg_index: usize,
    ctx: &CompletionContext,
) -> (Vec<CompletionItem>, &'static str) {
    if arg_index > 0 {
        return (vec![], " Skills ");
    }
    let items: Vec<CompletionItem> = ctx
        .skills
        .skills()
        .iter()
        .map(|entry| CompletionItem {
            value: entry.manifest.skill.name.clone(),
            description: Some(entry.manifest.skill.description.clone()),
        })
        .collect();
    (filter_by_prefix(items, arg_text), " Skills ")
}

/// `/attach <tab>` — file path completion with tilde expansion.
pub fn complete_attach(
    arg_text: &str,
    _arg_index: usize,
    ctx: &CompletionContext,
) -> (Vec<CompletionItem>, &'static str) {
    let items = complete_file_path(arg_text, ctx.cwd);
    (items, " Files ")
}

/// Maximum number of file path completion entries.
const MAX_PATH_ENTRIES: usize = 100;

/// Complete file paths relative to a base directory.
fn complete_file_path(prefix: &str, cwd: &Path) -> Vec<CompletionItem> {
    let expanded = expand_tilde(prefix);
    let (dir, file_prefix) = split_path_for_completion(&expanded, cwd);

    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let show_hidden = file_prefix.starts_with('.');

    let mut items: Vec<CompletionItem> = entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden files unless prefix starts with "."
            if name.starts_with('.') && !show_hidden {
                return None;
            }

            // Filter by prefix
            if !name.to_lowercase().starts_with(&file_prefix.to_lowercase()) {
                return None;
            }

            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

            // Build the full completion value (reconstruct with directory prefix)
            let dir_prefix = if prefix.is_empty() {
                String::new()
            } else if let Some(last_slash) = prefix.rfind('/') {
                prefix[..=last_slash].to_string()
            } else {
                String::new()
            };

            let value = if is_dir {
                format!("{dir_prefix}{name}/")
            } else {
                format!("{dir_prefix}{name}")
            };

            let description = if is_dir {
                Some("directory".to_string())
            } else {
                None
            };

            Some(CompletionItem { value, description })
        })
        .take(MAX_PATH_ENTRIES)
        .collect();

    items.sort_by(|a, b| a.value.cmp(&b.value));
    items
}

/// Expand `~` to the user's home directory.
fn expand_tilde(path: &str) -> String {
    if path.starts_with('~')
        && let Some(home) = std::env::var_os("HOME")
    {
        return path.replacen('~', &home.to_string_lossy(), 1);
    }
    path.to_string()
}

/// Split a path into (directory_to_list, file_prefix) for completion.
fn split_path_for_completion(path: &str, cwd: &Path) -> (std::path::PathBuf, String) {
    if path.is_empty() {
        return (cwd.to_path_buf(), String::new());
    }

    let p = Path::new(path);
    if path.ends_with('/') {
        // User typed a directory path — list its contents
        (p.to_path_buf(), String::new())
    } else if let Some(parent) = p.parent() {
        let file_prefix = p
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let dir = if parent.as_os_str().is_empty() {
            cwd.to_path_buf()
        } else {
            parent.to_path_buf()
        };
        (dir, file_prefix)
    } else {
        (cwd.to_path_buf(), path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mika_agent::skills::SkillRegistry;

    fn test_ctx(cwd: &Path) -> CompletionContext<'_> {
        let registry = SkillRegistry::empty();
        // Leak to get 'static-like lifetime for test — fine in tests
        let skills = Box::leak(Box::new(registry));
        CompletionContext {
            home_dir: Path::new("/tmp/mika-test"),
            global_home: Path::new("/tmp/mika-test"),
            skills,
            current_agent: "main",
            cwd,
        }
    }

    #[test]
    fn test_complete_model_all() {
        let ctx = test_ctx(Path::new("/tmp"));
        let (items, title) = complete_model("", 0, &ctx);
        assert_eq!(items.len(), 3);
        assert_eq!(title, " Models ");
        assert_eq!(items[0].value, "sonnet");
    }

    #[test]
    fn test_complete_model_filtered() {
        let ctx = test_ctx(Path::new("/tmp"));
        let (items, _) = complete_model("s", 0, &ctx);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].value, "sonnet");
    }

    #[test]
    fn test_complete_think_all() {
        let ctx = test_ctx(Path::new("/tmp"));
        let (items, title) = complete_think("", 0, &ctx);
        assert_eq!(items.len(), 4);
        assert_eq!(title, " Think ");
    }

    #[test]
    fn test_complete_think_second_arg_empty() {
        let ctx = test_ctx(Path::new("/tmp"));
        let (items, _) = complete_think("", 1, &ctx);
        assert!(items.is_empty());
    }

    #[test]
    fn test_complete_config_subcommands() {
        let ctx = test_ctx(Path::new("/tmp"));
        let (items, title) = complete_config("", 0, &ctx);
        assert_eq!(items.len(), 2);
        assert_eq!(title, " Config ");
    }

    #[test]
    fn test_complete_config_keys() {
        let ctx = test_ctx(Path::new("/tmp"));
        let (items, title) = complete_config("", 1, &ctx);
        assert_eq!(items.len(), SETTABLE_CONFIG_KEYS.len());
        assert_eq!(title, " Config Keys ");
    }

    #[test]
    fn test_complete_memory_subcommands() {
        let ctx = test_ctx(Path::new("/tmp"));
        let (items, title) = complete_memory("", 0, &ctx);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].value, "search");
        assert_eq!(title, " Memory ");
    }

    #[test]
    fn test_expand_tilde() {
        // Just test the mechanics — HOME may vary
        let result = expand_tilde("/absolute/path");
        assert_eq!(result, "/absolute/path");

        let result = expand_tilde("relative");
        assert_eq!(result, "relative");
    }

    #[test]
    fn test_split_path_empty() {
        let (dir, prefix) = split_path_for_completion("", Path::new("/cwd"));
        assert_eq!(dir, Path::new("/cwd"));
        assert_eq!(prefix, "");
    }

    #[test]
    fn test_split_path_dir_slash() {
        let (dir, prefix) = split_path_for_completion("/tmp/", Path::new("/cwd"));
        assert_eq!(dir, Path::new("/tmp/"));
        assert_eq!(prefix, "");
    }

    #[test]
    fn test_split_path_partial() {
        let (dir, prefix) = split_path_for_completion("/tmp/fo", Path::new("/cwd"));
        assert_eq!(dir, Path::new("/tmp"));
        assert_eq!(prefix, "fo");
    }

    #[test]
    fn test_split_path_relative() {
        let (dir, prefix) = split_path_for_completion("foo", Path::new("/cwd"));
        assert_eq!(dir, Path::new("/cwd"));
        assert_eq!(prefix, "foo");
    }

    #[test]
    fn test_complete_file_path_nonexistent_dir() {
        let items = complete_file_path("/nonexistent/dir/", Path::new("/tmp"));
        assert!(items.is_empty());
    }

    #[test]
    fn test_complete_file_path_real_dir() {
        // /tmp should exist on any Linux/macOS system
        let items = complete_file_path("", Path::new("/tmp"));
        // We don't know exact contents, but it should not panic
        // and items should have values
        for item in &items {
            assert!(!item.value.is_empty());
        }
    }

    #[test]
    fn test_filter_by_prefix_empty() {
        let items = vec![CompletionItem {
            value: "test".to_string(),
            description: None,
        }];
        let filtered = filter_by_prefix(items, "");
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_by_prefix_match() {
        let items = vec![
            CompletionItem {
                value: "sonnet".to_string(),
                description: None,
            },
            CompletionItem {
                value: "opus".to_string(),
                description: None,
            },
        ];
        let filtered = filter_by_prefix(items, "s");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].value, "sonnet");
    }
}
