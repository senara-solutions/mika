pub mod index;
pub mod manifest;
pub mod matcher;

use std::collections::HashSet;
use std::path::Path;

use mika_common::claude::ToolDefinition;

use crate::tools::ToolRegistry;

use self::index::SkillEntry;

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
    ///
    /// Used by silent-mode heartbeats where there's no real user message
    /// to match against.
    pub fn always_on_skills(&self) -> Vec<&SkillEntry> {
        self.skills
            .iter()
            .filter(|e| e.manifest.options.always_on)
            .collect()
    }
}

/// Resolve matched skills into merged tool definitions.
///
/// For builtin handlers, tool definitions come from the `ToolRegistry`.
/// Deduplicates by tool name across skills.
pub fn resolve_matched_skills(
    tool_registry: &ToolRegistry,
    matched: &[&SkillEntry],
) -> Vec<ToolDefinition> {
    let mut tool_defs = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    for entry in matched {
        let manifest::Handler::Builtin { tools } = &entry.manifest.handler;
        for tool_name in tools {
            if seen_names.contains(tool_name) {
                continue;
            }
            if let Some(def) = tool_registry.definition_by_name(tool_name) {
                tool_defs.push(def.clone());
                seen_names.insert(tool_name.clone());
            }
        }
    }

    tool_defs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::{Handler, SkillManifest, SkillOptions, Triggers};
    use crate::tools;
    use std::path::PathBuf;

    fn make_builtin_entry(name: &str, tool_names: &[&str], always_on: bool) -> SkillEntry {
        SkillEntry {
            manifest: SkillManifest {
                name: name.to_string(),
                description: format!("{name} skill"),
                triggers: Triggers { keywords: vec![] },
                handler: Handler::Builtin {
                    tools: tool_names.iter().map(|s| s.to_string()).collect(),
                },
                options: SkillOptions {
                    always_on,
                    timeout_secs: 30,
                },
            },
            dir: PathBuf::from(format!("/skills/{name}")),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
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
    fn test_resolve_builtin_skills() {
        let tool_registry = tools::default_tools();
        let entry = make_builtin_entry("memory", &["store_fact", "search_memory"], true);
        let matched: Vec<&SkillEntry> = vec![&entry];

        let defs = resolve_matched_skills(&tool_registry, &matched);
        assert_eq!(defs.len(), 2);
        assert!(defs.iter().any(|d| d.name == "store_fact"));
        assert!(defs.iter().any(|d| d.name == "search_memory"));
    }

    #[test]
    fn test_always_on_skills() {
        let registry = SkillRegistry {
            skills: vec![
                make_builtin_entry("memory", &["store_fact"], true),
                make_builtin_entry("reminders", &["create_reminder"], false),
                make_builtin_entry("messaging", &["send_message"], true),
            ],
        };
        let always_on = registry.always_on_skills();
        assert_eq!(always_on.len(), 2);
        assert_eq!(always_on[0].manifest.name, "memory");
        assert_eq!(always_on[1].manifest.name, "messaging");
    }

    #[test]
    fn test_always_on_skills_empty() {
        let registry = SkillRegistry::empty();
        assert!(registry.always_on_skills().is_empty());
    }

    #[test]
    fn test_resolve_deduplicates() {
        let tool_registry = tools::default_tools();
        let entry1 = make_builtin_entry("memory", &["store_fact"], true);
        let entry2 = make_builtin_entry("memory2", &["store_fact"], true);
        let matched: Vec<&SkillEntry> = vec![&entry1, &entry2];

        let defs = resolve_matched_skills(&tool_registry, &matched);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "store_fact");
    }
}
