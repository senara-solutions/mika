use std::path::{Path, PathBuf};
use tracing::warn;

use mika_common::claude::ToolDefinition;

use super::builtin_handlers::KNOWN_BUILTINS;
use super::manifest::{SkillManifest, SkillToolDef, ToolHandler};

/// Maximum size for skill.toml files (64 KB).
const MAX_SKILL_TOML_SIZE: u64 = 64 * 1024;

/// Maximum size for system_prompt.md snippets (8 KB).
const MAX_PROMPT_SNIPPET_SIZE: u64 = 8 * 1024;

/// Maximum size for tools.json files (256 KB).
const MAX_TOOLS_JSON_SIZE: u64 = 256 * 1024;

/// A skill tool with its Claude-facing definition and dispatch handler.
#[derive(Debug, Clone)]
pub struct ResolvedSkillTool {
    pub definition: ToolDefinition,
    pub handler: ToolHandler,
    pub skill_dir: PathBuf,
}

/// A loaded skill entry with its manifest and pre-processed data.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub manifest: SkillManifest,
    pub dir: PathBuf,
    /// Pre-lowercased keywords for fast substring matching.
    pub keywords_lower: Vec<String>,
    /// Cached prompt snippet content (loaded at startup, empty if no file).
    pub prompt_snippet: String,
    /// Tools defined in this skill's `tools.json`.
    pub skill_tools: Vec<ResolvedSkillTool>,
    /// Whether the skill is enabled (no `.disabled` marker file).
    pub enabled: bool,
}

/// Scan a skills directory and load all valid skill manifests.
///
/// Each immediate subdirectory is expected to contain a `skill.toml`.
/// Invalid skills are logged at `warn` and skipped — never break startup.
/// Legacy-format skills (flat fields + `[handler]` with `type = "builtin"`) are
/// skipped with a deprecation warning.
pub fn scan_skills_dir(skills_dir: &Path) -> Vec<SkillEntry> {
    let read_dir = match std::fs::read_dir(skills_dir) {
        Ok(rd) => rd,
        Err(e) => {
            warn!(path = %skills_dir.display(), error = %e, "cannot read skills directory");
            return Vec::new();
        }
    };

    let mut entries = Vec::new();
    for dir_entry in read_dir {
        let dir_entry = match dir_entry {
            Ok(de) => de,
            Err(e) => {
                warn!(error = %e, "error reading skills directory entry");
                continue;
            }
        };

        let path = dir_entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("skill.toml");

        // Check file size before reading to prevent OOM from oversized files
        if let Ok(meta) = std::fs::metadata(&manifest_path)
            && meta.len() > MAX_SKILL_TOML_SIZE
        {
            warn!(
                path = %manifest_path.display(),
                size = meta.len(),
                "skill.toml exceeds 64KB, skipping"
            );
            continue;
        }

        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "cannot read skill manifest");
                continue;
            }
        };

        // Detect legacy format: has [handler] section with type = "builtin"
        if is_legacy_format(&content) {
            warn!(
                path = %manifest_path.display(),
                "skipping legacy-format skill (flat fields + [handler] type=\"builtin\"). \
                 Migrate to new [skill] section format."
            );
            continue;
        }

        let manifest: SkillManifest = match toml::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "invalid skill manifest");
                continue;
            }
        };

        let keywords_lower = manifest
            .triggers
            .keywords
            .iter()
            .map(|k| k.to_lowercase())
            .collect();

        // Load prompt snippet eagerly at startup (cached in SkillEntry)
        let snippet_path = path.join("system_prompt.md");
        let prompt_snippet = load_snippet_with_limit(&snippet_path);

        // Check for .disabled marker file
        let enabled = !path.join(".disabled").exists();

        // Parse tools.json if present
        let skill_tools = load_tools_json(&path);

        entries.push(SkillEntry {
            manifest,
            dir: path,
            keywords_lower,
            prompt_snippet,
            skill_tools,
            enabled,
        });
    }

    entries
}

/// Detect whether a skill.toml uses the legacy flat format.
///
/// Legacy format has top-level `name` field and a `[handler]` section.
/// New format wraps everything under `[skill]`.
fn is_legacy_format(content: &str) -> bool {
    // Parse as generic TOML table and check for legacy markers
    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(_) => return false,
    };
    // Legacy format has top-level "handler" table with type = "builtin"
    if let Some(handler) = table.get("handler").and_then(|v| v.as_table())
        && handler
            .get("type")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == "builtin")
    {
        return true;
    }
    false
}

/// Load and parse `tools.json` from a skill directory.
///
/// Returns an empty vec if the file doesn't exist or is invalid.
fn load_tools_json(skill_dir: &Path) -> Vec<ResolvedSkillTool> {
    let tools_path = skill_dir.join("tools.json");

    // Check file size
    if let Ok(meta) = std::fs::metadata(&tools_path)
        && meta.len() > MAX_TOOLS_JSON_SIZE
    {
        warn!(
            path = %tools_path.display(),
            size = meta.len(),
            "tools.json exceeds 256KB, skipping"
        );
        return Vec::new();
    }

    let content = match std::fs::read_to_string(&tools_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(), // File doesn't exist — normal for prompt-only skills
    };

    let tool_defs: Vec<SkillToolDef> = match serde_json::from_str(&content) {
        Ok(defs) => defs,
        Err(e) => {
            warn!(path = %tools_path.display(), error = %e, "invalid tools.json");
            return Vec::new();
        }
    };

    tool_defs
        .into_iter()
        .filter(|def| {
            if let ToolHandler::Builtin { function } = &def.handler
                && !KNOWN_BUILTINS.contains(&function.as_str())
            {
                warn!(
                    path = %tools_path.display(),
                    function = %function,
                    tool = %def.name,
                    "unknown builtin function, skipping tool"
                );
                return false;
            }
            true
        })
        .map(|def| ResolvedSkillTool {
            definition: ToolDefinition {
                name: def.name,
                description: def.description,
                input_schema: def.input_schema,
            },
            handler: def.handler,
            skill_dir: skill_dir.to_path_buf(),
        })
        .collect()
}

/// Load a prompt snippet file with size limit enforcement.
fn load_snippet_with_limit(path: &Path) -> String {
    if let Ok(meta) = std::fs::metadata(path)
        && meta.len() > MAX_PROMPT_SNIPPET_SIZE
    {
        warn!(
            path = %path.display(),
            size = meta.len(),
            "prompt snippet exceeds 8KB, skipping"
        );
        return String::new();
    }
    std::fs::read_to_string(path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_valid_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            version = "0.1.0"

            [triggers]
            keywords = ["Search", "LOOK UP"]
            "#,
        )
        .unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.skill.name, "web-search");
        assert_eq!(entries[0].keywords_lower, vec!["search", "look up"]);
        assert_eq!(entries[0].dir, skill_dir);
        assert!(entries[0].enabled);
        assert!(entries[0].skill_tools.is_empty());
    }

    #[test]
    fn test_scan_skips_legacy_format() {
        let tmp = tempfile::tempdir().unwrap();

        // Legacy format skill (should be skipped)
        let legacy = tmp.path().join("memory");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("skill.toml"),
            r#"
            name = "memory"
            description = "Memory tools"
            [triggers]
            keywords = ["remember"]
            [handler]
            type = "builtin"
            tools = ["store_fact"]
            [options]
            always_on = true
            "#,
        )
        .unwrap();

        // New format skill (should be loaded)
        let new_skill = tmp.path().join("web-search");
        fs::create_dir_all(&new_skill).unwrap();
        fs::write(
            new_skill.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.skill.name, "web-search");
    }

    #[test]
    fn test_scan_skips_invalid_manifests() {
        let tmp = tempfile::tempdir().unwrap();

        // Valid skill
        let valid = tmp.path().join("good");
        fs::create_dir_all(&valid).unwrap();
        fs::write(
            valid.join("skill.toml"),
            r#"
            [skill]
            name = "good"
            description = "Valid"
            "#,
        )
        .unwrap();

        // Invalid skill (bad TOML)
        let bad = tmp.path().join("bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("skill.toml"), "this is not valid toml {{{}}}").unwrap();

        // Missing manifest
        let missing = tmp.path().join("missing");
        fs::create_dir_all(&missing).unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.skill.name, "good");
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = scan_skills_dir(tmp.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn test_scan_nonexistent_dir() {
        let entries = scan_skills_dir(Path::new("/nonexistent/skills"));
        assert!(entries.is_empty());
    }

    #[test]
    fn test_scan_ignores_files() {
        let tmp = tempfile::tempdir().unwrap();
        // A file (not a directory) in the skills dir should be skipped
        fs::write(tmp.path().join("readme.txt"), "not a skill").unwrap();
        let entries = scan_skills_dir(tmp.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn test_scan_skips_oversized_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("big");
        fs::create_dir_all(&skill_dir).unwrap();
        // Write a file larger than 64KB
        let big_content = "x".repeat(65 * 1024);
        fs::write(skill_dir.join("skill.toml"), &big_content).unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn test_scan_loads_prompt_snippet() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Use web search wisely.").unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].prompt_snippet, "Use web search wisely.");
    }

    #[test]
    fn test_scan_missing_prompt_snippet_defaults_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].prompt_snippet, "");
    }

    #[test]
    fn test_snippet_size_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        // Write a file larger than 8KB
        let big_content = "x".repeat(9 * 1024);
        fs::write(&path, &big_content).unwrap();

        let snippet = load_snippet_with_limit(&path);
        assert_eq!(snippet, "");
    }

    #[test]
    fn test_disabled_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        // Create .disabled marker
        fs::write(skill_dir.join(".disabled"), "").unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].enabled);
    }

    #[test]
    fn test_tools_json_loading() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "web_search",
                "description": "Search the web for information",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query"}
                    },
                    "required": ["query"]
                },
                "handler": {"type": "exec", "command": "./handlers/search.sh"}
            }]"#,
        )
        .unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].skill_tools.len(), 1);
        assert_eq!(entries[0].skill_tools[0].definition.name, "web_search");
        assert_eq!(entries[0].skill_tools[0].skill_dir, skill_dir);
        assert!(matches!(
            &entries[0].skill_tools[0].handler,
            super::super::manifest::ToolHandler::Exec { command, .. } if command == "./handlers/search.sh"
        ));
    }

    #[test]
    fn test_tools_json_oversized() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("big-tools");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "big-tools"
            description = "Oversized tools"
            "#,
        )
        .unwrap();
        let big_json = "x".repeat(257 * 1024);
        fs::write(skill_dir.join("tools.json"), &big_json).unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert!(entries[0].skill_tools.is_empty());
    }

    #[test]
    fn test_tools_json_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("bad-tools");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "bad-tools"
            description = "Invalid tools"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("tools.json"), "not json").unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert!(entries[0].skill_tools.is_empty());
    }

    #[test]
    fn test_tools_json_unknown_builtin_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("bad-builtin");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "bad-builtin"
            description = "Has unknown builtin"
            "#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("tools.json"),
            r#"[
                {
                    "name": "valid_tool",
                    "description": "Valid builtin",
                    "input_schema": {"type": "object", "properties": {}},
                    "handler": {"type": "builtin", "function": "get_documentation"}
                },
                {
                    "name": "bad_tool",
                    "description": "Unknown builtin",
                    "input_schema": {"type": "object", "properties": {}},
                    "handler": {"type": "builtin", "function": "get_clii_reference"}
                }
            ]"#,
        )
        .unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].skill_tools.len(),
            1,
            "unknown builtin should be filtered out"
        );
        assert_eq!(entries[0].skill_tools[0].definition.name, "valid_tool");
    }

    #[test]
    fn test_is_legacy_format() {
        assert!(is_legacy_format(
            r#"
            name = "memory"
            description = "Memory"
            [handler]
            type = "builtin"
            tools = ["store_fact"]
            "#
        ));

        // New format is NOT legacy
        assert!(!is_legacy_format(
            r#"
            [skill]
            name = "web-search"
            description = "Search"
            "#
        ));

        // Invalid TOML is not legacy
        assert!(!is_legacy_format("{{not toml}}"));
    }
}
