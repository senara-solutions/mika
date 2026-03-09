use super::index::SkillEntry;

/// Match skills against a user message.
///
/// Returns all enabled `always_on` skills plus any enabled skill where at least
/// one keyword is a substring of the lowercased message. Cheap and predictable —
/// Claude still decides which tools to actually call.
pub fn match_skills<'a>(skills: &'a [SkillEntry], user_message: &str) -> Vec<&'a SkillEntry> {
    let message_lower = user_message.to_lowercase();

    skills
        .iter()
        .filter(|entry| {
            entry.enabled
                && (entry.manifest.skill.always_on
                    || entry
                        .keywords_lower
                        .iter()
                        .any(|kw| message_lower.contains(kw)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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
                },
                triggers: Triggers {
                    keywords: keywords.iter().map(|s| s.to_string()).collect(),
                },
            },
            dir: PathBuf::from(format!("/skills/{name}")),
            keywords_lower: keywords.iter().map(|s| s.to_lowercase()).collect(),
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
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
}
