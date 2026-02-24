pub mod handler;
pub mod index;
pub mod loader;
pub mod manifest;
pub mod matcher;

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use mika_common::claude::ToolDefinition;

use crate::tools::ToolRegistry;

use self::index::SkillEntry;
use self::manifest::Handler;

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
}

/// Resolve matched skills into merged tool definitions and a dispatch map
/// for external (exec/http) tool handlers.
///
/// For builtin handlers, tool definitions come from the `ToolRegistry`.
/// For exec/http handlers, tool definitions come from `tools.json`.
///
/// Returns:
/// - `Vec<ToolDefinition>`: all tool defs to send to Claude
/// - `HashMap<String, (Handler, Duration)>`: external tool name → handler + timeout
pub async fn resolve_matched_skills(
    tool_registry: &ToolRegistry,
    matched: &[&SkillEntry],
) -> (Vec<ToolDefinition>, HashMap<String, (Handler, Duration)>) {
    let mut tool_defs = Vec::new();
    let mut external_map: HashMap<String, (Handler, Duration)> = HashMap::new();
    let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in matched {
        let timeout = Duration::from_secs(entry.manifest.options.timeout_secs);

        match &entry.manifest.handler {
            Handler::Builtin { tools } => {
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
            handler @ (Handler::Exec { tools, .. } | Handler::Http { tools, .. }) => {
                // Load external tool definitions from tools.json
                let external_defs = match loader::load_tool_definitions(&entry.dir).await {
                    Ok(defs) => defs,
                    Err(e) => {
                        tracing::warn!(
                            skill = %entry.manifest.name,
                            error = %e,
                            "failed to load tools.json"
                        );
                        continue;
                    }
                };

                for def in external_defs {
                    if tools.contains(&def.name) && !seen_names.contains(&def.name) {
                        seen_names.insert(def.name.clone());
                        external_map.insert(def.name.clone(), (handler.clone(), timeout));
                        tool_defs.push(def);
                    }
                }
            }
        }
    }

    (tool_defs, external_map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::{SkillManifest, SkillOptions, Triggers};
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

    #[tokio::test]
    async fn test_resolve_builtin_skills() {
        let tool_registry = tools::default_tools();
        let entry = make_builtin_entry("memory", &["store_fact", "search_memory"], true);
        let matched: Vec<&SkillEntry> = vec![&entry];

        let (defs, external) = resolve_matched_skills(&tool_registry, &matched).await;
        assert_eq!(defs.len(), 2);
        assert!(defs.iter().any(|d| d.name == "store_fact"));
        assert!(defs.iter().any(|d| d.name == "search_memory"));
        assert!(external.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_deduplicates() {
        let tool_registry = tools::default_tools();
        let entry1 = make_builtin_entry("memory", &["store_fact"], true);
        let entry2 = make_builtin_entry("memory2", &["store_fact"], true);
        let matched: Vec<&SkillEntry> = vec![&entry1, &entry2];

        let (defs, _) = resolve_matched_skills(&tool_registry, &matched).await;
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "store_fact");
    }
}
