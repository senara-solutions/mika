use std::collections::{HashMap, VecDeque};

use regex::Regex;
use tracing::warn;

use super::index::SkillEntry;

/// Return `true` if `c` is a "word" character for the purpose of deciding
/// whether a word-boundary anchor (`\b`) makes sense next to a keyword edge.
///
/// This mirrors the ASCII notion of `\w = [A-Za-z0-9_]`. We only use it to
/// decide *whether to emit* a `\b` next to a keyword's edge — the regex engine
/// itself uses the Unicode-aware definition of `\b` (Rust `regex` crate
/// default) when it evaluates the resulting pattern against the message,
/// which is the safer choice against accented / non-ASCII neighbors.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Build a regex fragment for a single keyword that enforces word boundaries
/// only on edges where the keyword's edge character is itself a word char.
///
/// - `"gh"`           → `\bgh\b`             (both edges are word chars)
/// - `"issue #"`      → `\bissue #`          (trailing `#` is non-word; no `\b`)
/// - `"/mika-groom"`  → `/mika\-groom\b`     (leading `/` is non-word; no `\b`)
/// - `"[GitHub]"`     → `\[GitHub\]`         (both edges non-word; no `\b`)
///
/// Only emitting `\b` at word-char edges avoids the classic `\b#\b` bug where
/// a boundary can never satisfy (e.g. `issue #` followed by digits would not
/// match with an unconditional trailing `\b`).
fn keyword_to_pattern(kw: &str) -> String {
    let escaped = regex::escape(kw);
    let leading = kw.chars().next().map(is_word_char).unwrap_or(false);
    let trailing = kw.chars().last().map(is_word_char).unwrap_or(false);
    match (leading, trailing) {
        (true, true) => format!(r"\b{escaped}\b"),
        (true, false) => format!(r"\b{escaped}"),
        (false, true) => format!(r"{escaped}\b"),
        (false, false) => escaped,
    }
}

/// Build a single alternation regex over all keywords for a skill.
///
/// Returns `None` when the keyword list is empty or every keyword is empty
/// after filtering. On regex compile failure (which should be unreachable
/// once each keyword is `regex::escape`'d) we log a warning and return
/// `None`, effectively skipping keyword matching for that skill on this
/// call — safer than falling silently back to substring semantics.
fn build_matcher_regex(skill_name: &str, keywords_lower: &[String]) -> Option<Regex> {
    let alternation = keywords_lower
        .iter()
        .filter(|k| !k.is_empty())
        .map(|k| keyword_to_pattern(k))
        .collect::<Vec<_>>()
        .join("|");
    if alternation.is_empty() {
        return None;
    }
    match Regex::new(&alternation) {
        Ok(re) => Some(re),
        Err(e) => {
            warn!(
                skill = %skill_name,
                error = %e,
                pattern = %alternation,
                "keyword matcher regex failed to compile — skill will not fire on keyword this turn"
            );
            None
        }
    }
}

/// Why a skill was included in the matched set.
///
/// Used by the `required_tools` enforcement gate to only enforce constraints
/// from skills that matched via keyword (not just `always_on`). See #463.
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
/// one keyword matches the lowercased message on **word boundaries** (Rust
/// `regex` crate `\b` — Unicode-aware by default). Then resolves the full
/// transitive dependency tree via BFS: if a matched skill declares
/// `dependencies = ["foo"]` and foo depends on `["bar"]`, all three are included
/// (if enabled and present). Disabled mid-chain skills break their sub-tree.
///
/// **Word-boundary matching (mika#1878).** Historically the matcher used
/// `message_lower.contains(kw)` — a bare `"gh"` keyword collided on
/// `"thought"`/`"through"`, `"pr"` on `"approach"`/`"press"`. Every gated
/// skill had to defend against that class per-skill (see
/// `docs/architecture/skill-keyword-design-rules.md`). Word-boundary matching
/// retires the class structurally: keywords with word-char edges get `\b`
/// anchors; keywords with non-word edges (`"issue #"`, `"/mika-groom-ticket"`,
/// `"[GitHub]"`) are anchored only on their word-char side, so punctuation-
/// suffixed intent phrases still work.
///
/// Each returned skill is annotated with a [`MatchReason`] indicating why it was
/// included. If a skill is both `always_on` and has a keyword hit, the reason is
/// `Keyword` (the more specific match wins). Dependencies are tagged `Dependency`.
pub fn match_skills<'a>(skills: &'a [SkillEntry], user_message: &str) -> Vec<MatchedSkill<'a>> {
    let message_lower = user_message.to_lowercase();

    // First pass: direct matches (always_on or keyword hit), tracking reason
    let mut matched_reasons: HashMap<usize, MatchReason> = HashMap::new();
    for (i, entry) in skills.iter().enumerate() {
        let keyword_hit = build_matcher_regex(&entry.manifest.skill.name, &entry.keywords_lower)
            .map(|re| re.is_match(&message_lower))
            .unwrap_or(false);
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
        // Both `remember` and `remind` appear as whole tokens in the message
        // (they are adjacent words separated by "to"), so both fire.
        // Post-mika#1878: word-boundary matching means `remind` does NOT
        // match on `remember` as a substring — it matches on the standalone
        // `remind` token later in the sentence.
        let matched = match_skills(&skills, "remember to remind me");
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn test_always_on_plus_keyword() {
        let skills = vec![
            make_entry("memory", &["remember"], true),
            make_entry("reminders", &["remind"], false),
        ];
        // Message uses `remind` as a standalone token so the word-boundary
        // matcher (mika#1878) fires the reminders skill via keyword. `memory`
        // fires via always_on. The pre-mika#1878 wording of this test used
        // `"set a reminder"`, which relied on substring matching (`remind`
        // inside `reminder`); that's the exact false-positive class the
        // structural fix retires.
        let matched = match_skills(&skills, "please remind me later");
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

    // --- Match reason tests (#463) ---

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
    fn test_dev_pilot_and_dev_groom_loader_symmetry_on_ready_label_webhook() {
        // mika#1251 (follow-up to mika#1173): pins the exact path that broke.
        //
        // A keyword-less `[GitHub] Issue labeled ready` webhook message must
        // still pull dev-groom into the matched set via the always-on
        // self-dev → dev-groom dependency edge — exactly as it pulls in
        // dev-pilot. Without the edge, run_claude_pilot_groom never enters the
        // turn's tool map and the dispatcher falls through to "Unknown tool".
        //
        // Targets `match_skills` (the conversation-mode selector), NOT
        // `callback_safe_skills`: the webhook ready-label turn is
        // conversation-mode and routes through match_message → match_skills.
        let skills = vec![
            make_entry_with_deps(
                "self-dev",
                &["implement"],
                true,
                &["dev-pilot", "dev-groom"],
            ),
            make_entry("dev-pilot", &[], false),
            make_entry("dev-groom", &["groom", "groom ticket"], false),
        ];

        // Webhook-shaped message containing none of dev-groom's keywords.
        let matched = match_skills(
            &skills,
            "[GitHub] Issue labeled ready on senara-solutions/mika#9999 — fix the thing",
        );

        let dev_groom = matched
            .iter()
            .find(|m| m.entry.manifest.skill.name == "dev-groom")
            .expect("dev-groom must be matched via the self-dev dependency edge");
        assert_eq!(
            dev_groom.reason,
            MatchReason::Dependency,
            "dev-groom must arrive as a Dependency (no keyword hit on a webhook message)"
        );

        // dev-pilot is the sibling the mechanism already worked for — assert it
        // too, so the test distinguishes the bug from a passing baseline.
        let dev_pilot = matched
            .iter()
            .find(|m| m.entry.manifest.skill.name == "dev-pilot")
            .expect("dev-pilot must also be matched via the dependency edge");
        assert_eq!(dev_pilot.reason, MatchReason::Dependency);
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

    // --- Google Workspace skill activation regression tests (mika#152) ---

    #[test]
    fn test_google_workspace_always_on_activates_on_natural_language() {
        // mika#152: natural language prompts like "show my latest 5 emails" must
        // activate the google-workspace skill via always_on, even without keywords.
        let skills = vec![make_entry(
            "google-workspace",
            &[
                "google",
                "gmail",
                "google calendar",
                "google drive",
                "gdrive",
                "email",
                "emails",
                "inbox",
                "send email",
                "calendar",
                "meeting",
                "meetings",
                "schedule",
                "agenda",
                "free",
                "busy",
                "drive",
                "document",
                "documents",
                "spreadsheet",
                "slides",
                "triage",
                "gws",
                "workspace",
            ],
            true,
        )];

        // No keyword hit — matched via always_on
        let matched = match_skills(&skills, "what's on my plate today");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].entry.manifest.skill.name, "google-workspace");
        assert_eq!(matched[0].reason, MatchReason::AlwaysOn);

        // Keyword hit on "meeting" — matched as Keyword (takes precedence)
        let matched = match_skills(&skills, "what meetings do I have today");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].reason, MatchReason::Keyword);

        // Keyword hit on "triage" + "inbox"
        let matched = match_skills(&skills, "triage my inbox");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].reason, MatchReason::Keyword);

        // Keyword hit on "drive"
        let matched = match_skills(&skills, "search drive for quarterly report");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].reason, MatchReason::Keyword);
    }

    // --- gh-read-only required-tools-gate keyword-collision tests (mika#1650) ---

    /// The fixed `gh-read-only` keyword set, mirrored from
    /// `skills/bundled/gh-read-only/skill.toml`. Kept in sync with the manifest
    /// by hand — the design rule (no bare common-English bigrams under
    /// substring-only matching) is documented in
    /// `docs/architecture/skill-keyword-design-rules.md`.
    const GH_READ_ONLY_KEYWORDS: &[&str] = &[
        "github issue",
        "github pr",
        "github pull request",
        "pull request",
        "pull requests",
        "view issue",
        "view pr",
        "view pull request",
        "check issue",
        "check pr",
        "check pull request",
        "list issues",
        "list prs",
        "list pull requests",
        "open issues",
        "open prs",
        "open pull requests",
        "pr diff",
        "pr body",
        "issue body",
        "issue list",
        "fetch issue",
        "fetch pr",
        "merge pr",
        "close pr",
        "close issue",
        "issue #",
        "pr #",
    ];

    #[test]
    fn test_gh_read_only_does_not_fire_on_incidental_prose() {
        // AC3 (mika#1650): replay of mika-litha's offending turn class — prose
        // that merely *mentions* "issue" and "pr" with zero intent to fetch
        // GitHub data. The old bare-bigram keywords ("pr", "issue") matched as
        // substrings of "approach"/"issue", firing the required_tools gate and
        // forcing a spurious gh_read call. The fixed intent-phrase set must NOT
        // match — no keyword hit → no required_tools constraint → no engine retry.
        let skills = vec![make_entry("gh-read-only", GH_READ_ONLY_KEYWORDS, false)];
        for prose in &[
            "the issue with that pr approach is too risky",
            "I thought the proposal was appropriate, though the process is rough",
            "reissue the tissue report — high priority, brought up at the meeting",
        ] {
            let matched = match_skills(&skills, prose);
            assert!(
                matched.is_empty(),
                "gh-read-only must NOT match incidental prose: {prose:?}"
            );
        }
    }

    #[test]
    fn test_gh_read_only_fires_on_genuine_fetch_intent() {
        // AC4 (mika#1650): genuine fetch-intent user requests Litha (or any
        // agent with gh-read-only allowlisted) would legitimately send. The
        // intent-phrase set must still catch these — tightening the keyword
        // list trades false-positives-down for false-negatives-up, so verify
        // the kept set covers the real request patterns.
        let skills = vec![make_entry("gh-read-only", GH_READ_ONLY_KEYWORDS, false)];
        for request in &[
            "view issue #123",
            "list open prs",
            "merge pr 1678",
            "check pr 1644",
            "show me the pr diff for that branch",
            "what's in the github issue tracker today",
        ] {
            let matched = match_skills(&skills, request);
            assert_eq!(
                matched.len(),
                1,
                "gh-read-only MUST match genuine fetch-intent: {request:?}"
            );
            assert_eq!(matched[0].reason, MatchReason::Keyword);
        }
    }

    // --- Word-boundary matcher tests (mika#1878) ---

    #[test]
    fn test_word_boundary_bare_bigram_does_not_collide_on_prose() {
        // Before mika#1878, bare `"gh"` as a keyword matched anywhere it
        // appeared as a substring — including inside `"thought"`, `"through"`,
        // `"eight"`, `"high"`, `"light"`, `"right"`. This is the load-bearing
        // collision class the design-rule doc calls out and mika#1650 had to
        // paper over per-skill by dropping the bare bigram entirely.
        //
        // With word-boundary matching, `\bgh\b` will never anchor inside any of
        // those tokens — `g`/`h` are word chars adjacent to word chars on both
        // sides, so no `\b` is satisfied.
        let skills = vec![make_entry("gh-skill", &["gh"], false)];
        for prose in &[
            "I thought about it",
            "we went through the plan",
            "eight items remain",
            "high priority stuff",
            "let it shine a light on the bug",
            "that's the right approach",
            "although it may take time",
            "neighbor concerns",
        ] {
            let matched = match_skills(&skills, prose);
            assert!(
                matched.is_empty(),
                "bare `gh` keyword must NOT collide on incidental prose: {prose:?}"
            );
        }
    }

    #[test]
    fn test_word_boundary_bare_bigram_still_fires_on_standalone_token() {
        // Corollary of the above: a bare `"gh"` MUST still fire when the user
        // actually types `gh` as a standalone token (with normal English
        // whitespace / punctuation neighbors). Word boundaries are anchored on
        // both edges here because `g` and `h` are word chars — spaces,
        // commas, question marks, and start/end of message all count as
        // non-word neighbors and satisfy the anchor.
        let skills = vec![make_entry("gh-skill", &["gh"], false)];
        for msg in &[
            "can you gh auth for me?",
            "gh",
            "please run gh",
            "run gh, then check the output",
            "run gh; then check",
            "use 'gh' to view the PR",
        ] {
            let matched = match_skills(&skills, msg);
            assert_eq!(
                matched.len(),
                1,
                "bare `gh` keyword MUST fire on standalone token: {msg:?}"
            );
            assert_eq!(matched[0].reason, MatchReason::Keyword);
        }
    }

    #[test]
    fn test_word_boundary_bare_pr_does_not_collide_on_prose() {
        // Same class as `gh`, exercised on the other founding-incident bigram:
        // `"pr"` used to false-positive on `"approach"`, `"appropriate"`,
        // `"press"`, `"process"`, `"problem"`, `"project"`, `"provide"`, etc.
        let skills = vec![make_entry("pr-skill", &["pr"], false)];
        for prose in &[
            "this approach is too risky",
            "the proposal was appropriate",
            "press the button",
            "the process is rough",
            "there's a problem with the plan",
            "this project is on track",
            "please provide feedback",
            "prevent the regression",
            "properly configured",
        ] {
            let matched = match_skills(&skills, prose);
            assert!(
                matched.is_empty(),
                "bare `pr` keyword must NOT collide on incidental prose: {prose:?}"
            );
        }
    }

    #[test]
    fn test_word_boundary_multiword_keyword_requires_adjacent_tokens() {
        // A multi-word keyword `"view issue"` must match only when the tokens
        // appear adjacent in the message (whitespace-separated). It must NOT
        // fire when the same tokens appear scattered elsewhere.
        let skills = vec![make_entry("gh-read", &["view issue"], false)];

        // Positive: adjacent tokens, various neighbors.
        for msg in &[
            "view issue #123",
            "please view issue 42",
            "view issue, then close it",
            "if you view issue 5 you'll see",
        ] {
            let matched = match_skills(&skills, msg);
            assert_eq!(
                matched.len(),
                1,
                "multi-word keyword MUST fire on adjacent tokens: {msg:?}"
            );
        }

        // Negative: tokens present but not adjacent, or embedded in larger words.
        for msg in &[
            "the view is nice, and there's an issue",
            "reviewing issues in the tracker",
            "overview: no issue reported",
            "preview the issue tomorrow",
        ] {
            let matched = match_skills(&skills, msg);
            assert!(
                matched.is_empty(),
                "multi-word keyword must NOT fire on non-adjacent or embedded tokens: {msg:?}"
            );
        }
    }

    #[test]
    fn test_word_boundary_punctuation_suffixed_keyword_still_fires() {
        // Keywords like `"issue #"` and `"pr #"` end in a non-word character.
        // The old substring matcher happily anchored them; the word-boundary
        // matcher must still let them fire — the fix is to skip the trailing
        // `\b` when the keyword's trailing char is non-word (`#`).
        let skills = vec![make_entry("gh-read", &["issue #", "pr #"], false)];
        for msg in &[
            "close issue #123",
            "reference issue #4 tomorrow",
            "merge pr #1878",
            "the pr #1899 landed",
        ] {
            let matched = match_skills(&skills, msg);
            assert_eq!(
                matched.len(),
                1,
                "punctuation-suffixed intent phrase MUST still fire: {msg:?}"
            );
        }
        // Negative: the leading token must still be word-anchored so
        // `"pr #"` doesn't collide inside `"appropriate #tag"`.
        for prose in &["an appropriate #tag would help", "reissue #123 next week"] {
            let matched = match_skills(&skills, prose);
            assert!(
                matched.is_empty(),
                "punctuation-suffixed keyword must still respect leading boundary: {prose:?}"
            );
        }
    }

    #[test]
    fn test_word_boundary_slash_prefixed_keyword_still_fires() {
        // Slash-command keywords like `"/mika-groom-ticket"` start with a
        // non-word char. The matcher must not emit a leading `\b` (the
        // regex-`\b` between two non-word chars — start of a word char
        // sequence — is fine, but between two non-word chars is UB in intent).
        // In practice: `\b/…` would demand the preceding char be a word char,
        // which is the opposite of what we want. Skipping the leading `\b`
        // gives the correct semantic: match `/mika-groom-ticket` anywhere
        // provided its trailing edge respects the word boundary.
        let skills = vec![make_entry("dev-groom", &["/mika-groom-ticket"], false)];
        for msg in &[
            "/mika-groom-ticket #123",
            "please /mika-groom-ticket the plan",
            "run /mika-groom-ticket",
        ] {
            let matched = match_skills(&skills, msg);
            assert_eq!(
                matched.len(),
                1,
                "slash-prefixed keyword MUST still fire: {msg:?}"
            );
        }
    }

    #[test]
    fn test_word_boundary_hyphenated_keyword_matches_on_outer_edges() {
        // `groom-ticket` has word chars on both outer edges; the internal `-`
        // is non-word but that doesn't affect the outer anchors. `\b` on both
        // edges rejects embedded matches inside larger identifier-shaped
        // tokens like `mygroom-ticketxyz`.
        let skills = vec![make_entry(
            "mika-arch-groom-ticket",
            &["groom-ticket"],
            false,
        )];

        // Positive.
        assert_eq!(
            match_skills(&skills, "please review groom-ticket now").len(),
            1
        );
        assert_eq!(match_skills(&skills, "groom-ticket").len(), 1);

        // Negative: embedded inside larger identifier.
        assert!(
            match_skills(&skills, "mygroom-ticket run").is_empty(),
            "hyphenated keyword must not match inside a larger identifier prefix"
        );
        assert!(
            match_skills(&skills, "groom-ticketxyz").is_empty(),
            "hyphenated keyword must not match inside a larger identifier suffix"
        );
    }

    #[test]
    fn test_word_boundary_case_insensitivity_preserved() {
        // Boundary matching still runs against the lowercased message with
        // lowercased keywords, so case-insensitivity from mika#0 is preserved.
        let skills = vec![make_entry("memory", &["remember"], false)];
        let matched = match_skills(&skills, "REMEMBER this");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].reason, MatchReason::Keyword);

        // And still respects the boundary — `REMEMBER` inside `REMEMBERED`
        // is (correctly) NOT a match under boundary semantics.
        // (See test_multiple_matches: `remind` in `remember` doesn't match.)
        let matched = match_skills(&skills, "REMEMBERED it");
        assert!(
            matched.is_empty(),
            "case-insensitivity must NOT bypass word-boundary anchoring"
        );
    }

    #[test]
    fn test_word_boundary_empty_keyword_string_is_skipped() {
        // An empty string inside the keyword list must not create an
        // alternation entry that matches every message. This is a
        // defense-in-depth check against future manifest bugs.
        let skills = vec![make_entry("s", &["", "gh"], false)];

        // Empty keyword must not fire on unrelated prose.
        assert!(
            match_skills(&skills, "unrelated text").is_empty(),
            "empty keyword must not match any message"
        );

        // Non-empty keyword still fires normally.
        let matched = match_skills(&skills, "run gh please");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].reason, MatchReason::Keyword);
    }
}
