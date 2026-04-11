pub mod builtin_handlers;
pub mod context;
pub mod executor;
pub mod git;
pub mod index;
pub mod install;
pub mod manifest;
pub mod marketplace;
pub mod matcher;

use std::path::Path;

use self::index::{SkillEntry, SkippedSkill};
use crate::db::SkillOverride;

/// Registry of discovered skills, built once at startup.
#[derive(Debug)]
pub struct SkillRegistry {
    skills: Vec<SkillEntry>,
    skipped: Vec<SkippedSkill>,
}

impl SkillRegistry {
    /// Scan a skills directory and build the registry.
    pub fn from_dir(skills_dir: &Path) -> Self {
        let result = index::scan_skills_dir(skills_dir);
        if !result.skipped.is_empty() {
            tracing::warn!(
                count = result.skipped.len(),
                "skipped invalid skill(s) at startup — run `mika skills validate` for details"
            );
        }
        // Log successfully loaded skills
        let loaded_names: Vec<&str> = result
            .entries
            .iter()
            .map(|e| e.manifest.skill.name.as_str())
            .collect();
        tracing::info!(
            count = loaded_names.len(),
            skipped = result.skipped.len(),
            names = ?loaded_names,
            "skills loaded"
        );
        Self {
            skills: result.entries,
            skipped: result.skipped,
        }
    }

    /// Create an empty registry (no skills directory).
    pub fn empty() -> Self {
        Self {
            skills: Vec::new(),
            skipped: Vec::new(),
        }
    }

    /// Create a registry with pre-populated skipped skills (for testing/display).
    pub fn with_skipped(skipped: Vec<SkippedSkill>) -> Self {
        Self {
            skills: Vec::new(),
            skipped,
        }
    }

    /// Number of skill directories that were skipped during scan (invalid, legacy, etc.).
    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }

    /// Details of skills that were skipped during scan (name + reason).
    pub fn skipped(&self) -> &[SkippedSkill] {
        &self.skipped
    }

    /// Match skills against a user message.
    /// Only returns enabled skills, annotated with match reason.
    pub fn match_message(&self, user_message: &str) -> Vec<matcher::MatchedSkill<'_>> {
        matcher::match_skills(&self.skills, user_message)
    }

    /// Whether any skills are loaded.
    pub fn has_skills(&self) -> bool {
        !self.skills.is_empty()
    }

    /// Access the underlying skill entries.
    pub fn skills(&self) -> &[SkillEntry] {
        &self.skills
    }

    /// Return all always-on skills (no keyword matching needed).
    /// Only returns enabled skills.
    ///
    /// Used by silent-mode heartbeats where there's no real user message
    /// to match against.
    pub fn always_on_skills(&self) -> Vec<&SkillEntry> {
        self.skills
            .iter()
            .filter(|e| e.enabled && e.manifest.skill.always_on)
            .collect()
    }

    /// Apply database-backed overrides to skill entries and validate dependencies.
    ///
    /// For each override, finds the matching skill by name (case-insensitive)
    /// and applies the `always_on` value, marking the entry as overridden.
    ///
    /// After applying overrides, logs warnings for any declared dependency that
    /// doesn't match an installed skill name. Does not fail — only emits
    /// `tracing::warn` for each unresolvable dependency.
    pub fn apply_overrides(&mut self, overrides: &[SkillOverride]) {
        for ov in overrides {
            let Some(entry) = self
                .skills
                .iter_mut()
                .find(|e| e.manifest.skill.name.eq_ignore_ascii_case(&ov.skill_name))
            else {
                continue;
            };

            if let Some(always_on) = ov.always_on {
                entry.manifest.skill.always_on = always_on;
                entry.has_override = true;
            }

            if ov.llm_provider.is_some() || ov.llm_model.is_some() {
                if let Some(p) = &ov.llm_provider {
                    entry.manifest.llm.provider = Some(p.clone());
                }
                if let Some(m) = &ov.llm_model {
                    entry.manifest.llm.model = Some(m.clone());
                }
                entry.has_override = true;
            }
        }

        // Validate dependencies after overrides are applied
        for entry in &self.skills {
            for dep in &entry.manifest.skill.dependencies {
                if !self
                    .skills
                    .iter()
                    .any(|e| e.manifest.skill.name.eq_ignore_ascii_case(dep))
                {
                    tracing::warn!(
                        skill = %entry.manifest.skill.name,
                        dependency = %dep,
                        "skill declares dependency on unknown skill"
                    );
                }
            }
        }

        // Post-override validation: remove always_on skills with empty prompts
        // caused by oversized prompt files. This catches the edge case where a DB
        // override flips always_on=true on a skill whose prompt was already silently
        // emptied during scan (because it was not always_on at scan time).
        //
        // Collect removed skills into `skipped` for TUI visibility.
        let mut removed = Vec::new();
        self.skills.retain(|entry| {
            if entry.manifest.skill.always_on && entry.has_override && entry.prompt_snippet.is_empty()
            {
                // Check if the skill has a prompt file that exceeds its size limit
                let snippet_path = entry.dir.join("system_prompt.md");
                let effective_limit = entry
                    .manifest
                    .skill
                    .max_prompt_size
                    .map(|v| v.min(index::MAX_PROMPT_SIZE_CEILING))
                    .unwrap_or(index::MAX_PROMPT_SNIPPET_SIZE);
                if let Ok(meta) = std::fs::metadata(&snippet_path)
                    && meta.len() > effective_limit
                {
                    tracing::error!(
                        skill = %entry.manifest.skill.name,
                        size = meta.len(),
                        limit = effective_limit,
                        "always_on skill (via DB override) has oversized prompt — removing from registry. \
                         An always_on skill without its prompt is functionally broken. \
                         Increase max_prompt_size in skill.toml (ceiling: 64KB) or reduce the prompt."
                    );
                    removed.push(SkippedSkill {
                        name: entry.manifest.skill.name.clone(),
                        reason: format!(
                            "removed: always_on override but prompt oversized ({}B, limit {}B)",
                            meta.len(),
                            effective_limit
                        ),
                    });
                    return false;
                }
            }
            true
        });
        self.skipped.extend(removed);
    }

    /// Return always-on skills that are safe for silent/background mode.
    ///
    /// Filters out skills whose tools use `Exec` or `Http` handlers
    /// (e.g., tmux, shell-exec) since those should not run autonomously
    /// in heartbeat or reminder contexts without user interaction.
    ///
    /// Note: This method does NOT resolve skill dependencies (unlike `match_skills()`).
    /// This is intentional — dependency resolution could pull in Exec/Http handler
    /// skills that must not run in autonomous background contexts.
    pub fn safe_always_on_skills(&self) -> Vec<&SkillEntry> {
        use crate::skills::manifest::ToolHandler;

        self.skills
            .iter()
            .filter(|e| {
                e.enabled
                    && e.manifest.skill.always_on
                    && !e.skill_tools.iter().any(|t| {
                        matches!(
                            t.handler,
                            ToolHandler::Exec { .. } | ToolHandler::Http { .. }
                        )
                    })
            })
            .collect()
    }
}

/// Lightweight markdown well-formedness check (#511).
///
/// Returns `Ok(())` for valid-looking markdown, or `Err(description)` for
/// common corruption patterns: empty/whitespace-only, binary content (null
/// bytes or control characters), and unclosed code fences.
///
/// This is intentionally lightweight — no AST parsing or heavyweight
/// dependencies. It catches the most common corruption from generated prompts.
pub(crate) fn validate_markdown_content(content: &str) -> Result<(), String> {
    // 1. Reject empty/whitespace-only
    if content.trim().is_empty() {
        return Err("content is empty or whitespace-only".to_string());
    }
    // 2. Reject binary content (null bytes)
    if content.bytes().any(|b| b == 0) {
        return Err("content contains null bytes — likely binary data".to_string());
    }
    // 3. Reject control characters (except newline, carriage return, tab)
    let control_count = content
        .bytes()
        .filter(|&b| b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t')
        .count();
    if control_count > 0 {
        return Err(format!(
            "content contains {control_count} control character(s) — likely corrupted"
        ));
    }
    // 4. Check for unclosed code fences
    let fence_count = content
        .lines()
        .filter(|l| l.trim_start().starts_with("```"))
        .count();
    if fence_count % 2 != 0 {
        return Err(format!(
            "content has {fence_count} code fence(s) — odd count suggests an unclosed code block"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::{SkillInfo, SkillManifest, Triggers};
    use std::path::PathBuf;

    fn make_entry(name: &str, always_on: bool, enabled: bool) -> SkillEntry {
        make_entry_with_deps(name, always_on, enabled, &[])
    }

    fn make_entry_with_deps(
        name: &str,
        always_on: bool,
        enabled: bool,
        deps: &[&str],
    ) -> SkillEntry {
        SkillEntry {
            manifest: SkillManifest {
                skill: SkillInfo {
                    name: name.to_string(),
                    description: format!("{name} skill"),
                    version: String::new(),
                    always_on,
                    timeout_secs: 30,
                    dependencies: deps.iter().map(|s| s.to_string()).collect(),
                    max_prompt_size: None,
                },
                triggers: Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from(format!("/skills/{name}")),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled,
            has_override: false,
            provider_overrides: std::collections::HashMap::new(),
            model_prompts: std::collections::HashMap::new(),
            model_overrides: std::collections::HashMap::new(),
            generated_model_prompts: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_registry_from_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::from_dir(tmp.path());
        assert!(!registry.has_skills());
        let matched = registry.match_message("hello");
        assert!(matched.is_empty());
    }

    #[test]
    fn test_registry_empty() {
        let registry = SkillRegistry::empty();
        assert!(!registry.has_skills());
    }

    #[test]
    fn test_always_on_skills() {
        let registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![
                make_entry("memory", true, true),
                make_entry("reminders", false, true),
                make_entry("messaging", true, true),
            ],
        };
        let always_on = registry.always_on_skills();
        assert_eq!(always_on.len(), 2);
        assert_eq!(always_on[0].manifest.skill.name, "memory");
        assert_eq!(always_on[1].manifest.skill.name, "messaging");
    }

    #[test]
    fn test_always_on_skills_filters_disabled() {
        let registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![
                make_entry("memory", true, true),
                make_entry("disabled-skill", true, false),
            ],
        };
        let always_on = registry.always_on_skills();
        assert_eq!(always_on.len(), 1);
        assert_eq!(always_on[0].manifest.skill.name, "memory");
    }

    #[test]
    fn test_always_on_skills_empty() {
        let registry = SkillRegistry::empty();
        assert!(registry.always_on_skills().is_empty());
    }

    #[test]
    fn test_safe_always_on_skills_filters_exec_and_http() {
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::ToolHandler;
        use mika_common::claude::ToolDefinition;

        let dummy_def = ToolDefinition {
            name: "dummy".to_string(),
            description: "dummy".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };

        // A safe always-on skill with only builtin tools
        let mut safe_entry = make_entry("memory", true, true);
        safe_entry.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Builtin {
                function: "get_documentation".to_string(),
            },
            skill_dir: PathBuf::from("/skills/memory"),
        }];

        // An unsafe always-on skill with an exec handler
        let mut exec_entry = make_entry("tmux", true, true);
        exec_entry.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Exec {
                command: "./run.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: PathBuf::from("/skills/tmux"),
        }];

        // An unsafe always-on skill with an http handler
        let mut http_entry = make_entry("webhook", true, true);
        http_entry.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Http {
                url: "https://example.com".to_string(),
                method: "POST".to_string(),
            },
            skill_dir: PathBuf::from("/skills/webhook"),
        }];

        // A safe always-on skill with no tools (prompt-only)
        let prompt_only = make_entry("guidelines", true, true);

        let registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![safe_entry, exec_entry, http_entry, prompt_only],
        };

        // always_on_skills returns all 4
        assert_eq!(registry.always_on_skills().len(), 4);

        // safe_always_on_skills filters out exec and http
        let safe = registry.safe_always_on_skills();
        assert_eq!(safe.len(), 2);
        assert_eq!(safe[0].manifest.skill.name, "memory");
        assert_eq!(safe[1].manifest.skill.name, "guidelines");
    }

    #[test]
    fn test_safe_always_on_skills_empty() {
        let registry = SkillRegistry::empty();
        assert!(registry.safe_always_on_skills().is_empty());
    }

    #[test]
    fn test_safe_always_on_skills_excludes_exec_dependency() {
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::ToolHandler;
        use mika_common::claude::ToolDefinition;

        let dummy_def = ToolDefinition {
            name: "dummy".to_string(),
            description: "dummy".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };

        // A safe always-on skill with only builtin tools that depends on "tmux"
        let mut safe_with_dep = make_entry_with_deps("self-knowledge", true, true, &["tmux"]);
        safe_with_dep.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Builtin {
                function: "get_documentation".to_string(),
            },
            skill_dir: PathBuf::from("/skills/self-knowledge"),
        }];

        // An exec-handler skill that is a dependency (not always-on itself)
        let mut exec_dep = make_entry("tmux", false, true);
        exec_dep.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Exec {
                command: "./run.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: PathBuf::from("/skills/tmux"),
        }];

        let registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![safe_with_dep, exec_dep],
        };

        // safe_always_on_skills must NOT resolve dependencies — only the safe
        // builtin-handler skill should be returned, not its exec-handler dependency.
        let safe = registry.safe_always_on_skills();
        assert_eq!(safe.len(), 1);
        assert_eq!(safe[0].manifest.skill.name, "self-knowledge");
    }

    #[test]
    fn test_apply_overrides_sets_always_on() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![
                make_entry("web-search", false, true),
                make_entry("tmux", false, true),
            ],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
        }]);

        assert!(registry.skills[0].manifest.skill.always_on);
        assert!(registry.skills[0].has_override);
        assert!(!registry.skills[1].manifest.skill.always_on);
        assert!(!registry.skills[1].has_override);
    }

    #[test]
    fn test_apply_overrides_case_insensitive_match() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![make_entry("Web-Search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
        }]);

        assert!(registry.skills[0].manifest.skill.always_on);
        assert!(registry.skills[0].has_override);
    }

    #[test]
    fn test_apply_overrides_none_skips() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: None,
            llm_provider: None,
            llm_model: None,
        }]);

        assert!(!registry.skills[0].manifest.skill.always_on);
        assert!(!registry.skills[0].has_override);
    }

    #[test]
    fn test_apply_overrides_nonexistent_skill_ignored() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "nonexistent".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
        }]);

        // No crash, web-search unchanged
        assert!(!registry.skills[0].manifest.skill.always_on);
    }

    #[test]
    fn test_apply_overrides_affects_always_on_skills_filter() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![
                make_entry("web-search", false, true),
                make_entry("shell-exec", true, true),
            ],
        };

        assert_eq!(registry.always_on_skills().len(), 1);

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
        }]);

        assert_eq!(registry.always_on_skills().len(), 2);
    }

    #[test]
    fn test_apply_overrides_validates_dependencies_silent_on_valid() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![
                make_entry_with_deps("self-dev", true, true, &["tmux"]),
                make_entry("tmux", false, true),
            ],
        };
        // Should not panic — valid dependency validated during apply_overrides
        registry.apply_overrides(&[]);
    }

    #[test]
    fn test_apply_overrides_validates_dependencies_warns_on_missing() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![make_entry_with_deps(
                "self-dev",
                true,
                true,
                &["nonexistent"],
            )],
        };
        // Should not panic — logs warning but doesn't fail
        registry.apply_overrides(&[]);
    }

    #[test]
    fn test_apply_overrides_validates_dependencies_case_insensitive() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![
                make_entry_with_deps("self-dev", true, true, &["TMUX"]),
                make_entry("tmux", false, true),
            ],
        };
        // Should not warn — case-insensitive match via eq_ignore_ascii_case
        registry.apply_overrides(&[]);
    }

    #[test]
    fn test_apply_overrides_validates_dependencies_no_deps_no_warn() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![
                make_entry("web-search", true, true),
                make_entry("tmux", false, true),
            ],
        };
        // No dependencies declared — nothing to validate
        registry.apply_overrides(&[]);
    }

    #[test]
    fn test_apply_overrides_removes_always_on_override_with_oversized_prompt() {
        use crate::db::SkillOverride;
        use std::fs;

        // Create a real skill directory with an oversized prompt
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("big-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        // 20KB prompt exceeds 16KB default limit
        let content = "x".repeat(20 * 1024);
        fs::write(skill_dir.join("system_prompt.md"), &content).unwrap();

        // Simulate a skill that was loaded with empty prompt (not always_on at scan time)
        let mut entry = make_entry("big-skill", false, true);
        entry.dir = skill_dir;
        entry.prompt_snippet = String::new(); // emptied due to oversized prompt

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![entry],
        };

        // DB override flips always_on to true
        registry.apply_overrides(&[SkillOverride {
            skill_name: "big-skill".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
        }]);

        // Skill should be removed: always_on + override + empty prompt + oversized file
        assert!(registry.skills.is_empty());
        assert_eq!(registry.skipped_count(), 1);
    }

    #[test]
    fn test_apply_overrides_merges_llm_columns() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![make_entry("qa-review", false, true)],
        };
        // Baseline: manifest [llm] empty.
        assert!(registry.skills[0].manifest.llm.is_empty());

        registry.apply_overrides(&[SkillOverride {
            skill_name: "qa-review".to_string(),
            always_on: None,
            llm_provider: Some("anthropic".to_string()),
            llm_model: Some("claude-sonnet-4-6".to_string()),
        }]);

        assert!(registry.skills[0].has_override);
        assert_eq!(
            registry.skills[0].manifest.llm.provider.as_deref(),
            Some("anthropic")
        );
        assert_eq!(
            registry.skills[0].manifest.llm.model.as_deref(),
            Some("claude-sonnet-4-6")
        );
    }

    #[test]
    fn test_apply_overrides_llm_partial_merges_onto_manifest() {
        use crate::db::SkillOverride;
        use crate::skills::manifest::LlmOverride;

        let mut entry = make_entry("qa-review", false, true);
        // Manifest already sets provider + model (author default).
        entry.manifest.llm = LlmOverride {
            provider: Some("deepseek".to_string()),
            model: Some("deepseek-chat".to_string()),
        };
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![entry],
        };

        // DB override supplies only the model — provider should stay as manifest.
        registry.apply_overrides(&[SkillOverride {
            skill_name: "qa-review".to_string(),
            always_on: None,
            llm_provider: None,
            llm_model: Some("deepseek-reasoner".to_string()),
        }]);

        assert!(registry.skills[0].has_override);
        assert_eq!(
            registry.skills[0].manifest.llm.provider.as_deref(),
            Some("deepseek"),
            "provider should remain as manifest default"
        );
        assert_eq!(
            registry.skills[0].manifest.llm.model.as_deref(),
            Some("deepseek-reasoner"),
            "model should be overridden by DB"
        );
    }

    #[test]
    fn test_apply_overrides_keeps_always_on_override_with_no_prompt_file() {
        use crate::db::SkillOverride;

        // Skill with no prompt file (tool-only) — should NOT be removed
        let mut entry = make_entry("tool-only", false, true);
        entry.prompt_snippet = String::new();
        // dir points to a non-existent path, so metadata check will fail → keep

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            skills: vec![entry],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "tool-only".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
        }]);

        // Should still be present — no prompt file means tool-only, not broken
        assert_eq!(registry.skills.len(), 1);
        assert!(registry.skills[0].manifest.skill.always_on);
    }

    // -- validate_markdown_content tests (#511) --

    #[test]
    fn test_validate_markdown_content_valid() {
        assert!(validate_markdown_content("# Hello\n\nSome text.\n").is_ok());
        assert!(validate_markdown_content("Simple text.").is_ok());
        assert!(validate_markdown_content("```rust\nfn main() {}\n```\n").is_ok());
    }

    #[test]
    fn test_validate_markdown_content_empty() {
        assert!(validate_markdown_content("").is_err());
        assert!(validate_markdown_content("   ").is_err());
        assert!(validate_markdown_content("\n\n").is_err());
    }

    #[test]
    fn test_validate_markdown_content_null_bytes() {
        assert!(validate_markdown_content("hello\0world").is_err());
    }

    #[test]
    fn test_validate_markdown_content_control_chars() {
        assert!(validate_markdown_content("hello\x01world").is_err());
        assert!(validate_markdown_content("hello\x07world").is_err());
    }

    #[test]
    fn test_validate_markdown_content_unclosed_fence() {
        assert!(validate_markdown_content("```\ncode here\n").is_err());
        assert!(validate_markdown_content("text\n```rust\ncode\n").is_err());
    }

    #[test]
    fn test_validate_markdown_content_balanced_fences() {
        assert!(validate_markdown_content("```\ncode\n```\n").is_ok());
        assert!(validate_markdown_content("```rust\ncode\n```\n```\nmore\n```\n").is_ok());
    }

    #[test]
    fn test_validate_markdown_content_allows_common_whitespace() {
        // Tabs, newlines, carriage returns are fine
        assert!(validate_markdown_content("hello\tworld\r\n").is_ok());
    }
}
