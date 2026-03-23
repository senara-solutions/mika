use std::collections::{HashSet, VecDeque};

use super::index::SkillEntry;

/// Match skills against a user message.
///
/// Returns all enabled `always_on` skills plus any enabled skill where at least
/// one keyword is a substring of the lowercased message. Then resolves the full
/// transitive dependency tree via BFS: if a matched skill declares
/// `dependencies = ["foo"]` and foo depends on `["bar"]`, all three are included
/// (if enabled and present). Disabled mid-chain skills break their sub-tree.
pub fn match_skills<'a>(skills: &'a [SkillEntry], user_message: &str) -> Vec<&'a SkillEntry> {
    let message_lower = user_message.to_lowercase();

    // First pass: direct matches (always_on or keyword hit)
    let mut matched_indices: HashSet<usize> = HashSet::new();
    for (i, entry) in skills.iter().enumerate() {
        if entry.enabled
            && (entry.manifest.skill.always_on
                || entry
                    .keywords_lower
                    .iter()
                    .any(|kw| message_lower.contains(kw)))
        {
            matched_indices.insert(i);
        }
    }

    // Second pass: BFS transitive dependency resolution
    let mut queue: VecDeque<usize> = matched_indices.iter().copied().collect();

    while let Some(idx) = queue.pop_front() {
        for dep_name in &skills[idx].manifest.skill.dependencies {
            if let Some(dep_idx) = skills
                .iter()
                .position(|e| e.manifest.skill.name.eq_ignore_ascii_case(dep_name))
            {
                // Disabled mid-chain dep breaks its sub-tree
                if skills[dep_idx].enabled && matched_indices.insert(dep_idx) {
                    queue.push_back(dep_idx);
                }
            }
        }
    }

    // Collect in original order
    skills
        .iter()
        .enumerate()
        .filter(|(i, _)| matched_indices.contains(i))
        .map(|(_, entry)| entry)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::{SkillInfo, SkillManifest, Triggers};
    use std::path::PathBuf;

    fn make_entry(name: &str, keywords: &[&str], always_on: bool) -> SkillEntry {
        make_entry_with_deps(name, keywords, always_on, &[])
    }

    fn make_entry_with_deps(
        name: &str,
        keywords: &[&str],
        always_on: bool,
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
                triggers: Triggers {
                    keywords: keywords.iter().map(|s| s.to_string()).collect(),
                },
                llm: Default::default(),
            },
            dir: PathBuf::from(format!("/skills/{name}")),
            keywords_lower: keywords.iter().map(|s| s.to_lowercase()).collect(),
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: std::collections::HashMap::new(),
            model_prompts: std::collections::HashMap::new(),
            model_overrides: std::collections::HashMap::new(),
            llm: Default::default(),
        }
    }

    fn make_disabled_entry(name: &str, keywords: &[&str], always_on: bool) -> SkillEntry {
        let mut entry = make_entry(name, keywords, always_on);
        entry.enabled = false;
        entry
    }

    #[test]
    fn test_always_on_included_regardless() {
        let skills = vec![make_entry("memory", &[], true)];
        let matched = match_skills(&skills, "hello there");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].manifest.skill.name, "memory");
    }

    #[test]
    fn test_keyword_match() {
        let skills = vec![make_entry("reminders", &["remind", "alarm"], false)];
        let matched = match_skills(&skills, "Please remind me tomorrow");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].manifest.skill.name, "reminders");
    }

    #[test]
    fn test_no_match() {
        let skills = vec![make_entry("reminders", &["remind", "alarm"], false)];
        let matched = match_skills(&skills, "What's the weather like?");
        assert!(matched.is_empty());
    }

    #[test]
    fn test_case_insensitive() {
        let skills = vec![make_entry("memory", &["remember"], false)];
        let matched = match_skills(&skills, "REMEMBER this");
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn test_multiple_matches() {
        let skills = vec![
            make_entry("memory", &["remember"], false),
            make_entry("reminders", &["remind"], false),
            make_entry("other", &["unrelated"], false),
        ];
        // "remind" is a substring of "remember" so both match
        let matched = match_skills(&skills, "remember to remind me");
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn test_always_on_plus_keyword() {
        let skills = vec![
            make_entry("memory", &["remember"], true),
            make_entry("reminders", &["remind"], false),
        ];
        let matched = match_skills(&skills, "set a reminder");
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn test_empty_skills() {
        let matched = match_skills(&[], "hello");
        assert!(matched.is_empty());
    }

    #[test]
    fn test_disabled_skills_excluded() {
        let skills = vec![
            make_entry("enabled", &["search"], false),
            make_disabled_entry("disabled", &["search"], false),
        ];
        let matched = match_skills(&skills, "search for something");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].manifest.skill.name, "enabled");
    }

    #[test]
    fn test_disabled_always_on_excluded() {
        let skills = vec![
            make_entry("enabled", &[], true),
            make_disabled_entry("disabled", &[], true),
        ];
        let matched = match_skills(&skills, "hello");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].manifest.skill.name, "enabled");
    }

    // --- Dependency resolution tests ---

    #[test]
    fn test_dependency_pulls_in_dependent_skill() {
        let skills = vec![
            make_entry_with_deps("self-dev", &[], true, &["tmux"]),
            make_entry("tmux", &["tmux"], false),
        ];
        // "yes please" has no tmux keyword, but self-dev depends on tmux
        let matched = match_skills(&skills, "yes please");
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].manifest.skill.name, "self-dev");
        assert_eq!(matched[1].manifest.skill.name, "tmux");
    }

    #[test]
    fn test_dependency_on_disabled_skill_skipped() {
        let mut tmux = make_entry("tmux", &["tmux"], false);
        tmux.enabled = false;
        let skills = vec![make_entry_with_deps("self-dev", &[], true, &["tmux"]), tmux];
        let matched = match_skills(&skills, "yes please");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].manifest.skill.name, "self-dev");
    }

    #[test]
    fn test_dependency_on_nonexistent_skill_silently_skipped() {
        let skills = vec![make_entry_with_deps(
            "self-dev",
            &[],
            true,
            &["nonexistent"],
        )];
        let matched = match_skills(&skills, "yes please");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].manifest.skill.name, "self-dev");
    }

    #[test]
    fn test_circular_dependencies_no_infinite_loop() {
        let skills = vec![
            make_entry_with_deps("skill-a", &[], true, &["skill-b"]),
            make_entry_with_deps("skill-b", &["something"], false, &["skill-a"]),
        ];
        // skill-a is always_on, depends on skill-b; skill-b depends on skill-a
        let matched = match_skills(&skills, "yes please");
        // skill-a matched directly, skill-b pulled in via dependency
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].manifest.skill.name, "skill-a");
        assert_eq!(matched[1].manifest.skill.name, "skill-b");
    }

    #[test]
    fn test_no_duplicates_from_repeated_dependencies() {
        let skills = vec![
            make_entry_with_deps("skill-a", &[], true, &["shared"]),
            make_entry_with_deps("skill-b", &[], true, &["shared"]),
            make_entry("shared", &[], false),
        ];
        let matched = match_skills(&skills, "hello");
        assert_eq!(matched.len(), 3);
        // Each skill appears exactly once
        let names: Vec<&str> = matched
            .iter()
            .map(|e| e.manifest.skill.name.as_str())
            .collect();
        assert_eq!(names, vec!["skill-a", "skill-b", "shared"]);
    }

    #[test]
    fn test_dependency_case_insensitive_lookup() {
        let skills = vec![
            make_entry_with_deps("self-dev", &[], true, &["Tmux"]),
            make_entry("tmux", &["tmux"], false),
        ];
        let matched = match_skills(&skills, "yes please");
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn test_transitive_dependencies() {
        // A depends on B, B depends on C. All three should match (full transitive resolution).
        let skills = vec![
            make_entry_with_deps("skill-a", &[], true, &["skill-b"]),
            make_entry_with_deps("skill-b", &[], false, &["skill-c"]),
            make_entry("skill-c", &[], false),
        ];
        let matched = match_skills(&skills, "hello");
        assert_eq!(matched.len(), 3);
        let names: Vec<&str> = matched
            .iter()
            .map(|e| e.manifest.skill.name.as_str())
            .collect();
        assert_eq!(names, vec!["skill-a", "skill-b", "skill-c"]);
    }

    #[test]
    fn test_disabled_mid_chain_breaks_subtree() {
        // A depends on B, B depends on C. B is disabled → C is NOT loaded.
        let mut skill_b = make_entry_with_deps("skill-b", &[], false, &["skill-c"]);
        skill_b.enabled = false;
        let skills = vec![
            make_entry_with_deps("skill-a", &[], true, &["skill-b"]),
            skill_b,
            make_entry("skill-c", &[], false),
        ];
        let matched = match_skills(&skills, "hello");
        assert_eq!(matched.len(), 1);
        let names: Vec<&str> = matched
            .iter()
            .map(|e| e.manifest.skill.name.as_str())
            .collect();
        assert_eq!(names, vec!["skill-a"]);
    }
}
