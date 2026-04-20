use std::collections::{HashMap, VecDeque};

use super::index::SkillEntry;

/// Why a skill was included in the matched set.
///
/// Used by the `required_tools` enforcement gate to only enforce constraints
/// from skills that matched via keyword (not just `always_on`). See #265.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchReason {
    /// Matched because `always_on = true` with no keyword hit on this message.
    AlwaysOn,
    /// Matched because at least one keyword matched the user message
    /// (regardless of whether `always_on` is also true).
    Keyword,
    /// Pulled in as a transitive dependency of another matched skill.
    Dependency,
}

/// A skill entry annotated with the reason it was included in the matched set.
#[derive(Debug)]
pub struct MatchedSkill<'a> {
    pub entry: &'a SkillEntry,
    pub reason: MatchReason,
}

/// Match skills against a user message.
///
/// Returns all enabled `always_on` skills plus any enabled skill where at least
/// one keyword is a substring of the lowercased message. Then resolves the full
/// transitive dependency tree via BFS: if a matched skill declares
/// `dependencies = ["foo"]` and foo depends on `["bar"]`, all three are included
/// (if enabled and present). Disabled mid-chain skills break their sub-tree.
///
/// Each returned skill is annotated with a [`MatchReason`] indicating why it was
/// included. If a skill is both `always_on` and has a keyword hit, the reason is
/// `Keyword` (the more specific match wins). Dependencies are tagged `Dependency`.
pub fn match_skills<'a>(skills: &'a [SkillEntry], user_message: &str) -> Vec<MatchedSkill<'a>> {
    let message_lower = user_message.to_lowercase();

    // First pass: direct matches (always_on or keyword hit), tracking reason
    let mut matched_reasons: HashMap<usize, MatchReason> = HashMap::new();
    for (i, entry) in skills.iter().enumerate() {
        let keyword_hit = entry
            .keywords_lower
            .iter()
            .any(|kw| message_lower.contains(kw));
        if keyword_hit {
            // Keyword match takes precedence even if also always_on
            matched_reasons.insert(i, MatchReason::Keyword);
        } else if entry.manifest.skill.always_on {
            matched_reasons.insert(i, MatchReason::AlwaysOn);
        }
    }

    // Second pass: BFS transitive dependency resolution
    let mut queue: VecDeque<usize> = matched_reasons.keys().copied().collect();

    while let Some(idx) = queue.pop_front() {
        for dep_name in &skills[idx].manifest.skill.dependencies {
            if let Some(dep_idx) = skills
                .iter()
                .position(|e| e.manifest.skill.name.eq_ignore_ascii_case(dep_name))
            {
                // Disabled mid-chain dep breaks its sub-tree.
                // Don't overwrite an existing reason (a dep can also be a direct match).
                if skills[dep_idx].enabled && !matched_reasons.contains_key(&dep_idx) {
                    matched_reasons.insert(dep_idx, MatchReason::Dependency);
                    queue.push_back(dep_idx);
                }
            }
        }
    }

    // Collect in original order
    skills
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            matched_reasons
                .get(&i)
                .map(|&reason| MatchedSkill { entry, reason })
        })
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
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
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
            generated_model_prompts: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_always_on_included_regardless() {
        let skills = vec![make_entry("memory", &[], true)];
        let matched = match_skills(&skills, "hello there");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].entry.manifest.skill.name, "memory");
        assert_eq!(matched[0].reason, MatchReason::AlwaysOn);
    }

    #[test]
    fn test_keyword_match() {
        let skills = vec![make_entry("reminders", &["remind", "alarm"], false)];
        let matched = match_skills(&skills, "Please remind me tomorrow");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].entry.manifest.skill.name, "reminders");
        assert_eq!(matched[0].reason, MatchReason::Keyword);
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
        assert_eq!(matched[0].reason, MatchReason::Keyword);
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

    // Note: disabled skills are evicted from the registry by apply_overrides()
    // before match_skills() is ever called (#629, #630). No match-time disabled
    // filter exists — the registry contract guarantees all entries are enabled.

    // --- Match reason tests (#265) ---

    #[test]
    fn test_always_on_with_keyword_hit_is_keyword_reason() {
        // Self-dev pattern: always_on=true AND has keywords. When keyword matches,
        // reason should be Keyword (not AlwaysOn) so required_tools are enforced.
        let skills = vec![make_entry("self-dev", &["implement", "build"], true)];
        let matched = match_skills(&skills, "implement feature X");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].reason, MatchReason::Keyword);
    }

    #[test]
    fn test_always_on_with_keyword_no_hit_is_always_on_reason() {
        // Same skill, but message doesn't match any keyword
        let skills = vec![make_entry("self-dev", &["implement", "build"], true)];
        let matched = match_skills(&skills, "what time is it?");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].reason, MatchReason::AlwaysOn);
    }

    #[test]
    fn test_dependency_reason_tagged() {
        let skills = vec![
            make_entry_with_deps("self-dev", &[], true, &["tmux"]),
            make_entry("tmux", &["tmux"], false),
        ];
        let matched = match_skills(&skills, "hello");
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].reason, MatchReason::AlwaysOn);
        assert_eq!(matched[1].reason, MatchReason::Dependency);
    }

    #[test]
    fn test_dependency_that_also_keyword_matches_keeps_keyword() {
        // If a dep also matches via keyword, its reason should be Keyword (not Dependency)
        let skills = vec![
            make_entry_with_deps("self-dev", &[], true, &["tmux"]),
            make_entry("tmux", &["tmux"], false),
        ];
        let matched = match_skills(&skills, "use tmux please");
        assert_eq!(matched.len(), 2);
        // tmux matched directly via keyword, so it should be Keyword (not Dependency)
        assert_eq!(matched[1].entry.manifest.skill.name, "tmux");
        assert_eq!(matched[1].reason, MatchReason::Keyword);
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
        assert_eq!(matched[0].entry.manifest.skill.name, "self-dev");
        assert_eq!(matched[1].entry.manifest.skill.name, "tmux");
    }

    #[test]
    fn test_dependency_on_disabled_skill_skipped() {
        let mut tmux = make_entry("tmux", &["tmux"], false);
        tmux.enabled = false;
        let skills = vec![make_entry_with_deps("self-dev", &[], true, &["tmux"]), tmux];
        let matched = match_skills(&skills, "yes please");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].entry.manifest.skill.name, "self-dev");
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
        assert_eq!(matched[0].entry.manifest.skill.name, "self-dev");
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
        assert_eq!(matched[0].entry.manifest.skill.name, "skill-a");
        assert_eq!(matched[1].entry.manifest.skill.name, "skill-b");
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
            .map(|m| m.entry.manifest.skill.name.as_str())
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
            .map(|m| m.entry.manifest.skill.name.as_str())
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
            .map(|m| m.entry.manifest.skill.name.as_str())
            .collect();
        assert_eq!(names, vec!["skill-a"]);
    }

    // --- Keyword false-positive prevention tests (#576) ---

    #[test]
    fn test_no_keywords_skill_not_matched_by_pr_discussing_it() {
        // skill-review with empty keywords must NOT match when a PR body
        // merely discusses skill-review (the feature itself).
        let skills = vec![make_entry("skill-review", &[], false)];
        let matched = match_skills(
            &skills,
            "[GitHub] PR opened: senara-solutions/mika#576 — skill-review fires on PRs \
             that discuss skill-review (branch: feat/576/skill-review-fires)",
        );
        assert!(
            matched.is_empty(),
            "skill-review should not match on PR meta-discussion"
        );
    }

    #[test]
    fn test_no_keywords_skill_not_matched_by_review_skill_phrase() {
        // Even a message containing "review skill" should not trigger
        // skill-review when it has no keywords.
        let skills = vec![make_entry("skill-review", &[], false)];
        let matched = match_skills(&skills, "the review skill feature is broken");
        assert!(
            matched.is_empty(),
            "skill-review with empty keywords should never keyword-match"
        );
    }

    #[test]
    fn test_no_keywords_skill_not_matched_by_old_keywords() {
        // Phrases that would have matched the old keywords ("adapt skill",
        // "generate variant", etc.) must not trigger skill-review.
        let skills = vec![make_entry("skill-review", &[], false)];
        for phrase in &[
            "adapt skill for claude",
            "generate variant of qa-review",
            "tune prompt for self-dev",
            "skill variant needed",
        ] {
            let matched = match_skills(&skills, phrase);
            assert!(
                matched.is_empty(),
                "skill-review should not match on '{phrase}' with empty keywords"
            );
        }
    }

    #[test]
    fn test_no_keywords_skill_loaded_as_dependency() {
        // skill-review must still load when pulled in as a dependency
        // of another matched skill — dependency loading is unaffected.
        let skills = vec![
            make_entry_with_deps("qa-review", &["review"], true, &["skill-review"]),
            make_entry("skill-review", &[], false),
        ];
        let matched = match_skills(&skills, "review this PR");
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].entry.manifest.skill.name, "qa-review");
        assert_eq!(matched[0].reason, MatchReason::Keyword);
        assert_eq!(matched[1].entry.manifest.skill.name, "skill-review");
        assert_eq!(matched[1].reason, MatchReason::Dependency);
    }
}
