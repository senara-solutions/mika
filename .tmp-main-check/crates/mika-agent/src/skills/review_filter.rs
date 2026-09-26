use super::matcher::{MatchReason, MatchedSkill};

/// Name of the skill-review skill. Used to detect when a review turn is active.
const SKILL_REVIEW_NAME: &str = "skill-review";

/// Identify matched skills that should be excluded because they are the target
/// of a `skill-review` turn.
///
/// When `skill-review` is keyword-matched, any *other* keyword-matched skill
/// whose name appears in the user message (case-insensitive) is treated as the
/// review target and excluded from the matched set. This prevents the reviewed
/// skill's prompt from contaminating the review context.
///
/// Returns indices in **descending** order so the caller can safely remove them
/// via `matched.remove(idx)` without invalidating earlier indices.
///
/// Only `Keyword`-matched skills are candidates for exclusion — `AlwaysOn` and
/// `Dependency` skills are never excluded by this mechanism.
/// Exclude review-target skills from the matched set in place.
///
/// Convenience wrapper around [`exclude_review_targets`] that removes the
/// identified skills from `matched` and emits a debug log for each removal.
/// Call this once at the call site instead of duplicating the removal loop.
pub fn apply_review_filter(matched: &mut Vec<MatchedSkill<'_>>, user_message: &str) {
    let exclude = exclude_review_targets(matched, user_message);
    for &idx in &exclude {
        tracing::debug!(
            skill = matched[idx].entry.manifest.skill.name,
            "excluding skill from review turn"
        );
        matched.remove(idx);
    }
}

/// Identify matched skills that should be excluded because they are the target
/// of a `skill-review` turn (internal — prefer [`apply_review_filter`]).
fn exclude_review_targets(matched: &[MatchedSkill<'_>], user_message: &str) -> Vec<usize> {
    // Check if skill-review is keyword-matched in this turn.
    let has_skill_review_keyword = matched.iter().any(|m| {
        m.reason == MatchReason::Keyword
            && m.entry
                .manifest
                .skill
                .name
                .eq_ignore_ascii_case(SKILL_REVIEW_NAME)
    });

    if !has_skill_review_keyword {
        return Vec::new();
    }

    let message_lower = user_message.to_lowercase();

    let mut exclude_indices: Vec<usize> = matched
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            // Only exclude Keyword-matched skills (not AlwaysOn or Dependency).
            m.reason == MatchReason::Keyword
                // Never exclude skill-review itself.
                && !m
                    .entry
                    .manifest
                    .skill
                    .name
                    .eq_ignore_ascii_case(SKILL_REVIEW_NAME)
                // The skill's name must appear in the user message (case-insensitive).
                && message_lower.contains(&m.entry.manifest.skill.name.to_lowercase())
        })
        .map(|(i, _)| i)
        .collect();

    // Sort descending for safe removal via matched.remove(idx).
    exclude_indices.sort_unstable_by(|a, b| b.cmp(a));
    exclude_indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::index::SkillEntry;
    use crate::skills::manifest::{SkillInfo, SkillManifest, Triggers};
    use std::path::PathBuf;

    fn make_entry(name: &str, keywords: &[&str], always_on: bool) -> SkillEntry {
        SkillEntry {
            manifest: SkillManifest {
                skill: SkillInfo {
                    name: name.to_string(),
                    description: format!("{name} skill"),
                    version: String::new(),
                    always_on,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                    data_grade: Default::default(),
                },
                triggers: Triggers {
                    keywords: keywords.iter().map(|s| s.to_string()).collect(),
                },
                llm: Default::default(),
                constraints: Default::default(),
                output: Default::default(),
                context: std::collections::HashMap::new(),
                variants: Default::default(),
            },
            dir: PathBuf::from(format!("/skills/{name}")),
            keywords_lower: keywords.iter().map(|s| s.to_lowercase()).collect(),
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: std::collections::HashMap::new(),
            prompt_sources: SkillEntry::empty_prompt_sources(),
            model_overrides: std::collections::HashMap::new(),
        }
    }

    fn matched_keyword(entry: &SkillEntry) -> MatchedSkill<'_> {
        MatchedSkill {
            entry,
            reason: MatchReason::Keyword,
        }
    }

    fn matched_always_on(entry: &SkillEntry) -> MatchedSkill<'_> {
        MatchedSkill {
            entry,
            reason: MatchReason::AlwaysOn,
        }
    }

    fn matched_dependency(entry: &SkillEntry) -> MatchedSkill<'_> {
        MatchedSkill {
            entry,
            reason: MatchReason::Dependency,
        }
    }

    #[test]
    fn test_excludes_review_target() {
        let review = make_entry("skill-review", &["review skill"], false);
        let self_dev = make_entry("self-dev", &["self-dev", "dev"], false);
        let matched = vec![matched_keyword(&review), matched_keyword(&self_dev)];

        let excluded = exclude_review_targets(&matched, "review skill self-dev");
        assert_eq!(excluded, vec![1]);
    }

    #[test]
    fn test_excludes_web_search_target() {
        let review = make_entry("skill-review", &["review skill"], false);
        let web_search = make_entry("web-search", &["web search", "search"], false);
        let matched = vec![matched_keyword(&review), matched_keyword(&web_search)];

        let excluded = exclude_review_targets(&matched, "review skill web-search");
        assert_eq!(excluded, vec![1]);
    }

    #[test]
    fn test_no_other_keyword_matched_skills() {
        let review = make_entry("skill-review", &["review skill"], false);
        let matched = vec![matched_keyword(&review)];

        let excluded = exclude_review_targets(&matched, "review skill self-dev");
        assert!(excluded.is_empty());
    }

    #[test]
    fn test_skill_review_matched_via_always_on_no_exclusion() {
        let review = make_entry("skill-review", &["review skill"], true);
        let self_dev = make_entry("self-dev", &["self-dev"], false);
        let matched = vec![matched_always_on(&review), matched_keyword(&self_dev)];

        // skill-review is AlwaysOn, not Keyword — no exclusion triggered.
        let excluded = exclude_review_targets(&matched, "review skill self-dev");
        assert!(excluded.is_empty());
    }

    #[test]
    fn test_target_matched_via_always_on_not_excluded() {
        let review = make_entry("skill-review", &["review skill"], false);
        let self_dev = make_entry("self-dev", &["self-dev"], true);
        let matched = vec![matched_keyword(&review), matched_always_on(&self_dev)];

        // self-dev is AlwaysOn — should not be excluded.
        let excluded = exclude_review_targets(&matched, "review skill self-dev");
        assert!(excluded.is_empty());
    }

    #[test]
    fn test_target_matched_via_dependency_not_excluded() {
        let review = make_entry("skill-review", &["review skill"], false);
        let self_dev = make_entry("self-dev", &["self-dev"], false);
        let matched = vec![matched_keyword(&review), matched_dependency(&self_dev)];

        // self-dev is a Dependency — should not be excluded.
        let excluded = exclude_review_targets(&matched, "review skill self-dev");
        assert!(excluded.is_empty());
    }

    #[test]
    fn test_multiple_keyword_only_matching_name_excluded() {
        let review = make_entry("skill-review", &["review skill"], false);
        let self_dev = make_entry("self-dev", &["self-dev"], false);
        let reminders = make_entry("reminders", &["review"], false);
        let matched = vec![
            matched_keyword(&review),
            matched_keyword(&self_dev),
            matched_keyword(&reminders),
        ];

        // "reminders" does not appear in the message, only "self-dev" does.
        let excluded = exclude_review_targets(&matched, "review skill self-dev");
        assert_eq!(excluded, vec![1]);
    }

    #[test]
    fn test_skill_review_not_in_matched_set() {
        let self_dev = make_entry("self-dev", &["self-dev"], false);
        let matched = vec![matched_keyword(&self_dev)];

        let excluded = exclude_review_targets(&matched, "review skill self-dev");
        assert!(excluded.is_empty());
    }

    #[test]
    fn test_batch_mode_no_exclusion() {
        let review = make_entry("skill-review", &["review skill"], false);
        let self_dev = make_entry("self-dev", &["self-dev"], false);
        let matched = vec![matched_keyword(&review), matched_keyword(&self_dev)];

        // Batch mode: "review skill *" — asterisk is not a skill name.
        let excluded = exclude_review_targets(&matched, "review skill *");
        assert!(excluded.is_empty());
    }

    #[test]
    fn test_case_insensitive_skill_name_match() {
        let review = make_entry("skill-review", &["review skill"], false);
        let self_dev = make_entry("Self-Dev", &["self-dev"], false);
        let matched = vec![matched_keyword(&review), matched_keyword(&self_dev)];

        let excluded = exclude_review_targets(&matched, "review skill self-dev");
        assert_eq!(excluded, vec![1]);
    }

    #[test]
    fn test_descending_index_order_for_safe_removal() {
        let review = make_entry("skill-review", &["review skill"], false);
        let skill_a = make_entry("skill-a", &["skill-a"], false);
        let skill_b = make_entry("skill-b", &["skill-b"], false);
        let matched = vec![
            matched_keyword(&review),
            matched_keyword(&skill_a),
            matched_keyword(&skill_b),
        ];

        // Both skill names appear in the message.
        let excluded =
            exclude_review_targets(&matched, "review skill skill-a and skill-b together");
        assert_eq!(excluded, vec![2, 1]); // Descending order.
    }
}
