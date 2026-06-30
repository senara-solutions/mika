//! Fabrication-guard predicates and grounding-rule enforcement helpers.
//!
//! Owns the pure predicate functions that the agent loop's guard-dispatch
//! logic consults at EndTurn. The enforcement timing (reject-and-reprompt
//! machinery) stays in `crate::agent`.

use std::collections::HashSet;
use tracing::debug;

use crate::tool_execution::ToolCallSummary;

// ---------------------------------------------------------------------------
// #308 — Fabricated action-claim detection
// ---------------------------------------------------------------------------

/// Regex matching GitHub resource URLs that look like created resources:
/// issue comments, review comments, PR review IDs, issues, and PRs.
static GITHUB_RESOURCE_URL_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    // Use [^\s>\]] to allow `)` inside URLs — LLMs often emit markdown links like
    // [comment](https://github.com/org/repo/pull/1#issuecomment-99) where `)` is
    // part of the surrounding syntax but the URL itself contains the resource anchor.
    regex::Regex::new(
            r"https?://github\.com/[^\s>\]]+(?:#issuecomment-\d+|#discussion_r\d+|#pullrequestreview-\d+|/(?:issues|pull)/\d+)",
        )
        .expect("github resource url regex must compile")
});

/// Regex matching action-claim verbs that indicate the agent is claiming
/// to have performed an action (posting, commenting, creating, etc.).
static ACTION_CLAIM_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(posted|commented|created|submitted|opened|reviewed|published|added|wrote|replied|approved|filed|raised|left a (?:comment|review))\b")
        .expect("action claim regex must compile")
});

/// Detects whether assistant text claims to have performed an action with a
/// fabricated GitHub URL. Returns `(verb, url)` for logging, or `None`.
///
/// Only detects fabrication when the agent made zero tool calls — if any tool
/// was called, the URL may have come from a tool result.
pub(crate) fn detect_fabricated_action_claim(text: &str) -> Option<(&str, &str)> {
    // Fast path: skip regex if no likely substring present.
    if !text.contains("github.com/") {
        return None;
    }
    let url_match = GITHUB_RESOURCE_URL_RE.find(text)?;
    let verb_match = ACTION_CLAIM_RE.find(text)?;
    Some((verb_match.as_str(), url_match.as_str()))
}

// ---------------------------------------------------------------------------
// #716 — Callback state-claim detection
// ---------------------------------------------------------------------------

/// Regex matching callback-turn state claims about downstream GitHub state
/// (PR status, issue close reason, branch existence) that are commonly
/// fabricated when the LLM rationalizes callback error signals. See #716.
static CALLBACK_STATE_CLAIM_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(
    || {
        regex::Regex::new(
        r"(?i)\b(no\s+PR|without\s+PR|manually\s+closed|closed\s+without|no\s+commits?|handler\s+crashed|no\s+branch)\b",
    )
    .expect("callback state claim regex must compile")
    },
);

/// Detects when callback-turn assistant text claims downstream GitHub state
/// (PR status, issue close reason) without verification. Returns the matched
/// claim fragment if found.
///
/// Only meaningful when checked against `tools_called` — the guard fires
/// when this returns `Some` AND neither `run_gh` nor `check_task` was called.
/// See #716.
pub(crate) fn detect_unverified_callback_state_claim(text: &str) -> Option<&str> {
    // Fast path: skip regex if no likely substrings present.
    let lower = text.to_lowercase();
    let has_candidate = lower.contains("no pr")
        || lower.contains("without pr")
        || lower.contains("manually closed")
        || lower.contains("closed without")
        || lower.contains("no commit")
        || lower.contains("handler crashed")
        || lower.contains("no branch");

    if !has_candidate {
        return None;
    }

    CALLBACK_STATE_CLAIM_RE.find(text).map(|m| m.as_str())
}

// ---------------------------------------------------------------------------
// #862 — Asserted-unavailability guard
// ---------------------------------------------------------------------------

/// Label used for `intent_guard_retries` tracking of the asserted-unavailability
/// guard (#862). Inline guard (not in `INTENT_GUARDS` const array) because it
/// checks *assistant* text, not user-input text, and needs the enabled-tool-set
/// snapshot + dynamic correction message.
pub(crate) const ASSERTED_UNAVAILABILITY_LABEL: &str = "asserted_unavailability";

/// Regex patterns from the gate-evasion compound doc (Rule 2).
/// Each uses a named capture group `(?P<tool>...)` so extraction is
/// `captures["tool"]` uniformly (F2 resolution).
///
/// P1-P5: original five patterns (#862, #894).
/// P6-P9: extension shapes (#1177) — descriptor-word absorption (P6),
///        antonym `unavailable` (P7), modal/periphrastic negation (P8),
///        inverted modal `unable to` (P9).
static ASSERTED_UNAVAILABILITY_PATTERNS: std::sync::LazyLock<Vec<regex::Regex>> =
    std::sync::LazyLock::new(|| {
        vec![
            // P1: "I don't have access to X"
            regex::Regex::new(
                r"(?i)\bi (?:don'?t|do not) have access to (?P<tool>[a-z_][a-z0-9_]*)",
            )
            .expect("asserted_unavailability pattern 1"),
            // P2: "X [is] [adverb] not available/callable/accessible"
            regex::Regex::new(
                r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) (?:is )?(?:\w+ly )?not (?:available|callable|accessible)",
            )
            .expect("asserted_unavailability pattern 2"),
            // P3: "X isn't [adverb] available/callable/accessible"
            regex::Regex::new(
                r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) isn'?t (?:\w+ly )?(?:available|callable|accessible)",
            )
            .expect("asserted_unavailability pattern 3"),
            // P4: "X [is] skill-scoped"
            regex::Regex::new(r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) (?:is )?skill-scoped")
                .expect("asserted_unavailability pattern 4"),
            // P5: "cannot call [the] X"
            regex::Regex::new(r"(?i)\bcannot call (?:the )?(?P<tool>[a-z_][a-z0-9_]*)")
                .expect("asserted_unavailability pattern 5"),
            // P6 (#1177 Shape A): "the X tool/function/feature/skill/handler is not available"
            // Descriptor-word absorption — captures the tool name before the descriptor noun.
            regex::Regex::new(
                r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) (?:tool|function|feature|skill|handler) (?:is )?(?:\w+ly )?not (?:available|callable|accessible)",
            )
            .expect("asserted_unavailability pattern 6"),
            // P7 (#1177 Shape B): "X [is] [adverb] unavailable"
            regex::Regex::new(
                r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) (?:is )?(?:\w+ly )?unavailable\b",
            )
            .expect("asserted_unavailability pattern 7"),
            // P8 (#1177 Shape C): "X may/could/cannot/can't/won't/wouldn't [not] be called/invoked/used/accessed/reached"
            // Also covers "X doesn't/doesn't appear/seem to be callable/..."
            regex::Regex::new(
                r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) (?:doesn'?t (?:appear|seem) to (?:be )?|(?:may|could|cannot|can'?t|won'?t|wouldn'?t) (?:not )?be )(?:called|invoked|used|accessed|reached|callable|accessible)",
            )
            .expect("asserted_unavailability pattern 8"),
            // P9 (#1177 Shape C inverted): "unable to call/invoke/use/access/reach X"
            regex::Regex::new(
                r"(?i)\bunable to (?:call|invoke|use|access|reach) (?P<tool>[a-z_][a-z0-9_]*)\b",
            )
            .expect("asserted_unavailability pattern 9"),
        ]
    });

/// Detects asserted-unavailability phrases in assistant text.
///
/// Scans the text for one of nine compound-doc-cited patterns (P1-P5 original,
/// P6-P9 from #1177). If a match
/// is found AND the captured tool name is in the `enabled_tools` set (turn-start
/// snapshot), returns `Some(tool_name)`. Otherwise returns `None`.
///
/// Two-layer false-positive filter (F5): the snake-case capture group constraint
/// filters most natural-language matches; the enabled-set lookup filters the rest.
/// A sentence like "the service is not available" extracts `service`, which is
/// not in the registry → `None` → no violation.
pub(crate) fn detect_asserted_unavailability(
    text: &str,
    enabled_tools: &HashSet<String>,
) -> Option<String> {
    for re in ASSERTED_UNAVAILABILITY_PATTERNS.iter() {
        for caps in re.captures_iter(text) {
            // Normalize to lowercase: `(?i)` makes the capture group match
            // mixed-case text (e.g., "Search_Memory"), but the enabled_tools
            // HashSet contains lowercase names from tool definitions. Without
            // normalization, a mixed-case capture silently fails the lookup.
            let tool_name = caps["tool"].to_ascii_lowercase();
            if enabled_tools.contains(&tool_name) {
                return Some(tool_name);
            }
        }
    }
    None
}

/// Returns `true` when the asserted-unavailability guard should NOT fire
/// (i.e., the assertion is structurally true or backed by a real attempt).
///
/// Satisfied when:
/// - `tool_name` is NOT in `enabled_tools` (assertion is structurally true), OR
/// - a call to `tool_name` was *attempted* in this turn (success or failure).
///   The guard's purpose is to force an attempt, not a successful outcome.
///   When the tool was called and returned a real error (auth, rate limit,
///   network), the agent has evidence of the failure mode — that is a real
///   signal, not a fabrication.
pub(crate) fn asserted_unavailability_satisfied(
    tool_name: &str,
    enabled_tools: &HashSet<String>,
    summaries: &[ToolCallSummary],
) -> bool {
    !enabled_tools.contains(tool_name) || summaries.iter().any(|s| s.name == tool_name)
}

// ---------------------------------------------------------------------------
// #1331 — Assert-grounded guard (affirmative state-claim detection)
// ---------------------------------------------------------------------------

/// Label used for `intent_guard_retries` tracking of the assert-grounded
/// guard (#1331). Inline guard (not in `INTENT_GUARDS` const array) because it
/// checks *assistant* text and needs `all_tool_summaries` + dynamic correction.
pub const ASSERT_GROUNDED_LABEL: &str = "assert_grounded";

/// Tools that ground a verifiable state claim about a resource.
pub const GROUNDING_TOOLS: &[&str] = &["run_gh", "check_task", "gh_read"];

/// Structured result from affirmative state-claim detection.
pub struct AffirmativeStateClaim {
    pub resource_type: &'static str,
    pub resource_ref: String,
    pub claim_text: String,
}

/// Four regex patterns detecting affirmative state claims about resources.
/// Mirror of `ASSERTED_UNAVAILABILITY_PATTERNS` for affirmative (not negative) claims.
static AFFIRMATIVE_STATE_CLAIM_PATTERNS: std::sync::LazyLock<Vec<regex::Regex>> =
    std::sync::LazyLock::new(|| {
        vec![
            // Pattern 1: "I checked/confirmed/verified/reviewed/inspected/looked at the issue/PR #N"
            regex::Regex::new(
                r"(?i)\bI (?:checked|confirmed|verified|reviewed|inspected|looked at) (?:the )?(?P<rtype>issue|PR|pull request|task|ticket) #(?P<ref>\d+)",
            )
            .expect("assert_grounded pattern 1"),
            // Pattern 2: "I checked/confirmed/verified/reviewed the issue/PR and it's <state>"
            // Requires resource-type noun but may lack #N — caller extracts ref from vicinity.
            regex::Regex::new(
                r"(?i)\bI (?:checked|confirmed|verified|reviewed|inspected|looked at) (?:the )?(?P<rtype>issue|PR|pull request|task|ticket) and (?:it's|it is|they're|they are) (?P<state>\w+)",
            )
            .expect("assert_grounded pattern 2"),
            // Pattern 3: "issue/PR #N is/was/has been <state>"
            regex::Regex::new(
                r"(?i)\b(?P<rtype>issue|PR|pull request|task|ticket) #(?P<ref>\d+) (?:is|was|has been) (?:groomed|merged|closed|completed|ready|approved|reviewed|open|blocked)",
            )
            .expect("assert_grounded pattern 3"),
            // Pattern 4: "the handler/callback/subprocess/dispatch (already) closed/completed/... the issue/PR/task"
            regex::Regex::new(
                r"(?i)\b(?:the handler|the callback|the subprocess|the dispatch) (?:already )?(?:closed|completed|merged|finished|resolved) (?:the )?(?P<rtype>issue|PR|pull request|task|ticket)",
            )
            .expect("assert_grounded pattern 4"),
        ]
    });

/// Regex for extracting a GitHub issue/PR number from nearby text.
static RESOURCE_REF_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"#(\d+)").expect("resource_ref pattern"));

/// UUID pattern for task references.
static TASK_UUID_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
        .expect("task_uuid pattern")
});

/// Detects affirmative state claims about referenced resources in assistant text.
///
/// Scans for one of four high-precision claim patterns. If a pattern matches,
/// attempts to extract the resource reference (`#N` for issues/PRs, UUID for tasks).
/// Returns `None` when no pattern matches OR when a pattern matches but no resource
/// reference can be extracted (lean-narrow fail-open per D2/OQ1).
pub(crate) fn detect_affirmative_state_claim(text: &str) -> Option<AffirmativeStateClaim> {
    for (idx, re) in AFFIRMATIVE_STATE_CLAIM_PATTERNS.iter().enumerate() {
        if let Some(caps) = re.captures(text) {
            let matched_text = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let resource_type = match caps.name("rtype") {
                Some(m) => {
                    let rt = m.as_str().to_ascii_lowercase();
                    match rt.as_str() {
                        "pr" | "pull request" => "PR",
                        "issue" => "issue",
                        "task" => "task",
                        "ticket" => "ticket",
                        _ => "issue",
                    }
                }
                None => continue,
            };

            // Try to extract resource ref from named capture group first
            if let Some(ref_match) = caps.name("ref") {
                return Some(AffirmativeStateClaim {
                    resource_type,
                    resource_ref: format!("#{}", ref_match.as_str()),
                    claim_text: matched_text.to_string(),
                });
            }

            // For patterns without inline #N (Pattern 2, Pattern 4):
            // search the surrounding text for a resource reference.
            let match_start = caps.get(0).map(|m| m.start()).unwrap_or(0);
            let search_start = match_start.saturating_sub(100);
            let search_end = (match_start + 200).min(text.len());
            let vicinity = &text[search_start..search_end];

            // For task-type claims, try UUID first
            if resource_type == "task"
                && let Some(uuid_match) = TASK_UUID_RE.find(vicinity)
            {
                return Some(AffirmativeStateClaim {
                    resource_type,
                    resource_ref: uuid_match.as_str().to_string(),
                    claim_text: matched_text.to_string(),
                });
            }

            // Try #N extraction from vicinity
            if let Some(ref_caps) = RESOURCE_REF_RE.captures(vicinity)
                && let Some(num) = ref_caps.get(1)
            {
                return Some(AffirmativeStateClaim {
                    resource_type,
                    resource_ref: format!("#{}", num.as_str()),
                    claim_text: matched_text.to_string(),
                });
            }

            // Pattern matched but no resource ref extractable → fail-open (D2 lean-narrow)
            // Log for observability but don't fire the guard.
            debug!(
                pattern = idx + 1,
                matched = matched_text,
                "assert_grounded: pattern matched but no resource ref extractable — skipping"
            );
        }
    }
    None
}

/// Returns `true` when the assert-grounded guard should NOT fire
/// (i.e., a grounding tool call for the claimed resource exists in the turn).
///
/// Accepts any call attempt (success or failure) matching the resource ref,
/// same as `asserted_unavailability_satisfied`. The purpose is to force an
/// attempt — a failed `run_gh` means the agent tried to verify (real failure
/// is a signal, not fabrication).
pub(crate) fn assert_grounded_satisfied(
    claim: &AffirmativeStateClaim,
    summaries: &[ToolCallSummary],
) -> bool {
    // Extract the bare number from "#500" → "500" for matching against input_summary
    let bare_ref = claim.resource_ref.trim_start_matches('#');

    summaries
        .iter()
        .any(|s| GROUNDING_TOOLS.contains(&s.name.as_str()) && s.input_summary.contains(bare_ref))
}

// ---------------------------------------------------------------------------
// #1645 — Cross-artifact equivalence-claim guard (qa-review-scoped)
// ---------------------------------------------------------------------------

/// Label used for `intent_guard_retries` tracking of the equivalence-claim
/// guard (#1645). Inline guard, scoped to qa-review (via the `qa_pr_view`
/// tool's presence in the turn-start enabled set). Sibling of the
/// assert-grounded guard (#1331), specialized for cross-artifact equivalence
/// assertions ("duplicate of #X", "content identical", "same as PR #Y").
pub const EQUIVALENCE_CLAIM_LABEL: &str = "equivalence_claim";

/// Tools whose call grounds a cross-artifact equivalence claim by fetching the
/// *compared* artifact: `run_gh` (`pr diff` / `pr list` / `issue view`),
/// `qa_pr_view` (the compared PR's metadata + file list), and `gh_read` (the
/// architect read path). The satisfaction predicate additionally requires the
/// compared artifact's reference to appear in the tool's input — qa-review
/// always fetches the *current* PR's diff in Step 2, so a bare "any diff call"
/// check would trivially satisfy the guard and defeat it.
pub const EQUIVALENCE_GROUNDING_TOOLS: &[&str] = &["run_gh", "qa_pr_view", "gh_read"];

/// Structured result from cross-artifact equivalence-claim detection.
pub struct EquivalenceClaim {
    /// The compared artifact reference (e.g. `#1638`), extracted from the
    /// vicinity of the equivalence keyword.
    pub compared_ref: String,
    /// The matched claim-keyword fragment (for logging).
    pub claim_text: String,
}

/// Four regex patterns detecting cross-artifact equivalence assertions. Keyword
/// list per mika#1645 AC1: `identical` (in equivalence context), `duplicate of`,
/// `duplicate to`, `same as`, `equivalent to`, `content identical`.
static EQUIVALENCE_CLAIM_PATTERNS: std::sync::LazyLock<Vec<regex::Regex>> =
    std::sync::LazyLock::new(|| {
        vec![
            // P1: "duplicate of" / "duplicate to" — "Duplicate of merged mika#1638"
            regex::Regex::new(r"(?i)\bduplicate (?:of|to)\b").expect("equivalence_claim pattern 1"),
            // P2: "identical" in equivalence context — "content identical",
            // "identical to PR #X", "is/are identical".
            regex::Regex::new(
                r"(?i)\b(?:content[s]?\s+identical|identical\s+to|(?:is|are|both)\s+identical)\b",
            )
            .expect("equivalence_claim pattern 2"),
            // P3: "same as" — "same as #X"
            regex::Regex::new(r"(?i)\bsame as\b").expect("equivalence_claim pattern 3"),
            // P4: "equivalent to" — "equivalent to commit Z"
            regex::Regex::new(r"(?i)\bequivalent to\b").expect("equivalence_claim pattern 4"),
        ]
    });

/// Detects cross-artifact equivalence assertions in assistant text.
///
/// Scans for one of four equivalence-keyword patterns. On a match, extracts the
/// *compared* artifact reference (`#N`) — biased FORWARD of the keyword, since
/// the compared artifact normally follows it ("duplicate of <ref>", "identical
/// to <ref>"), with a bounded fall-back to a reference shortly before. Returns
/// `None` when no pattern matches OR no nearby reference can be extracted
/// (lean-narrow fail-open, mirroring the assert-grounded guard's D2 policy).
///
/// Reference extraction iterates `#N` matches and compares byte positions
/// instead of slicing the string — panic-safe on multi-byte text (the founding
/// incident verdict contained an em-dash; see mika#764 byte-slice lint).
pub(crate) fn detect_equivalence_claim(text: &str) -> Option<EquivalenceClaim> {
    // Fast path: skip regex when no candidate substring is present.
    let lower = text.to_lowercase();
    if !(lower.contains("duplicate")
        || lower.contains("identical")
        || lower.contains("same as")
        || lower.contains("equivalent to"))
    {
        return None;
    }

    const FORWARD_WINDOW: usize = 200;
    const BACKWARD_WINDOW: usize = 100;

    for re in EQUIVALENCE_CLAIM_PATTERNS.iter() {
        let Some(kw) = re.find(text) else { continue };
        let kw_start = kw.start();
        let kw_end = kw.end();

        let mut after: Option<&str> = None;
        let mut before: Option<&str> = None;
        for caps in RESOURCE_REF_RE.captures_iter(text) {
            let whole = caps.get(0).expect("regex match 0 always present");
            let num = caps.get(1).expect("resource_ref capture group 1").as_str();
            let pos = whole.start();
            if pos >= kw_end {
                if after.is_none() && pos - kw_end <= FORWARD_WINDOW {
                    after = Some(num);
                }
            } else if pos < kw_start && kw_start - pos <= BACKWARD_WINDOW {
                // Iteration is position-ascending, so the last assignment is
                // the reference closest before the keyword.
                before = Some(num);
            }
        }

        if let Some(num) = after.or(before) {
            return Some(EquivalenceClaim {
                compared_ref: format!("#{num}"),
                claim_text: kw.as_str().to_string(),
            });
        }

        // Pattern matched but no nearby ref → fail-open; try the next pattern.
        debug!(
            matched = kw.as_str(),
            "equivalence_claim: pattern matched but no nearby resource ref — skipping"
        );
    }
    None
}

/// Returns `true` when the equivalence-claim guard should NOT fire (i.e. a
/// grounding tool call that fetched the *compared* artifact exists in the turn).
///
/// Accepts any attempt (success or failure) to an equivalence-grounding tool
/// whose input references the compared artifact — same accept-any-attempt
/// semantics as `assert_grounded_satisfied`.
pub(crate) fn equivalence_claim_satisfied(
    claim: &EquivalenceClaim,
    summaries: &[ToolCallSummary],
) -> bool {
    let bare_ref = claim.compared_ref.trim_start_matches('#');
    summaries.iter().any(|s| {
        EQUIVALENCE_GROUNDING_TOOLS.contains(&s.name.as_str()) && s.input_summary.contains(bare_ref)
    })
}

// ---------------------------------------------------------------------------
// #1680 — Non-Latin-script (CJK/Hangul) response-quality guard
// ---------------------------------------------------------------------------

/// Label used for `intent_guard_retries` tracking of the non-Latin-script
/// guard (#1680). Inline guard (not in `INTENT_GUARDS`): checks *assistant*
/// text and needs a dynamic correction message that quotes the offending
/// substring.
///
/// Response-quality class, NOT fabrication class (architect F1): CJK bleed is a
/// wrong-language regression, not a grounding failure — the guard sits AFTER
/// `persistence-eval` in the EndTurn chain, not among the fabrication guards
/// (6c/6d). It is therefore intentionally absent from the #953 guard-fabrication
/// telemetry pairs.
pub const NON_LATIN_SCRIPT_LABEL: &str = "non_latin_script";

/// Codepoint-count threshold for firing the non-Latin-script guard.
///
/// Architect F3 rationale: 5 catches the observed emission density (the mika#1680
/// sample was 576 chars of fluent Mandarin — well above threshold) while leaving
/// headroom for incidental single-character technical content (a lone CJK char
/// quoted verbatim from a foreign issue body). Raise to 10 if false positives are
/// observed on legitimate foreign-language quotes.
pub const NON_LATIN_SCRIPT_THRESHOLD: usize = 5;

/// Max chars of the offending substring quoted back in the correction message.
const NON_LATIN_SCRIPT_SAMPLE_CHARS: usize = 100;

/// A non-Latin-script detection hit.
pub struct NonLatinScriptHit {
    /// Number of codepoints from the targeted CJK/Hangul ranges.
    pub count: usize,
    /// First `NON_LATIN_SCRIPT_SAMPLE_CHARS` chars starting at the first
    /// offending codepoint — quoted in the correction message (architect F4)
    /// so the model sees exactly what it emitted.
    pub sample: String,
}

/// Returns `true` when `c` is in one of the targeted CJK/Hangul ranges.
///
/// Ranges (mika#1680): CJK Symbols and Punctuation (U+3000–U+303F), Hiragana
/// (U+3040–U+309F), Katakana (U+30A0–U+30FF), CJK Unified Ideographs
/// (U+4E00–U+9FFF), Hangul Syllables (U+AC00–U+D7AF). Deliberately NOT general
/// non-ASCII: emoji, accented Latin, and mathematical symbols are legitimate
/// output and must not trip the guard (plan § Out of scope — "General Unicode
/// normalization").
fn is_targeted_cjk_codepoint(c: char) -> bool {
    matches!(c,
        '\u{3000}'..='\u{303F}'   // CJK Symbols and Punctuation
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{4E00}'..='\u{9FFF}' // CJK Unified Ideographs
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
    )
}

/// Detects CJK/Hangul-script bleed in assistant text (mika#1680).
///
/// Counts codepoints in the targeted ranges; returns `Some(hit)` once the count
/// reaches `NON_LATIN_SCRIPT_THRESHOLD`, else `None`. The hit carries a sample of
/// the offending region (starting at the first CJK/Hangul codepoint) for the
/// correction message.
///
/// Root cause: CN-trained models (glm-5.2 on mika-dev/mika-qa since mika#1633 /
/// mika#1670) respond in Chinese to English-language prompts. This guard is the
/// engine-level defense-in-depth behind the prompt-level English-only instruction
/// in the agents' souls and skill prompts — prompt enforcement alone does not bind
/// across model classes (see
/// `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`).
pub(crate) fn detect_non_latin_script(text: &str) -> Option<NonLatinScriptHit> {
    let count = text
        .chars()
        .filter(|c| is_targeted_cjk_codepoint(*c))
        .count();
    if count < NON_LATIN_SCRIPT_THRESHOLD {
        return None;
    }
    let sample: String = text
        .chars()
        .skip_while(|c| !is_targeted_cjk_codepoint(*c))
        .take(NON_LATIN_SCRIPT_SAMPLE_CHARS)
        .collect();
    Some(NonLatinScriptHit { count, sample })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_execution::ToolCallSummary;

    // -- detect_fabricated_action_claim tests (#308) --

    #[test]
    fn test_detect_fabricated_action_claim_comment_posted() {
        let text = "Comment posted: https://github.com/senara-solutions/mika/pull/307#issuecomment-4146200192";
        let result = detect_fabricated_action_claim(text);
        assert!(result.is_some());
        let (verb, url) = result.unwrap();
        assert_eq!(verb, "posted");
        assert!(url.contains("#issuecomment-4146200192"));
    }

    #[test]
    fn test_detect_fabricated_action_claim_review_submitted() {
        let text = "I've reviewed the PR: https://github.com/org/repo/pull/42#pullrequestreview-99";
        let result = detect_fabricated_action_claim(text);
        assert!(result.is_some());
        let (verb, _url) = result.unwrap();
        assert_eq!(verb, "reviewed");
    }

    #[test]
    fn test_detect_fabricated_action_claim_issue_created() {
        let text = "I created the issue at https://github.com/org/repo/issues/123 for tracking.";
        let result = detect_fabricated_action_claim(text);
        assert!(result.is_some());
        let (verb, url) = result.unwrap();
        assert_eq!(verb, "created");
        assert!(url.contains("/issues/123"));
    }

    #[test]
    fn test_detect_fabricated_action_claim_left_a_comment() {
        let text = "I left a comment on https://github.com/org/repo/pull/5#issuecomment-100";
        let result = detect_fabricated_action_claim(text);
        assert!(result.is_some());
        let (verb, _) = result.unwrap();
        assert_eq!(verb, "left a comment");
    }

    #[test]
    fn test_detect_fabricated_action_claim_discussion_comment() {
        let text =
            "I've submitted my feedback at https://github.com/org/repo/pull/10#discussion_r555";
        let result = detect_fabricated_action_claim(text);
        assert!(result.is_some());
        let (verb, url) = result.unwrap();
        assert_eq!(verb, "submitted");
        assert!(url.contains("#discussion_r555"));
    }

    #[test]
    fn test_detect_fabricated_action_claim_no_github_url() {
        let text = "I posted the comment on Slack.";
        assert!(detect_fabricated_action_claim(text).is_none());
    }

    #[test]
    fn test_detect_fabricated_action_claim_no_action_verb() {
        let text = "You can view the PR at https://github.com/org/repo/pull/42#issuecomment-100";
        assert!(detect_fabricated_action_claim(text).is_none());
    }

    #[test]
    fn test_detect_fabricated_action_claim_plain_repo_url() {
        // A bare repo URL without resource anchor should not match
        let text = "I posted at https://github.com/org/repo";
        assert!(detect_fabricated_action_claim(text).is_none());
    }

    #[test]
    fn test_detect_fabricated_action_claim_no_github_fast_path() {
        // All inputs without "github.com/" hit the fast-path early return
        assert!(detect_fabricated_action_claim("").is_none());
        assert!(
            detect_fabricated_action_claim("I posted a comment on the issue tracker.").is_none()
        );
    }

    #[test]
    fn test_detect_fabricated_action_claim_case_insensitive_verb() {
        let text = "POSTED the review: https://github.com/org/repo/pull/1#pullrequestreview-42";
        let result = detect_fabricated_action_claim(text);
        assert!(result.is_some());
        let (verb, _) = result.unwrap();
        assert_eq!(verb, "POSTED");
    }

    #[test]
    fn test_detect_fabricated_action_claim_synonym_verbs() {
        // Verb synonyms added per review #754
        for verb in &["added", "wrote", "replied", "approved", "filed", "raised"] {
            let text =
                format!("I {verb} a review at https://github.com/org/repo/pull/1#issuecomment-42");
            let result = detect_fabricated_action_claim(&text);
            assert!(result.is_some(), "should detect verb: {verb}");
            assert_eq!(result.unwrap().0, *verb);
        }
    }

    #[test]
    fn test_detect_fabricated_action_claim_markdown_link() {
        // LLMs often emit markdown link syntax — the regex must match through `)`
        let text = "I posted [a comment](https://github.com/org/repo/pull/307#issuecomment-4146200192) on the PR.";
        let result = detect_fabricated_action_claim(text);
        assert!(result.is_some());
        let (verb, url) = result.unwrap();
        assert_eq!(verb, "posted");
        assert!(url.contains("#issuecomment-4146200192"));
    }

    // -- Callback state claim detection tests (#716) --

    #[test]
    fn test_detect_callback_claim_no_pr() {
        let result = detect_unverified_callback_state_claim("There was no PR created");
        assert!(result.is_some());
        assert!(result.unwrap().to_lowercase().contains("no pr"));
    }

    #[test]
    fn test_detect_callback_claim_without_pr() {
        // "without PR" is standalone — fast path matches "without pr"
        let result =
            detect_unverified_callback_state_claim("The run ended without PR being created");
        assert!(result.is_some());
    }

    #[test]
    fn test_detect_callback_claim_manually_closed() {
        let result = detect_unverified_callback_state_claim("Issue was manually closed by someone");
        assert!(result.is_some());
    }

    #[test]
    fn test_detect_callback_claim_handler_crashed() {
        let result = detect_unverified_callback_state_claim("The handler crashed");
        assert!(result.is_some());
    }

    #[test]
    fn test_detect_callback_claim_no_commits() {
        let result = detect_unverified_callback_state_claim("The branch had no commits on it.");
        assert!(result.is_some());
    }

    #[test]
    fn test_detect_callback_claim_no_branch() {
        let result = detect_unverified_callback_state_claim("There is no branch for this work");
        assert!(result.is_some());
    }

    #[test]
    fn test_detect_callback_claim_closed_without() {
        let result = detect_unverified_callback_state_claim("It was closed without any resolution");
        assert!(result.is_some());
    }

    #[test]
    fn test_detect_callback_claim_no_match_normal_text() {
        assert!(detect_unverified_callback_state_claim("Task completed successfully").is_none());
    }

    #[test]
    fn test_detect_callback_claim_no_match_empty() {
        assert!(detect_unverified_callback_state_claim("").is_none());
    }

    #[test]
    fn test_detect_callback_claim_case_insensitive() {
        assert!(detect_unverified_callback_state_claim("NO PR was found").is_some());
        assert!(
            detect_unverified_callback_state_claim("Handler Crashed during execution").is_some()
        );
    }

    // -- #862/#894 asserted-unavailability detection and satisfaction tests --

    #[test]
    fn test_asserted_unavailability_satisfied_not_in_registry() {
        let enabled = HashSet::new();
        let summaries = vec![];
        assert!(
            asserted_unavailability_satisfied("gh_read", &enabled, &summaries),
            "Tool not in enabled set = assertion is true = satisfied"
        );
    }

    #[test]
    fn test_asserted_unavailability_satisfied_successful_call() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "gh_read".to_string(),
            input_summary: "op: issue_view".to_string(),
            output_summary: "Issue #862: ...".to_string(),
            success: true,
            non_zero_exit: false,
        }];
        assert!(
            asserted_unavailability_satisfied("gh_read", &enabled, &summaries),
            "Tool called successfully = satisfied"
        );
    }

    #[test]
    fn test_asserted_unavailability_not_satisfied_no_call() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        let summaries = vec![]; // no calls
        assert!(
            !asserted_unavailability_satisfied("gh_read", &enabled, &summaries),
            "Tool in enabled set with no call = NOT satisfied"
        );
    }

    #[test]
    fn test_asserted_unavailability_satisfied_failed_call() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "gh_read".to_string(),
            input_summary: "op: issue_view".to_string(),
            output_summary: "Error: auth failed".to_string(),
            success: false,
            non_zero_exit: false,
        }];
        assert!(
            asserted_unavailability_satisfied("gh_read", &enabled, &summaries),
            "Tool in enabled set with failed call = satisfied (attempt was made, \
             real failure surfaced — not a fabrication)"
        );
    }

    // -- #894 asserted-unavailability elided-copula + adverb-interposed detection tests --

    #[test]
    fn test_detect_asserted_unavailability_elided_copula() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // Elided copula: "gh_read not callable in CLI session" (mika#893 verbatim shape)
        assert_eq!(
            detect_asserted_unavailability("gh_read not callable in CLI session", &enabled),
            Some("gh_read".to_string()),
            "Elided copula 'X not callable' must match (mika#893 shape)"
        );
        // Elided copula with "not available"
        assert_eq!(
            detect_asserted_unavailability("gh_read not available here", &enabled),
            Some("gh_read".to_string()),
            "Elided copula 'X not available' must match"
        );
        // Elided copula with "not accessible"
        assert_eq!(
            detect_asserted_unavailability("gh_read not accessible in this mode", &enabled),
            Some("gh_read".to_string()),
            "Elided copula 'X not accessible' must match"
        );
    }

    #[test]
    fn test_detect_asserted_unavailability_adverb_interposed() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // Adverb interposed with copula: "gh_read is structurally not callable" (mika#863 shape)
        assert_eq!(
            detect_asserted_unavailability(
                "gh_read is structurally not callable in this session",
                &enabled
            ),
            Some("gh_read".to_string()),
            "Adverb-interposed 'X is structurally not callable' must match (mika#863 shape)"
        );
        // Adverb interposed without copula: "gh_read structurally not callable"
        assert_eq!(
            detect_asserted_unavailability("gh_read structurally not callable", &enabled),
            Some("gh_read".to_string()),
            "Elided copula + adverb 'X structurally not callable' must match"
        );
        // Adverb interposed with isn't (P3): "gh_read isn't currently callable"
        assert_eq!(
            detect_asserted_unavailability("gh_read isn't currently callable", &enabled),
            Some("gh_read".to_string()),
            "Adverb-interposed isn't 'X isn't currently callable' must match"
        );
    }

    #[test]
    fn test_detect_asserted_unavailability_elided_skill_scoped() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // Elided copula on skill-scoped: "gh_read skill-scoped" (mika#654 variant)
        assert_eq!(
            detect_asserted_unavailability("gh_read skill-scoped, not callable here", &enabled),
            Some("gh_read".to_string()),
            "Elided copula 'X skill-scoped' must match (mika#654 variant)"
        );
    }

    #[test]
    fn test_detect_asserted_unavailability_elided_copula_natural_language_filtered() {
        let mut enabled = HashSet::new();
        enabled.insert("search_memory".to_string());
        // "service not available" — elided form of existing natural-language filter test.
        // "service" is not in the enabled set → None.
        assert_eq!(
            detect_asserted_unavailability("the service not available right now", &enabled),
            None,
            "Natural language 'service not available' (elided copula) must still be \
             filtered by the enabled-set lookup — 'service' is not a tool"
        );
    }

    // -- #1177 Shape A: descriptor-word absorption tests --

    #[test]
    fn test_detect_asserted_unavailability_descriptor_word_absorption() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // Shape A: "the gh_read tool is not available" — must capture "gh_read", not "tool"
        let result = detect_asserted_unavailability("the gh_read tool is not available", &enabled);
        assert_eq!(
            result,
            Some("gh_read".to_string()),
            "Descriptor-word 'the gh_read tool is not available' must capture 'gh_read' \
             (not 'tool') and match (mika#1177 Shape A)"
        );
    }

    #[test]
    fn test_detect_asserted_unavailability_descriptor_word_variants() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // "function" descriptor
        assert_eq!(
            detect_asserted_unavailability("the gh_read function is not callable", &enabled),
            Some("gh_read".to_string()),
            "Descriptor 'function': must capture gh_read"
        );
        // "skill" descriptor
        assert_eq!(
            detect_asserted_unavailability("the gh_read skill is not accessible", &enabled),
            Some("gh_read".to_string()),
            "Descriptor 'skill': must capture gh_read"
        );
    }

    #[test]
    fn test_detect_asserted_unavailability_descriptor_word_filter_preserved() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // "the service tool is not available" — "service" not in enabled set → None
        assert_eq!(
            detect_asserted_unavailability("the service tool is not available right now", &enabled),
            None,
            "Natural language 'the service tool is not available' must still be \
             filtered — 'service' is not a tool"
        );
        // "the storage feature is not callable" — "storage" not in enabled set → None
        assert_eq!(
            detect_asserted_unavailability("the storage feature is not callable", &enabled),
            None,
            "Natural language 'the storage feature is not callable' must still be \
             filtered — 'storage' is not a tool"
        );
    }

    // -- #1177 Shape B: antonym `unavailable` tests --

    #[test]
    fn test_detect_asserted_unavailability_antonym_unavailable() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // "gh_read is currently unavailable"
        assert_eq!(
            detect_asserted_unavailability("gh_read is currently unavailable", &enabled),
            Some("gh_read".to_string()),
            "Antonym 'gh_read is currently unavailable' must match (mika#1177 Shape B)"
        );
    }

    #[test]
    fn test_detect_asserted_unavailability_antonym_unavailable_variants() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // bare "gh_read unavailable"
        assert_eq!(
            detect_asserted_unavailability("gh_read unavailable in this session", &enabled),
            Some("gh_read".to_string()),
            "'gh_read unavailable' (no copula) must match"
        );
        // "gh_read is unavailable"
        assert_eq!(
            detect_asserted_unavailability("gh_read is unavailable", &enabled),
            Some("gh_read".to_string()),
            "'gh_read is unavailable' must match"
        );
        // "gh_read structurally unavailable"
        assert_eq!(
            detect_asserted_unavailability("gh_read structurally unavailable", &enabled),
            Some("gh_read".to_string()),
            "'gh_read structurally unavailable' (adverb) must match"
        );
    }

    #[test]
    fn test_detect_asserted_unavailability_antonym_unavailable_filter_preserved() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // "the service is currently unavailable" — "service" not in enabled set → None
        assert_eq!(
            detect_asserted_unavailability("the service is currently unavailable", &enabled),
            None,
            "Natural language 'the service is currently unavailable' must still be \
             filtered — 'service' is not a tool"
        );
    }

    // -- #1177 Shape C: modal / periphrastic negation tests --

    #[test]
    fn test_detect_asserted_unavailability_modal_negation() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // "gh_read may not be callable"
        assert_eq!(
            detect_asserted_unavailability("gh_read may not be callable", &enabled),
            Some("gh_read".to_string()),
            "Modal 'gh_read may not be callable' must match (mika#1177 Shape C)"
        );
        // "gh_read could not be called"
        assert_eq!(
            detect_asserted_unavailability("gh_read could not be called", &enabled),
            Some("gh_read".to_string()),
            "Modal 'gh_read could not be called' must match"
        );
        // "gh_read cannot be invoked here"
        assert_eq!(
            detect_asserted_unavailability("gh_read cannot be invoked here", &enabled),
            Some("gh_read".to_string()),
            "Modal 'gh_read cannot be invoked' must match"
        );
    }

    #[test]
    fn test_detect_asserted_unavailability_doesnt_appear() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // "gh_read doesn't appear to be callable"
        assert_eq!(
            detect_asserted_unavailability("gh_read doesn't appear to be callable", &enabled),
            Some("gh_read".to_string()),
            "'gh_read doesn't appear to be callable' must match (mika#1177 Shape C)"
        );
    }

    #[test]
    fn test_detect_asserted_unavailability_unable_to() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // "unable to call gh_read"
        assert_eq!(
            detect_asserted_unavailability("unable to call gh_read", &enabled),
            Some("gh_read".to_string()),
            "Inverted 'unable to call gh_read' must match (mika#1177 Shape C)"
        );
        // "unable to invoke gh_read in this session"
        assert_eq!(
            detect_asserted_unavailability("unable to invoke gh_read in this session", &enabled),
            Some("gh_read".to_string()),
            "Inverted 'unable to invoke gh_read in this session' must match"
        );
    }

    #[test]
    fn test_detect_asserted_unavailability_modal_filter_preserved() {
        let mut enabled = HashSet::new();
        enabled.insert("gh_read".to_string());
        // "service may not be called from this context" — "service" not in enabled set → None
        assert_eq!(
            detect_asserted_unavailability("service may not be called from this context", &enabled),
            None,
            "Natural language 'service may not be called' must still be filtered"
        );
        // "unable to reach the storage service" — "storage" not in enabled set → None
        assert_eq!(
            detect_asserted_unavailability("unable to reach the storage service", &enabled),
            None,
            "Natural language 'unable to reach the storage service' must still be \
             filtered — 'storage' is not a tool (and 'service' is trailing)"
        );
    }

    // -- #1331 assert-grounded detection tests --

    #[test]
    fn test_detect_affirmative_state_claim_pattern_1_issue() {
        let result = detect_affirmative_state_claim("I checked the issue #500 and it's groomed");
        let claim = result.expect("Pattern 1 should match");
        assert_eq!(claim.resource_type, "issue");
        assert_eq!(claim.resource_ref, "#500");
    }

    #[test]
    fn test_detect_affirmative_state_claim_pattern_1_pr() {
        let result = detect_affirmative_state_claim("I reviewed PR #123 — no issues found");
        let claim = result.expect("Pattern 1 should match PR");
        assert_eq!(claim.resource_type, "PR");
        assert_eq!(claim.resource_ref, "#123");
    }

    #[test]
    fn test_detect_affirmative_state_claim_pattern_2_with_nearby_ref() {
        // Pattern 2 matches the claim shape; the #456 is nearby in text
        let result =
            detect_affirmative_state_claim("Looking at #456, I confirmed the PR and it's merged");
        let claim = result.expect("Pattern 2 should match with nearby ref");
        assert_eq!(claim.resource_type, "PR");
        assert_eq!(claim.resource_ref, "#456");
    }

    #[test]
    fn test_detect_affirmative_state_claim_pattern_3_issue() {
        let result = detect_affirmative_state_claim("Issue #500 is groomed and ready for dispatch");
        let claim = result.expect("Pattern 3 should match");
        assert_eq!(claim.resource_type, "issue");
        assert_eq!(claim.resource_ref, "#500");
    }

    #[test]
    fn test_detect_affirmative_state_claim_pattern_3_passive_pr() {
        let result = detect_affirmative_state_claim("PR #123 has been merged");
        let claim = result.expect("Pattern 3 should match passive PR");
        assert_eq!(claim.resource_type, "PR");
        assert_eq!(claim.resource_ref, "#123");
    }

    #[test]
    fn test_detect_affirmative_state_claim_pattern_4_with_task_uuid() {
        let result = detect_affirmative_state_claim(
            "For task a1b2c3d4-e5f6-7890-abcd-ef1234567890, \
             the handler already closed the task",
        );
        let claim = result.expect("Pattern 4 should match with task UUID");
        assert_eq!(claim.resource_type, "task");
        assert_eq!(claim.resource_ref, "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
    }

    #[test]
    fn test_detect_affirmative_state_claim_no_match_casual_reference() {
        assert!(
            detect_affirmative_state_claim("This relates to the #500 groom we did").is_none(),
            "Casual reference should not match"
        );
    }

    #[test]
    fn test_detect_affirmative_state_claim_no_match_discussion() {
        assert!(
            detect_affirmative_state_claim("See #500 for details on the approach").is_none(),
            "Discussion reference should not match"
        );
    }

    #[test]
    fn test_detect_affirmative_state_claim_no_match_question() {
        assert!(
            detect_affirmative_state_claim("Is issue #500 groomed yet?").is_none(),
            "Question should not match"
        );
    }

    #[test]
    fn test_detect_affirmative_state_claim_no_match_negation() {
        assert!(
            detect_affirmative_state_claim("I haven't checked issue #500 yet").is_none(),
            "Negation should not match"
        );
    }

    #[test]
    fn test_detect_affirmative_state_claim_pattern_2_no_resource_ref() {
        // Pattern 2 matches text shape but no #N or UUID in vicinity → None
        assert!(
            detect_affirmative_state_claim("I confirmed the PR and it's merged").is_none(),
            "Pattern 2 without resource ref should return None (lean-narrow fail-open)"
        );
    }

    #[test]
    fn test_detect_affirmative_state_claim_pattern_4_no_resource_ref() {
        // Pattern 4 matches text shape but no task UUID or #N nearby → None
        assert!(
            detect_affirmative_state_claim("The handler already closed the task").is_none(),
            "Pattern 4 without resource ref should return None (lean-narrow fail-open)"
        );
    }

    // -- #1331 assert-grounded satisfaction predicate tests --

    #[test]
    fn test_assert_grounded_satisfied_run_gh_matching_ref() {
        let claim = AffirmativeStateClaim {
            resource_type: "issue",
            resource_ref: "#500".to_string(),
            claim_text: "I checked issue #500".to_string(),
        };
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: "gh issue view 500 --json state".to_string(),
            output_summary: "open".to_string(),
            success: true,
            non_zero_exit: false,
        }];
        assert!(
            assert_grounded_satisfied(&claim, &summaries),
            "run_gh with matching ref and success=true should satisfy"
        );
    }

    #[test]
    fn test_assert_grounded_not_satisfied_different_ref() {
        let claim = AffirmativeStateClaim {
            resource_type: "issue",
            resource_ref: "#500".to_string(),
            claim_text: "I checked issue #500".to_string(),
        };
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: "gh issue view 123 --json state".to_string(),
            output_summary: "open".to_string(),
            success: true,
            non_zero_exit: false,
        }];
        assert!(
            !assert_grounded_satisfied(&claim, &summaries),
            "run_gh with different ref should NOT satisfy"
        );
    }

    #[test]
    fn test_assert_grounded_satisfied_failed_run_gh() {
        // A failed run_gh still shows the agent attempted verification —
        // real failure is a signal, not fabrication (matches
        // asserted_unavailability's accept-any-attempt pattern).
        let claim = AffirmativeStateClaim {
            resource_type: "PR",
            resource_ref: "#500".to_string(),
            claim_text: "PR #500 is merged".to_string(),
        };
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: "gh pr view 500".to_string(),
            output_summary: "Error: auth failed".to_string(),
            success: false,
            non_zero_exit: false,
        }];
        assert!(
            assert_grounded_satisfied(&claim, &summaries),
            "run_gh attempt with matching ref should satisfy (even on failure)"
        );
    }

    #[test]
    fn test_assert_grounded_satisfied_check_task() {
        let claim = AffirmativeStateClaim {
            resource_type: "task",
            resource_ref: "a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string(),
            claim_text: "the handler already closed the task".to_string(),
        };
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "check_task".to_string(),
            input_summary: "task_id: a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string(),
            output_summary: "completed".to_string(),
            success: true,
            non_zero_exit: false,
        }];
        assert!(
            assert_grounded_satisfied(&claim, &summaries),
            "check_task with matching task ref should satisfy"
        );
    }

    #[test]
    fn test_assert_grounded_satisfied_gh_read() {
        let claim = AffirmativeStateClaim {
            resource_type: "issue",
            resource_ref: "#500".to_string(),
            claim_text: "Issue #500 is groomed".to_string(),
        };
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "gh_read".to_string(),
            input_summary: "op: issue_view, target: 500".to_string(),
            output_summary: "Issue #500: groomed".to_string(),
            success: true,
            non_zero_exit: false,
        }];
        assert!(
            assert_grounded_satisfied(&claim, &summaries),
            "gh_read with matching ref should satisfy"
        );
    }

    #[test]
    fn test_assert_grounded_not_satisfied_empty_summaries() {
        let claim = AffirmativeStateClaim {
            resource_type: "issue",
            resource_ref: "#500".to_string(),
            claim_text: "Issue #500 is groomed".to_string(),
        };
        assert!(
            !assert_grounded_satisfied(&claim, &[]),
            "Empty summaries should NOT satisfy"
        );
    }

    #[test]
    fn test_assert_grounded_not_satisfied_unrelated_tools() {
        let claim = AffirmativeStateClaim {
            resource_type: "issue",
            resource_ref: "#500".to_string(),
            claim_text: "I checked issue #500".to_string(),
        };
        let summaries = vec![
            ToolCallSummary {
                step: 0,
                name: "search_memory".to_string(),
                input_summary: "query: issue 500".to_string(),
                output_summary: "found 2 results".to_string(),
                success: true,
                non_zero_exit: false,
            },
            ToolCallSummary {
                step: 1,
                name: "store_fact".to_string(),
                input_summary: "category: issues".to_string(),
                output_summary: "stored".to_string(),
                success: true,
                non_zero_exit: false,
            },
        ];
        assert!(
            !assert_grounded_satisfied(&claim, &summaries),
            "Non-grounding tools should NOT satisfy"
        );
    }

    #[test]
    fn test_assert_grounded_satisfied_grounding_call_after_claim_text() {
        // Confirms same-turn ordering irrelevance (D3/Step 2)
        let claim = AffirmativeStateClaim {
            resource_type: "PR",
            resource_ref: "#500".to_string(),
            claim_text: "PR #500 looks good".to_string(),
        };
        // Summaries are accumulated over the full turn; a grounding call
        // appended after the claim text still satisfies the predicate.
        let summaries = vec![
            ToolCallSummary {
                step: 0,
                name: "search_memory".to_string(),
                input_summary: "query: something".to_string(),
                output_summary: "results".to_string(),
                success: true,
                non_zero_exit: false,
            },
            ToolCallSummary {
                step: 2,
                name: "run_gh".to_string(),
                input_summary: "gh pr view 500 --json state".to_string(),
                output_summary: "merged".to_string(),
                success: true,
                non_zero_exit: false,
            },
        ];
        assert!(
            assert_grounded_satisfied(&claim, &summaries),
            "Grounding call at any step in the turn should satisfy"
        );
    }

    // -- #1645 cross-artifact equivalence-claim detection tests --

    #[test]
    fn test_detect_equivalence_claim_founding_incident() {
        // The verbatim mika#1644 incident verdict (em-dash included).
        let result = detect_equivalence_claim(
            "VERDICT: hold[review] — Duplicate of merged mika#1638 — content identical; \
             dispatch-lib opened a second wip-rescue vehicle.",
        );
        let claim = result.expect("founding-incident verdict should match");
        assert_eq!(claim.compared_ref, "#1638");
    }

    #[test]
    fn test_detect_equivalence_claim_content_identical() {
        let result =
            detect_equivalence_claim("The diff is content identical to the prior PR #900.");
        let claim = result.expect("'content identical' should match");
        assert_eq!(claim.compared_ref, "#900");
    }

    #[test]
    fn test_detect_equivalence_claim_identical_to() {
        let result = detect_equivalence_claim("This PR is identical to PR #512.");
        let claim = result.expect("'identical to' should match");
        assert_eq!(claim.compared_ref, "#512");
    }

    #[test]
    fn test_detect_equivalence_claim_same_as() {
        let result = detect_equivalence_claim("These changes are the same as #777.");
        let claim = result.expect("'same as' should match");
        assert_eq!(claim.compared_ref, "#777");
    }

    #[test]
    fn test_detect_equivalence_claim_equivalent_to() {
        let result = detect_equivalence_claim("Functionally equivalent to #321 already merged.");
        let claim = result.expect("'equivalent to' should match");
        assert_eq!(claim.compared_ref, "#321");
    }

    #[test]
    fn test_detect_equivalence_claim_forward_bias_over_current_pr() {
        // The current PR (#1644) precedes the keyword; the compared artifact
        // (#1638) follows it. Forward bias must select the compared artifact.
        let result =
            detect_equivalence_claim("PR #1644 is a duplicate of the merged work in #1638.");
        let claim = result.expect("forward-bias case should match");
        assert_eq!(
            claim.compared_ref, "#1638",
            "compared artifact (#1638), not the current PR (#1644), must be chosen"
        );
    }

    #[test]
    fn test_detect_equivalence_claim_no_ref_fail_open() {
        // Keyword present but no nearby #N → fail-open (None).
        assert!(
            detect_equivalence_claim("The two implementations are content identical.").is_none(),
            "no nearby resource ref should fail-open to None"
        );
    }

    #[test]
    fn test_detect_equivalence_claim_no_keyword() {
        assert!(
            detect_equivalence_claim("PR #1644 adds a new calibration scenario for mika-qa.")
                .is_none(),
            "non-equivalence verdict should not match"
        );
    }

    // -- #1645 equivalence-claim satisfaction predicate tests --

    #[test]
    fn test_equivalence_claim_satisfied_run_gh_pr_diff_compared_ref() {
        let claim = EquivalenceClaim {
            compared_ref: "#1638".to_string(),
            claim_text: "duplicate of".to_string(),
        };
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: "gh pr diff 1638 --name-only".to_string(),
            output_summary: "skills/bundled/qa-review/system_prompt.md".to_string(),
            success: true,
            non_zero_exit: false,
        }];
        assert!(
            equivalence_claim_satisfied(&claim, &summaries),
            "run_gh pr diff of the compared artifact should satisfy"
        );
    }

    #[test]
    fn test_equivalence_claim_not_satisfied_only_current_pr_diff() {
        // qa-review's Step 2 always fetches the CURRENT PR's diff (#1644).
        // That does NOT ground an equivalence claim about #1638 — the guard
        // must still fire.
        let claim = EquivalenceClaim {
            compared_ref: "#1638".to_string(),
            claim_text: "content identical".to_string(),
        };
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: "gh pr diff 1644 --name-only".to_string(),
            output_summary: "crates/mika-agent/src/calibration/roles/mika_qa.rs".to_string(),
            success: true,
            non_zero_exit: false,
        }];
        assert!(
            !equivalence_claim_satisfied(&claim, &summaries),
            "current-PR diff (#1644) must NOT satisfy a claim about #1638"
        );
    }

    #[test]
    fn test_equivalence_claim_satisfied_qa_pr_view_compared_ref() {
        let claim = EquivalenceClaim {
            compared_ref: "#1638".to_string(),
            claim_text: "same as".to_string(),
        };
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "qa_pr_view".to_string(),
            input_summary: "pr_url: https://github.com/senara-solutions/mika/pull/1638".to_string(),
            output_summary: "PR #1638: files ...".to_string(),
            success: true,
            non_zero_exit: false,
        }];
        assert!(
            equivalence_claim_satisfied(&claim, &summaries),
            "qa_pr_view of the compared PR (URL contains 1638) should satisfy"
        );
    }

    #[test]
    fn test_equivalence_claim_not_satisfied_empty_summaries() {
        let claim = EquivalenceClaim {
            compared_ref: "#1638".to_string(),
            claim_text: "duplicate of".to_string(),
        };
        assert!(
            !equivalence_claim_satisfied(&claim, &[]),
            "no tool calls should NOT satisfy"
        );
    }

    #[test]
    fn test_equivalence_claim_satisfied_failed_attempt() {
        // A failed fetch still shows the reviewer attempted the comparison —
        // a real failure is a signal, not a fabrication (accept-any-attempt).
        let claim = EquivalenceClaim {
            compared_ref: "#1638".to_string(),
            claim_text: "duplicate of".to_string(),
        };
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: "gh pr diff 1638".to_string(),
            output_summary: "Error: rate limited".to_string(),
            success: false,
            non_zero_exit: false,
        }];
        assert!(
            equivalence_claim_satisfied(&claim, &summaries),
            "failed attempt to fetch the compared artifact should satisfy"
        );
    }

    // -- detect_non_latin_script tests (#1680) --

    #[test]
    fn test_non_latin_script_pure_ascii_returns_none() {
        let text = "mika#1665 grooming completed — plan on fix/1665 branch, ready label applied. \
                    Task f76f594e created and in_progress.";
        assert!(
            detect_non_latin_script(text).is_none(),
            "pure-ASCII (with an em-dash) must not trip the CJK guard"
        );
    }

    #[test]
    fn test_non_latin_script_five_cjk_returns_some() {
        // Exactly five CJK Unified Ideographs reaches the threshold.
        let text = "Status update: 引擎已确认 dispatch is authorized.";
        let hit = detect_non_latin_script(text).expect("5 CJK codepoints must fire");
        assert_eq!(hit.count, 5);
        assert!(
            hit.sample.starts_with('引'),
            "sample must start at the first offending codepoint"
        );
    }

    #[test]
    fn test_non_latin_script_four_cjk_below_threshold() {
        // Four CJK codepoints is below the threshold of five — no fire.
        let text = "Quoting an issue title: 引擎已确 (partial) for context.";
        assert!(
            detect_non_latin_script(text).is_none(),
            "4 CJK codepoints must stay below the threshold boundary"
        );
    }

    #[test]
    fn test_non_latin_script_mixed_japanese_kanji_returns_some() {
        // Mixed English + Japanese (kanji + hiragana) over the threshold fires.
        let text = "Summary: エンジンが確認しました — dispatch proceeding.";
        let hit = detect_non_latin_script(text).expect("mixed Japanese script must fire");
        assert!(hit.count >= NON_LATIN_SCRIPT_THRESHOLD);
    }

    #[test]
    fn test_non_latin_script_hangul_returns_some() {
        // Korean Hangul syllables over the threshold fire.
        let text = "Result: 엔진이확인되었습니다 dispatch ok.";
        assert!(
            detect_non_latin_script(text).is_some(),
            "Hangul syllables over the threshold must fire"
        );
    }

    #[test]
    fn test_non_latin_script_emoji_and_accents_do_not_fire() {
        // Emoji and accented Latin are legitimate non-ASCII — must NOT fire,
        // even in quantity (plan § Out of scope: general Unicode normalization).
        let text = "Déjà vu — résumé café naïve 🚀✅🔥📦🎯 done.";
        assert!(
            detect_non_latin_script(text).is_none(),
            "emoji and accented Latin must not be treated as CJK bleed"
        );
    }

    #[test]
    fn test_non_latin_script_sample_capped_at_100_chars() {
        // A long CJK run is sampled to at most NON_LATIN_SCRIPT_SAMPLE_CHARS chars.
        let long_cjk: String = std::iter::repeat('确').take(300).collect();
        let hit = detect_non_latin_script(&long_cjk).expect("300 CJK chars must fire");
        assert_eq!(hit.count, 300);
        assert_eq!(
            hit.sample.chars().count(),
            NON_LATIN_SCRIPT_SAMPLE_CHARS,
            "sample must be capped at the sample-char limit"
        );
    }
}
