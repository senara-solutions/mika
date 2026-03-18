use std::path::{Path, PathBuf};
use tracing::warn;

use mika_common::claude::ToolDefinition;

use super::builtin_handlers::KNOWN_BUILTINS;
use super::manifest::{SkillManifest, SkillToolDef, ToolHandler};

/// Maximum size for skill.toml files (64 KB).
const MAX_SKILL_TOML_SIZE: u64 = 64 * 1024;

/// Default maximum size for system_prompt.md snippets (16 KB).
const MAX_PROMPT_SNIPPET_SIZE: u64 = 16 * 1024;

/// Hard ceiling for per-skill `max_prompt_size` override (64 KB).
/// Prevents marketplace skills from loading arbitrarily large prompts.
const MAX_PROMPT_SIZE_CEILING: u64 = 64 * 1024;

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
    /// Whether this entry has a DB override applied (for display purposes).
    pub has_override: bool,
}

/// Result of scanning a skills directory.
pub struct ScanResult {
    pub entries: Vec<SkillEntry>,
    pub skipped_count: usize,
}

/// Scan a skills directory and load all valid skill manifests.
///
/// Each immediate subdirectory is expected to contain a `skill.toml`.
/// Invalid skills are logged at `warn` and skipped — never break startup.
/// Legacy-format skills (has `[handler]` section) are skipped with a
/// deprecation warning. Returns entries and the count of skipped directories.
pub fn scan_skills_dir(skills_dir: &Path) -> ScanResult {
    let read_dir = match std::fs::read_dir(skills_dir) {
        Ok(rd) => rd,
        Err(e) => {
            warn!(path = %skills_dir.display(), error = %e, "cannot read skills directory");
            return ScanResult {
                entries: Vec::new(),
                skipped_count: 0,
            };
        }
    };

    let mut entries = Vec::new();
    let mut skipped_count: usize = 0;
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
            skipped_count += 1;
            continue;
        }

        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "cannot read skill manifest");
                skipped_count += 1;
                continue;
            }
        };

        // Detect legacy format: has [handler] section with type = "builtin"
        if is_legacy_format(&content) {
            warn!(
                path = %manifest_path.display(),
                "skipping legacy-format skill (has [handler] section). \
                 Migrate to new [skill] section format — handler config belongs in tools.json."
            );
            skipped_count += 1;
            continue;
        }

        let manifest: SkillManifest = match toml::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "invalid skill manifest");
                skipped_count += 1;
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
        let max_size = manifest
            .skill
            .max_prompt_size
            .map(|v| v.min(MAX_PROMPT_SIZE_CEILING))
            .unwrap_or(MAX_PROMPT_SNIPPET_SIZE);
        if let Some(requested) = manifest.skill.max_prompt_size
            && requested > MAX_PROMPT_SIZE_CEILING
        {
            warn!(
                skill = %manifest.skill.name,
                requested = requested,
                ceiling = MAX_PROMPT_SIZE_CEILING,
                "max_prompt_size exceeds ceiling, clamping"
            );
        }
        let prompt_snippet = load_snippet_with_limit(&snippet_path, max_size);

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
            has_override: false,
        });
    }

    ScanResult {
        entries,
        skipped_count,
    }
}

/// Diagnostic level for skill validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Ok,
    Warn,
    Fail,
}

/// A single diagnostic finding from skill validation.
#[derive(Debug, Clone)]
pub struct SkillDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

impl SkillDiagnostic {
    fn ok(msg: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Ok,
            message: msg.into(),
        }
    }
    fn warn(msg: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Warn,
            message: msg.into(),
        }
    }
    fn fail(msg: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Fail,
            message: msg.into(),
        }
    }

    pub fn tag(&self) -> &'static str {
        match self.level {
            DiagnosticLevel::Ok => "[OK]",
            DiagnosticLevel::Warn => "[WARN]",
            DiagnosticLevel::Fail => "[FAIL]",
        }
    }
}

/// Validate a single skill directory and return diagnostics.
pub fn validate_skill(skill_dir: &Path) -> Vec<SkillDiagnostic> {
    let mut diags = Vec::new();

    // 1. Check skill.toml exists and is readable
    let manifest_path = skill_dir.join("skill.toml");
    if !manifest_path.exists() {
        diags.push(SkillDiagnostic::fail("skill.toml not found"));
        return diags;
    }

    // Check file size
    if let Ok(meta) = std::fs::metadata(&manifest_path)
        && meta.len() > MAX_SKILL_TOML_SIZE
    {
        diags.push(SkillDiagnostic::fail(format!(
            "skill.toml exceeds 64KB ({}KB)",
            meta.len() / 1024
        )));
        return diags;
    }

    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            diags.push(SkillDiagnostic::fail(format!(
                "cannot read skill.toml: {e}"
            )));
            return diags;
        }
    };

    // 2. Check for valid TOML and legacy format
    if is_legacy_format(&content) {
        diags.push(SkillDiagnostic::fail(
            "legacy format detected: has [handler] section. \
             Migrate to [skill] section + tools.json per-tool handlers."
                .to_string(),
        ));
        return diags;
    }

    // 3. Parse as SkillManifest
    let manifest: SkillManifest = match toml::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            diags.push(SkillDiagnostic::fail(format!("invalid skill.toml: {e}")));
            return diags;
        }
    };
    diags.push(SkillDiagnostic::ok(format!(
        "skill.toml valid — name={}, description={}",
        manifest.skill.name,
        manifest
            .skill
            .description
            .chars()
            .take(60)
            .collect::<String>()
    )));

    // 4. Check tools.json if present
    let tools_path = skill_dir.join("tools.json");
    if tools_path.exists() {
        if let Ok(meta) = std::fs::metadata(&tools_path)
            && meta.len() > MAX_TOOLS_JSON_SIZE
        {
            diags.push(SkillDiagnostic::fail(format!(
                "tools.json exceeds 256KB ({}KB)",
                meta.len() / 1024
            )));
            return diags;
        }
        match std::fs::read_to_string(&tools_path) {
            Ok(json_content) => {
                match serde_json::from_str::<Vec<super::manifest::SkillToolDef>>(&json_content) {
                    Ok(tools) => {
                        diags.push(SkillDiagnostic::ok(format!(
                            "tools.json valid — {} tool(s)",
                            tools.len()
                        )));

                        // 5. Check exec handler commands exist and are executable
                        for tool in &tools {
                            if let ToolHandler::Exec { command, .. } = &tool.handler {
                                let cmd_path = skill_dir.join(command);
                                if !cmd_path.exists() {
                                    diags.push(SkillDiagnostic::fail(format!(
                                        "tool '{}': handler command not found: {} (resolved to {})",
                                        tool.name,
                                        command,
                                        cmd_path.display()
                                    )));
                                } else {
                                    #[cfg(unix)]
                                    {
                                        use std::os::unix::fs::PermissionsExt;
                                        if let Ok(meta) = std::fs::metadata(&cmd_path) {
                                            if meta.permissions().mode() & 0o111 == 0 {
                                                diags.push(SkillDiagnostic::fail(format!(
                                                    "tool '{}': handler command not executable: {}",
                                                    tool.name,
                                                    cmd_path.display()
                                                )));
                                            } else {
                                                // Symlink containment check
                                                if let Ok(canonical) = cmd_path.canonicalize()
                                                    && !canonical.starts_with(skill_dir)
                                                {
                                                    diags.push(SkillDiagnostic::warn(format!(
                                                        "tool '{}': handler command '{}' resolves outside skill directory",
                                                        tool.name, command
                                                    )));
                                                }
                                                diags.push(SkillDiagnostic::ok(format!(
                                                    "tool '{}': handler command OK",
                                                    tool.name
                                                )));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        diags.push(SkillDiagnostic::fail(format!("invalid tools.json: {e}")));
                    }
                }
            }
            Err(e) => {
                diags.push(SkillDiagnostic::fail(format!(
                    "cannot read tools.json: {e}"
                )));
            }
        }
    }

    // 6. Check prompt snippet size against effective limit
    let snippet_path = skill_dir.join("system_prompt.md");
    if snippet_path.exists() {
        let effective_limit = manifest
            .skill
            .max_prompt_size
            .map(|v| v.min(MAX_PROMPT_SIZE_CEILING))
            .unwrap_or(MAX_PROMPT_SNIPPET_SIZE);

        if let Ok(meta) = std::fs::metadata(&snippet_path) {
            let size = meta.len();
            if size > effective_limit {
                diags.push(SkillDiagnostic::fail(format!(
                    "system_prompt.md ({} bytes) exceeds limit ({} bytes) — snippet will be skipped at startup",
                    size, effective_limit
                )));
            } else if effective_limit > 0 && size > effective_limit * 3 / 4 {
                diags.push(SkillDiagnostic::warn(format!(
                    "system_prompt.md ({} bytes) is above 75% of limit ({} bytes)",
                    size, effective_limit
                )));
            } else {
                diags.push(SkillDiagnostic::ok(format!(
                    "system_prompt.md size OK ({} bytes, limit {} bytes)",
                    size, effective_limit
                )));
            }
        }

        if let Some(requested) = manifest.skill.max_prompt_size
            && requested > MAX_PROMPT_SIZE_CEILING
        {
            diags.push(SkillDiagnostic::warn(format!(
                "max_prompt_size ({} bytes) exceeds ceiling ({} bytes), will be clamped",
                requested, MAX_PROMPT_SIZE_CEILING
            )));
        }
    }

    // 7. Warnings for no-op or never-activates skills
    let has_tools = tools_path.exists();
    let has_snippet = snippet_path.exists();
    if !has_tools && !has_snippet {
        diags.push(SkillDiagnostic::warn(
            "no-op skill: no tools.json and no system_prompt.md",
        ));
    }
    if !manifest.skill.always_on && manifest.triggers.keywords.is_empty() {
        diags.push(SkillDiagnostic::warn(
            "skill will never activate: not always_on and no trigger keywords",
        ));
    }

    diags
}

/// Detect whether a skill.toml uses the legacy flat format.
///
/// Legacy format has a top-level `[handler]` section with a `type` field
/// (any handler type: builtin, exec, http). New format wraps skill metadata
/// under `[skill]` and puts handler config in tools.json per-tool.
fn is_legacy_format(content: &str) -> bool {
    // Parse as generic TOML table and check for legacy markers
    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(_) => return false,
    };
    // Legacy format has top-level "handler" table with any "type" field
    if let Some(handler) = table.get("handler").and_then(|v| v.as_table())
        && handler.get("type").and_then(|v| v.as_str()).is_some()
    {
        return true;
    }
    false
}

/// Inject `work_item_id` as a required field into a tool's input schema.
///
/// Long-running exec handlers must track delegation via work items.
/// This adds the field to the JSON schema so the LLM knows to include it.
fn inject_work_item_id_field(schema: &mut serde_json::Value) {
    if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
        props.insert(
            "work_item_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "ID of the work item tracking this task. Create one first using create_work_item."
            }),
        );
    }
    if let Some(required) = schema.get_mut("required").and_then(|r| r.as_array_mut()) {
        required.push(serde_json::Value::String("work_item_id".to_string()));
    } else {
        schema["required"] = serde_json::json!(["work_item_id"]);
    }
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
        .map(|def| {
            let mut schema = def.input_schema;

            // Long-running exec handlers require a work_item_id for delegation tracking
            if let ToolHandler::Exec {
                long_running: true, ..
            } = &def.handler
            {
                inject_work_item_id_field(&mut schema);
            }

            ResolvedSkillTool {
                definition: ToolDefinition {
                    name: def.name,
                    description: def.description,
                    input_schema: schema,
                },
                handler: def.handler,
                skill_dir: skill_dir.to_path_buf(),
            }
        })
        .collect()
}

/// Load a prompt snippet file with size limit enforcement.
fn load_snippet_with_limit(path: &Path, max_size: u64) -> String {
    if let Ok(meta) = std::fs::metadata(path)
        && meta.len() > max_size
    {
        warn!(
            path = %path.display(),
            size = meta.len(),
            limit = max_size,
            "prompt snippet exceeds size limit, skipping"
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

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.skipped_count, 0);
        assert_eq!(scan.entries[0].manifest.skill.name, "web-search");
        assert_eq!(scan.entries[0].keywords_lower, vec!["search", "look up"]);
        assert_eq!(scan.entries[0].dir, skill_dir);
        assert!(scan.entries[0].enabled);
        assert!(scan.entries[0].skill_tools.is_empty());
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

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.skipped_count, 1);
        assert_eq!(scan.entries[0].manifest.skill.name, "web-search");
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

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.skipped_count, 2); // bad TOML + missing manifest
        assert_eq!(scan.entries[0].manifest.skill.name, "good");
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let scan = scan_skills_dir(tmp.path());
        assert!(scan.entries.is_empty());
    }

    #[test]
    fn test_scan_nonexistent_dir() {
        let scan = scan_skills_dir(Path::new("/nonexistent/skills"));
        assert!(scan.entries.is_empty());
    }

    #[test]
    fn test_scan_ignores_files() {
        let tmp = tempfile::tempdir().unwrap();
        // A file (not a directory) in the skills dir should be skipped
        fs::write(tmp.path().join("readme.txt"), "not a skill").unwrap();
        let scan = scan_skills_dir(tmp.path());
        assert!(scan.entries.is_empty());
    }

    #[test]
    fn test_scan_skips_oversized_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("big");
        fs::create_dir_all(&skill_dir).unwrap();
        // Write a file larger than 64KB
        let big_content = "x".repeat(65 * 1024);
        fs::write(skill_dir.join("skill.toml"), &big_content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert!(scan.entries.is_empty());
        assert_eq!(scan.skipped_count, 1);
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

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "Use web search wisely.");
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

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "");
    }

    #[test]
    fn test_snippet_size_limit_default() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        // Write a file larger than 16KB default
        let big_content = "x".repeat(17 * 1024);
        fs::write(&path, &big_content).unwrap();

        let snippet = load_snippet_with_limit(&path, MAX_PROMPT_SNIPPET_SIZE);
        assert_eq!(snippet, "");
    }

    #[test]
    fn test_snippet_size_limit_custom() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        // 10KB file — under 16KB default, tested with explicit 32KB limit
        let content = "x".repeat(10 * 1024);
        fs::write(&path, &content).unwrap();

        let snippet = load_snippet_with_limit(&path, 32 * 1024);
        assert_eq!(snippet.len(), 10 * 1024);
    }

    #[test]
    fn test_snippet_size_limit_zero_always_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        fs::write(&path, "tiny").unwrap();

        let snippet = load_snippet_with_limit(&path, 0);
        assert_eq!(snippet, "");
    }

    #[test]
    fn test_snippet_under_default_limit_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        let content = "x".repeat(15 * 1024); // 15KB, under 16KB default
        fs::write(&path, &content).unwrap();

        let snippet = load_snippet_with_limit(&path, MAX_PROMPT_SNIPPET_SIZE);
        assert_eq!(snippet.len(), 15 * 1024);
    }

    #[test]
    fn test_scan_loads_large_snippet_with_override() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("big-prompt");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "big-prompt"
            description = "Skill with large prompt"
            max_prompt_size = 32768
            "#,
        )
        .unwrap();
        // 20KB prompt — over 16KB default but under 32KB override
        let content = "x".repeat(20 * 1024);
        fs::write(skill_dir.join("system_prompt.md"), &content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet.len(), 20 * 1024);
    }

    #[test]
    fn test_scan_clamps_override_to_ceiling() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("huge-prompt");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "huge-prompt"
            description = "Skill requesting too much"
            max_prompt_size = 1048576
            "#,
        )
        .unwrap();
        // 100KB prompt — over 64KB ceiling
        let content = "x".repeat(100 * 1024);
        fs::write(skill_dir.join("system_prompt.md"), &content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        // Should be empty because 100KB > 64KB ceiling
        assert_eq!(scan.entries[0].prompt_snippet, "");
    }

    #[test]
    fn test_scan_skips_snippet_over_default() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("too-big");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "too-big"
            description = "Prompt over default limit"
            "#,
        )
        .unwrap();
        // 17KB prompt — over 16KB default
        let content = "x".repeat(17 * 1024);
        fs::write(skill_dir.join("system_prompt.md"), &content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "");
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

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert!(!scan.entries[0].enabled);
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

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].skill_tools.len(), 1);
        assert_eq!(scan.entries[0].skill_tools[0].definition.name, "web_search");
        assert_eq!(scan.entries[0].skill_tools[0].skill_dir, skill_dir);
        assert!(matches!(
            &scan.entries[0].skill_tools[0].handler,
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

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.entries[0].skill_tools.is_empty());
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

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.entries[0].skill_tools.is_empty());
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

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(
            scan.entries[0].skill_tools.len(),
            1,
            "unknown builtin should be filtered out"
        );
        assert_eq!(scan.entries[0].skill_tools[0].definition.name, "valid_tool");
    }

    #[test]
    fn test_is_legacy_format() {
        // Legacy builtin handler
        assert!(is_legacy_format(
            r#"
            name = "memory"
            description = "Memory"
            [handler]
            type = "builtin"
            tools = ["store_fact"]
            "#
        ));

        // Legacy exec handler (also detected now)
        assert!(is_legacy_format(
            r#"
            name = "weather"
            description = "Weather"
            [handler]
            type = "exec"
            command = "./handler.sh"
            tools = ["get_forecast"]
            "#
        ));

        // Legacy http handler
        assert!(is_legacy_format(
            r#"
            name = "weather"
            description = "Weather"
            [handler]
            type = "http"
            url = "http://localhost:8080/tools"
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

    #[test]
    fn test_long_running_tool_gets_work_item_id_injected() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("builder");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "builder"
            description = "Long-running builder"
            "#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "build_project",
                "description": "Build a project",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "Build command"}
                    },
                    "required": ["command"]
                },
                "handler": {"type": "exec", "command": "./build.sh", "long_running": true, "estimated_duration_secs": 300}
            }]"#,
        )
        .unwrap();

        let result = scan_skills_dir(tmp.path());
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].skill_tools.len(), 1);

        let schema = &result.entries[0].skill_tools[0].definition.input_schema;
        // work_item_id should be in properties
        assert!(
            schema["properties"]["work_item_id"].is_object(),
            "work_item_id property should be injected for long_running tools"
        );
        // work_item_id should be required
        let required = schema["required"].as_array().unwrap();
        assert!(
            required.contains(&serde_json::Value::String("work_item_id".to_string())),
            "work_item_id should be in required fields"
        );
    }

    #[test]
    fn test_validate_skill_prompt_size_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("small-prompt");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "small-prompt"
            description = "Small prompt skill"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Small prompt.").unwrap();

        let diags = validate_skill(&skill_dir);
        let prompt_diag = diags
            .iter()
            .find(|d| d.message.contains("system_prompt.md size OK"));
        assert!(prompt_diag.is_some());
    }

    #[test]
    fn test_validate_skill_prompt_oversized() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("big-prompt");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "big-prompt"
            description = "Big prompt skill"
            "#,
        )
        .unwrap();
        // 17KB — over 16KB default
        fs::write(
            skill_dir.join("system_prompt.md"),
            "x".repeat(17 * 1024),
        )
        .unwrap();

        let diags = validate_skill(&skill_dir);
        let fail_diag = diags
            .iter()
            .find(|d| d.message.contains("exceeds limit") && d.level == DiagnosticLevel::Fail);
        assert!(fail_diag.is_some());
    }

    #[test]
    fn test_validate_skill_prompt_near_limit_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("near-limit");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "near-limit"
            description = "Near limit prompt"
            "#,
        )
        .unwrap();
        // 13KB — above 75% of 16KB (12288) but under 16KB
        fs::write(
            skill_dir.join("system_prompt.md"),
            "x".repeat(13 * 1024),
        )
        .unwrap();

        let diags = validate_skill(&skill_dir);
        let warn_diag = diags
            .iter()
            .find(|d| d.message.contains("above 75%") && d.level == DiagnosticLevel::Warn);
        assert!(warn_diag.is_some());
    }

    #[test]
    fn test_validate_skill_prompt_with_override() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("override-prompt");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "override-prompt"
            description = "Skill with override"
            max_prompt_size = 32768
            "#,
        )
        .unwrap();
        // 20KB — over 16KB default but under 32KB override
        fs::write(
            skill_dir.join("system_prompt.md"),
            "x".repeat(20 * 1024),
        )
        .unwrap();

        let diags = validate_skill(&skill_dir);
        // Should NOT have a fail diagnostic — 20KB is under 32KB override
        let fail_diag = diags
            .iter()
            .find(|d| d.message.contains("exceeds limit"));
        assert!(fail_diag.is_none());
        // Should have an OK diagnostic
        let ok_diag = diags
            .iter()
            .find(|d| d.message.contains("system_prompt.md size OK"));
        assert!(ok_diag.is_some());
    }

    #[test]
    fn test_non_long_running_tool_no_work_item_id() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "search"
            description = "Web search"
            "#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "web_search",
                "description": "Search the web",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    },
                    "required": ["query"]
                },
                "handler": {"type": "exec", "command": "./search.sh"}
            }]"#,
        )
        .unwrap();

        let result = scan_skills_dir(tmp.path());
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].skill_tools.len(), 1);

        let schema = &result.entries[0].skill_tools[0].definition.input_schema;
        // work_item_id should NOT be injected for non-long-running tools
        assert!(
            schema["properties"]["work_item_id"].is_null(),
            "work_item_id should not be injected for non-long_running tools"
        );
    }
}
