pub mod builtin_handlers;
pub mod executor;
pub mod git;
pub mod index;
pub mod install;
pub mod manifest;
pub mod marketplace;
pub mod matcher;

use std::path::Path;

use self::index::SkillEntry;
use crate::db::SkillOverride;

/// Registry of discovered skills, built once at startup.
#[derive(Debug)]
pub struct SkillRegistry {
    skills: Vec<SkillEntry>,
}

impl SkillRegistry {
    /// Scan a skills directory and build the registry.
    pub fn from_dir(skills_dir: &Path) -> Self {
        Self {
            skills: index::scan_skills_dir(skills_dir),
        }
    }

    /// Create an empty registry (no skills directory).
    pub fn empty() -> Self {
        Self { skills: Vec::new() }
    }

    /// Match skills against a user message.
    /// Only returns enabled skills.
    pub fn match_message(&self, user_message: &str) -> Vec<&SkillEntry> {
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

    /// Apply database-backed overrides to skill entries.
    ///
    /// For each override, finds the matching skill by name (case-insensitive)
    /// and applies the `always_on` value, marking the entry as overridden.
    pub fn apply_overrides(&mut self, overrides: &[SkillOverride]) {
        for ov in overrides {
            if let Some(always_on) = ov.always_on
                && let Some(entry) = self
                    .skills
                    .iter_mut()
                    .find(|e| e.manifest.skill.name.eq_ignore_ascii_case(&ov.skill_name))
            {
                entry.manifest.skill.always_on = always_on;
                entry.has_override = true;
            }
        }
    }

    /// Return always-on skills that are safe for silent/background mode.
    ///
    /// Filters out skills whose tools use `Exec` or `Http` handlers
    /// (e.g., tmux, shell-exec) since those should not run autonomously
    /// in heartbeat or reminder contexts without user interaction.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::{SkillInfo, SkillManifest, Triggers};
    use std::path::PathBuf;

    fn make_entry(name: &str, always_on: bool, enabled: bool) -> SkillEntry {
        SkillEntry {
            manifest: SkillManifest {
                skill: SkillInfo {
                    name: name.to_string(),
                    description: format!("{name} skill"),
                    version: String::new(),
                    always_on,
                    timeout_secs: 30,
                },
                triggers: Triggers { keywords: vec![] },
            },
            dir: PathBuf::from(format!("/skills/{name}")),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled,
            has_override: false,
        }
    }

    #[test]
    fn test_registry_from_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::from_dir(tmp.path());
        assert!(!registry.has_skills());
        assert!(registry.match_message("hello").is_empty());
    }

    #[test]
    fn test_registry_empty() {
        let registry = SkillRegistry::empty();
        assert!(!registry.has_skills());
    }

    #[test]
    fn test_always_on_skills() {
        let registry = SkillRegistry {
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
    fn test_apply_overrides_sets_always_on() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skills: vec![
                make_entry("web-search", false, true),
                make_entry("tmux", false, true),
            ],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: Some(true),
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
            skills: vec![make_entry("Web-Search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: Some(true),
        }]);

        assert!(registry.skills[0].manifest.skill.always_on);
        assert!(registry.skills[0].has_override);
    }

    #[test]
    fn test_apply_overrides_none_skips() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skills: vec![make_entry("web-search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: None,
        }]);

        assert!(!registry.skills[0].manifest.skill.always_on);
        assert!(!registry.skills[0].has_override);
    }

    #[test]
    fn test_apply_overrides_nonexistent_skill_ignored() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skills: vec![make_entry("web-search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "nonexistent".to_string(),
            always_on: Some(true),
        }]);

        // No crash, web-search unchanged
        assert!(!registry.skills[0].manifest.skill.always_on);
    }

    #[test]
    fn test_apply_overrides_affects_always_on_skills_filter() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skills: vec![
                make_entry("web-search", false, true),
                make_entry("shell-exec", true, true),
            ],
        };

        assert_eq!(registry.always_on_skills().len(), 1);

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: Some(true),
        }]);

        assert_eq!(registry.always_on_skills().len(), 2);
    }
}
