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
// mika#1814 — Distribution Doctrine (public-promo) guard
// ---------------------------------------------------------------------------

/// Label used for `intent_guard_retries` tracking of the Distribution Doctrine
/// public-promo guard (mika#1814). Inline guard (not in `INTENT_GUARDS` const
/// array) because it checks *assistant* text and needs a dynamic correction
/// message. Sibling of `dev_groom_fabrication` (5b) and
/// `fabricated_action_claim` (5) — all three catch doctrine violations expressed
/// in a proposal / drafting shape.
pub(crate) const DOCTRINE_PUBLIC_PROMO_LABEL: &str = "doctrine_public_promo";

/// Structured result from Distribution Doctrine public-promo detection.
pub(crate) struct DoctrinePublicPromoMatch {
    /// The prohibited-surface keyword captured by Layer A (e.g. `Show HN`).
    pub(crate) subject: String,
    /// The proposal / drafting verb captured by Layer B (e.g. `let's`, `rédiger`).
    pub(crate) verb: String,
}

/// Layer A — prohibited public-launch surfaces (Show HN, Product Hunt, etc.).
///
/// Word-bounded so ambient prose ("I read a Reddit post about X") does not
/// trigger without the qualifying launch/thread/promo keyword. Extended past
/// the founding-incident seed to cover shapes the adversarial review surfaced
/// (mika#1814 code-review 2026-08-22):
/// - Direct forms: `show hn`, `hacker news launch`, `product hunt`.
/// - Punctuation-tolerant: `[\s\-_/]*` between compound-name halves so
///   `Show-HN`, `Product-Hunt`, `Show_HN`, `Show/HN` also match.
/// - Bare `HN` when adjacent to a launch-context noun
///   (`post`/`launch`/`thread`/`drop`/`submission`) — catches
///   `"préparer un post pour HN"`.
/// - Reversed word order: `launch on hacker news` / `thread on reddit`
///   (Layer A was previously subject-first only).
/// - Reddit / Twitter launch shapes with common launch-verb / thread nouns
///   in either direction.
/// - Growth-hack with tolerant separator class.
static DOCTRINE_SUBJECT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?ix)                                                # case-insensitive, extended
        \b(
              show[\s\-_/]*hn                                   # Show HN, Show-HN, Show_HN
            | hacker[\s\-_/]*news\s+(?:launch|thread|post|drop) # HN launch/thread/post/drop
            | product[\s\-_/]*hunt                              # Product Hunt / -Hunt / _Hunt
            | reddit\s+(?:launch|thread\s+launch|drop)          # Reddit launch shapes
            | hn\s+(?:post|launch|thread|drop|submission)       # bare HN + launch noun
            | (?:launch|thread|post|drop|submission)\s+
                  (?:on|to|at|pour|sur|[aà])\s+
                  (?:hacker[\s\-_/]*news|reddit|hn|product[\s\-_/]*hunt)
                                                                # reversed: launch on/pour/sur HN / Reddit / PH
            | twitter\s+(?:promo|launch|thread(?:\s+promo)?)    # Twitter launch shapes
            | thread\s+on\s+twitter                             # reversed: thread on Twitter
            | growth[\s\-_/]*hack                               # growth-hack / growth hack / growth/hack
        )\b",
    )
    .expect("doctrine subject regex must compile")
});

/// Layer B — first-person / second-person proposal, drafting, or planning
/// verb. Bilingual (French for family-tier Al re-play + English for
/// operator-tier). Requires both layers to fire so an educational answer
/// ("Mika does not do Show HN — she grows by invitation") does not match.
///
/// Extended past the seed to cover the very common `write`/`draft`/`help`
/// verb classes the adversarial review surfaced. A helpful Mika will say
/// `"I'll write the Show HN copy"` or `"I'd love to help you draft the
/// Product Hunt post"` far more often than the narrow gerund `drafting`
/// the seed regex was pinned to.
static DOCTRINE_PROPOSAL_VERB_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?ix)                                          # case-insensitive, extended
            \b(?:
                  let'?s                                     # let's
                | on\s+va | on\s+peut | je\s+peux            # FR proposal verbs
                | i\s+can | we\s+can                         # EN proposal verbs
                | i'?ll\s+(?:write|help|draft|prepare|start) # I'll write / I'll help
                | we'?ll\s+(?:write|help|draft|prepare|start)# we'll write / we'll help
                | i'?d\s+(?:love\s+to|be\s+happy\s+to)       # I'd love to / I'd be happy to
                | help\s+(?:you\s+)?(?:write|draft|prepare)  # help you write / help write
                | draft(?:s|ed|ing)?                         # draft / drafts / drafted / drafting
                | writ(?:e|es|ing)                           # write / writes / writing
                | r[eé]dig(?:er|eons|eant|é)?                # rédiger / rédigeons / rédigé
                | prepar(?:e|es|ing)                         # prepare / prepares / preparing
                | plan\s+(?:for|the|out|a|an)                # plan for / plan the / plan out
                | next\s+step\s+(?:is|would\s+be)            # next step is / would be
                | prochaine\s+[eé]tape                       # prochaine étape
                | brouillon                                  # FR: draft (noun)
                | j[eaæ]?\s+vais\s+(?:pr[eé]parer|[eé]crire) # je vais préparer / écrire
            )\b",
        )
        .expect("doctrine proposal-verb regex must compile")
    });

/// Doctrine-alignment override — assistant text that explicitly cites the
/// invitation-only redirect is a compliant response (or an educational answer
/// about the doctrine), not a violation. This carves out the false-positive
/// class the adversarial review surfaced where a legitimate meta-discussion
/// ("Let's remember growth-hack is prohibited", "I can explain why Mika does
/// not do Show HN") happens to trigger both layers of the regex.
///
/// Any one of these substrings (case-insensitive) suppresses the guard fire.
/// Kept narrow — the exact prescriptive fragments from the Distribution
/// Doctrine section itself — to avoid becoming a bypass shape.
static DOCTRINE_ALIGNMENT_SIGNALS: &[&str] = &[
    "Mika grandit par invitation entre proches",
    "Mika grows through personal invitation",
    "invitation entre proches",
    "personal invitation between people who know each other",
    "invitation-only distribution",
];

/// Detects Distribution Doctrine violations — assistant text that proposes,
/// drafts, or plans one of the prohibited public-launch surfaces (Show HN,
/// Product Hunt, Reddit launch, Twitter promo thread, growth-hack tactics).
///
/// Two-layer AND filter with a doctrine-alignment override (mirror of
/// `asserted_unavailability` shape, extended per mika#1814 adversarial review):
/// - **Layer A (subject match):** one of the prohibited-surface keywords —
///   direct, punctuation-tolerant, reversed-word-order, bare-HN-with-noun,
///   Twitter-thread variants, growth-hack.
/// - **Layer B (verb match):** a first-person / second-person proposal,
///   drafting, or planning verb (bilingual FR + EN — extended `write`/`draft`
///   /`help you write` class beyond the founding-incident seed).
/// - **Doctrine-alignment override:** if the response ALSO contains a
///   canonical invitation-chain redirect fragment (from the Distribution
///   Doctrine section), the guard suppresses. Educational answers about the
///   doctrine and compliant redirects that mention the surfaces to say what
///   Mika does NOT do pass through cleanly.
///
/// Both regex layers must match AND the alignment override must NOT be
/// present for a fire.
///
/// Fast path: skip both regex compiles when the cheap `contains` check finds
/// no candidate substring in the lowercased text. The fast-path list mirrors
/// the substring atoms of `DOCTRINE_SUBJECT_RE`; adding a new surface to the
/// regex requires updating both.
pub(crate) fn detect_doctrine_public_promo(text: &str) -> Option<DoctrinePublicPromoMatch> {
    // Fast path: none of the surface substrings present → return early.
    // Whitespace-normalize the lowered text so `"hacker news  launch"`
    // (double space, tab, etc.) hits the substring probe. The regex layer
    // uses `\s+`, so the fast path must not under-filter it.
    let lower_raw = text.to_lowercase();
    let lower: String = lower_raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let has_candidate = lower.contains("show hn")
        || lower.contains("showhn")
        || lower.contains("show-hn")
        || lower.contains("show_hn")
        || lower.contains("show/hn")
        || lower.contains("hacker news launch")
        || lower.contains("hacker news thread")
        || lower.contains("hacker news post")
        || lower.contains("hacker news drop")
        || lower.contains("hn post")
        || lower.contains("hn launch")
        || lower.contains("hn thread")
        || lower.contains("hn drop")
        || lower.contains("hn submission")
        || lower.contains("product hunt")
        || lower.contains("producthunt")
        || lower.contains("product-hunt")
        || lower.contains("product_hunt")
        || lower.contains("reddit launch")
        || lower.contains("reddit thread launch")
        || lower.contains("reddit drop")
        || lower.contains("launch on hacker")
        || lower.contains("launch on reddit")
        || lower.contains("launch on hn")
        || lower.contains("launch on product hunt")
        || lower.contains("launch pour hn")
        || lower.contains("launch pour hacker")
        || lower.contains("launch sur hn")
        || lower.contains("launch sur hacker")
        || lower.contains("thread on hacker")
        || lower.contains("thread on reddit")
        || lower.contains("thread on hn")
        || lower.contains("thread on twitter")
        || lower.contains("thread pour hn")
        || lower.contains("thread pour hacker")
        || lower.contains("thread sur hn")
        || lower.contains("thread sur hacker")
        || lower.contains("post on hn")
        || lower.contains("post on reddit")
        || lower.contains("post on hacker")
        || lower.contains("post pour hn")
        || lower.contains("post pour hacker")
        || lower.contains("post pour reddit")
        || lower.contains("post sur hn")
        || lower.contains("post sur hacker")
        || lower.contains("post sur reddit")
        || lower.contains("post à hn")
        || lower.contains("post à hacker")
        || lower.contains("drop on hn")
        || lower.contains("drop pour hn")
        || lower.contains("drop sur hn")
        || lower.contains("submission on hn")
        || lower.contains("submission pour hn")
        || lower.contains("twitter promo")
        || lower.contains("twitter launch")
        || lower.contains("twitter thread")
        || lower.contains("growth hack")
        || lower.contains("growth-hack")
        || lower.contains("growthhack")
        || lower.contains("growth/hack");
    if !has_candidate {
        return None;
    }

    let subject_match = DOCTRINE_SUBJECT_RE.find(text)?;
    let verb_match = DOCTRINE_PROPOSAL_VERB_RE.find(text)?;

    // Doctrine-alignment override — a compliant redirect or educational
    // answer citing the invitation-chain script is not a violation.
    if DOCTRINE_ALIGNMENT_SIGNALS
        .iter()
        .any(|signal| text.contains(signal))
    {
        return None;
    }

    Some(DoctrinePublicPromoMatch {
        subject: subject_match.as_str().to_string(),
        verb: verb_match.as_str().to_string(),
    })
}

// ---------------------------------------------------------------------------
// mika#1646 — Destructive-action grounding guard (pre-execution)
// ---------------------------------------------------------------------------
//
// Sibling of assert-grounded (mika#1331) and equivalence-claim (mika#1645),
// with one structural difference that governs everything below: **those guards
// fire at EndTurn, this one cannot**.
//
// The other two inspect assistant *text* and re-prompt. mika#1646's defect is
// not a sentence — it is a tool call. `gh pr close 1644` leaves at step 3 of
// the tool loop; by the time the EndTurn arm runs, the PR is already closed and
// a re-prompt can only comment on an accomplished fact. So the predicates here
// are consumed by a **pre-subprocess gate** in `run_gh`
// (`skills::builtin_handlers`), alongside the mika#1682 / mika#1196 / mika#1167
// gates that already refuse `gh` calls before any side effect.
//
// Founding incident: mika-dev closed PR #1644 twice in 9 minutes on the same
// fabricated "duplicate of mika#1638" rationale, the second time with a
// human's diff-grounded contradiction sitting in the thread. The second close
// came from a *deferred webhook replay* — a context sharing no in-memory state
// with the first. That is why repeat detection reads the persisted `tool_calls`
// table rather than any turn- or session-local state: the second execution has
// to KNOW it is a second execution, which is a property of the record, not of
// the process that happens to be running.

/// Audit-event `tool_name` for every destructive-action decision (AC3).
///
/// `audit_events` has no `event_type` column — the schema is free-form
/// `tool_name TEXT NOT NULL` — so this follows the established convention of
/// `phantom_aged_out` (mika#1712) and `wip_rescue` (mika#1852). No migration.
pub const DESTRUCTIVE_ACTION_AUDIT_TOOL: &str = "destructive_action_grounding";

/// Env var tuning the repeat-detection window, in seconds (architect F1).
pub const REPEAT_ACTION_WINDOW_ENV: &str = "MIKA_DEV_REPEAT_ACTION_WINDOW_SECS";

/// Default repeat-detection window: 30 minutes.
///
/// The founding incident's two closes were 6m55s apart; the window has to be
/// comfortably wider than the observed gap without reaching so far back that
/// an unrelated legitimate close on the same target is caught.
pub const REPEAT_ACTION_WINDOW_DEFAULT_SECS: i64 = 1800;

/// Tools whose call can ground a destructive action by fetching the target's
/// current state. Mirrors `GROUNDING_TOOLS` (mika#1331) plus the qa read path.
pub const DESTRUCTIVE_GROUNDING_TOOLS: &[&str] = &["run_gh", "qa_pr_view", "gh_read"];

/// The kind of resource a destructive action terminates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructiveTargetKind {
    Pr,
    Issue,
}

impl DestructiveTargetKind {
    /// The `gh` noun, as it appears in the command.
    pub fn noun(self) -> &'static str {
        match self {
            DestructiveTargetKind::Pr => "pr",
            DestructiveTargetKind::Issue => "issue",
        }
    }
}

/// A recognized destructive action: `gh pr close <N>` or `gh issue close <N>`.
#[derive(Debug, Clone)]
pub struct DestructiveAction {
    pub kind: DestructiveTargetKind,
    /// Bare target number, no `#` prefix (e.g. `1644`).
    pub number: String,
    /// `--comment` / `--body` text supplied with the close, when present. This
    /// is the text AC1 requires to carry the file-list justification and AC2
    /// requires to acknowledge a prior action.
    pub comment: Option<String>,
}

impl DestructiveAction {
    /// Stable identity of the *action*, used as the audit `target_key` and as
    /// the repeat-detection key: `pr:close:1644`.
    pub fn target_key(&self) -> String {
        format!("{}:close:{}", self.kind.noun(), self.number)
    }
}

/// Recognizes a destructive close in a `gh` argv.
///
/// Bounded on purpose to `pr close` / `issue close` (plan § Future expansion):
/// detection is **fail-open**, because a gate that cannot tell what it is
/// looking at must not turn into a blanket refusal of `gh`. Everything AFTER
/// recognition is fail-closed — see `run_gh`'s gate.
///
/// The noun and verb are matched positionally (`args[0]`, `args[1]`) because
/// that is the only shape `gh` accepts, and the target number is taken as the
/// first subsequent bare argument that parses as a number — flags and their
/// values are skipped, so `gh pr close --comment "see #99" 1644` yields 1644
/// and not 99.
pub fn detect_destructive_action(args: &[String]) -> Option<DestructiveAction> {
    let kind = match args.first().map(String::as_str) {
        Some("pr") => DestructiveTargetKind::Pr,
        Some("issue") => DestructiveTargetKind::Issue,
        _ => return None,
    };
    if args.get(1).map(String::as_str) != Some("close") {
        return None;
    }

    // Flags that take a separate value; their value must not be mistaken for
    // the target number.
    // Flags that consume a SEPARATE following argument. Boolean flags must NOT
    // be listed here: skipping two positions past one swallows the argument
    // after it, and if that argument is the target number the action stops
    // being recognized at all — a fail-open hole exactly where the gate is
    // supposed to bite (`gh pr close --delete-branch 1644`).
    const VALUE_FLAGS: &[&str] = &[
        "--comment",
        "-c",
        "--body",
        "-b",
        "--repo",
        "-R",
        "--reason",
    ];

    let mut number: Option<String> = None;
    let mut comment: Option<String> = None;
    let mut i = 2usize;
    while i < args.len() {
        let arg = &args[i];
        if let Some((flag, inline)) = arg.split_once('=')
            && flag.starts_with("--")
        {
            if matches!(flag, "--comment" | "--body") {
                comment = Some(inline.to_string());
            }
            i += 1;
            continue;
        }
        if arg.starts_with('-') {
            if VALUE_FLAGS.contains(&arg.as_str()) {
                if matches!(arg.as_str(), "--comment" | "-c" | "--body" | "-b")
                    && let Some(v) = args.get(i + 1)
                {
                    comment = Some(v.clone());
                }
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if number.is_none() {
            let bare = arg.trim_start_matches('#');
            if !bare.is_empty() && bare.chars().all(|c| c.is_ascii_digit()) {
                number = Some(bare.to_string());
            }
        }
        i += 1;
    }

    number.map(|number| DestructiveAction {
        kind,
        number,
        comment,
    })
}

/// Layer A — is the destructive action grounded in the target's current state?
///
/// Satisfied when the current turn contains a call to a grounding tool whose
/// input names BOTH this target and a state-reading verb (`view` / `diff`).
/// `rows` are this turn's tool calls, read from `tool_calls` by `trace_id`.
///
/// Requiring the *verb* matters: a turn that only ran `gh pr close 1644` would
/// otherwise satisfy a bare "any run_gh mentioning 1644" check with the
/// destructive call itself.
pub fn destructive_grounding_satisfied<'a>(
    action: &DestructiveAction,
    tool_inputs: impl Iterator<Item = (&'a str, &'a str)>,
) -> bool {
    let needle = &action.number;
    tool_inputs.into_iter().any(|(name, input)| {
        if !DESTRUCTIVE_GROUNDING_TOOLS.contains(&name) {
            return false;
        }
        // Anchor on the JSON quoting of the serialized argv, so #1644 does not
        // match #16440 (the DB-side repeat query is anchored the same way).
        if !input.contains(&format!("\"{needle}\"")) {
            return false;
        }
        let lower = input.to_lowercase();
        // A read of the target: `pr view`, `pr diff`, `issue view`, or the
        // qa/arch read paths (whose whole purpose is reading).
        lower.contains("view") || lower.contains("diff") || name != "run_gh"
    })
}

/// Layer A, second half — does the close comment cite the grounding it claims?
///
/// AC1 requires the close comment to cite the file-list comparison, not just
/// that a read happened somewhere in the turn. A close whose comment carries no
/// evidence is the exact shape of the founding incident: mika-dev's two
/// comments both paraphrased qa's rationale and cited nothing.
///
/// Deliberately generous about *form* (any of several evidence markers) and
/// strict about *presence* — the point is to force the author to look at the
/// diff and say what they saw, not to impose a template.
pub fn destructive_comment_cites_evidence(comment: Option<&str>) -> bool {
    let Some(text) = comment else { return false };
    let lower = text.to_lowercase();
    // Two families, both describing something the author SAW.
    //
    // Naming another ticket is deliberately NOT enough: "duplicate of
    // mika#1638" is verbatim the rationale that got replayed twice in the
    // founding incident, and admitting it would defeat the guard. What counts
    // is an observation — a file, a diff, a read-back state — that a reviewer
    // can check against the artifact.
    const EVIDENCE_MARKERS: &[&str] = &[
        // Family 1 — the diff / file-list comparison (AC1's named form).
        "--json files",
        "json files",
        "file list",
        "files changed",
        "changed files",
        "file diff",
        "pr diff",
        "diff shows",
        "diff confirms",
        "intersection",
        "no overlap",
        "zero overlap",
        "overlap:",
        "no commits",
        "no changes",
        "empty diff",
        "crates/",
        "skills/",
        "docs/",
        "scripts/",
        ".rs",
        ".toml",
        ".yaml",
        ".yml",
        // Family 2 — a read-back state, for the legitimate administrative
        // close that has no diff to cite (obsolete ticket, superseded scope).
        // These describe what `gh issue view` / `gh pr view` returned, not what
        // someone else concluded.
        "closed as",
        "state:",
        "labels:",
        "merged at",
        "merged_at",
        "already merged",
        "branch deleted",
    ];
    EVIDENCE_MARKERS.iter().any(|m| lower.contains(m))
}

/// Layer B — does a repeated action's comment acknowledge the prior one?
///
/// Presence of a prior identical action inside the window makes this a *second
/// execution*. AC2 requires the new action's body to say so explicitly, which
/// is what distinguishes idempotence by intention from idempotence by accident:
/// a replayed rationale cannot accidentally contain an acknowledgment it was
/// never written with.
pub fn destructive_repeat_acknowledged(comment: Option<&str>) -> bool {
    let Some(text) = comment else { return false };
    let lower = text.to_lowercase();
    const ACK_MARKERS: &[&str] = &[
        "previously closed",
        "prior close",
        "closed before",
        "closed earlier",
        "re-close",
        "reclose",
        "reclosing",
        "re-closing",
        "closing again",
        "second close",
        "reopened",
        "re-opened",
        "reviewed the comments since",
        "comments since the prior",
        "prior action",
        "earlier close",
        "after reviewing the reopen",
    ];
    ACK_MARKERS.iter().any(|m| lower.contains(m))
}

/// Reads the repeat-detection window from the environment (architect F1).
///
/// Absent / empty → default. Unparseable or non-positive → default, WARN-logged
/// (same three-tier shape as `MIKA_WIP_RESCUE_MIN_AGE_SECS`, mika#1852). A `0`
/// does NOT disable the guard: on a destructive action the safe default is to
/// keep checking, so an operator typo cannot silently reopen the hole.
pub fn repeat_action_window_secs() -> i64 {
    parse_repeat_window(std::env::var(REPEAT_ACTION_WINDOW_ENV).ok().as_deref())
}

/// Pure half of `repeat_action_window_secs`, split out so the three-tier
/// fallback is testable without mutating process env (edition 2024 makes
/// `set_var` unsafe, and parallel tests would race on it).
pub fn parse_repeat_window(raw: Option<&str>) -> i64 {
    match raw {
        None => REPEAT_ACTION_WINDOW_DEFAULT_SECS,
        Some(s) if s.trim().is_empty() => REPEAT_ACTION_WINDOW_DEFAULT_SECS,
        Some(s) => match s.trim().parse::<i64>() {
            Ok(v) if v > 0 => v,
            _ => {
                tracing::warn!(
                    event = "destructive_window_invalid",
                    raw = %s,
                    default_secs = REPEAT_ACTION_WINDOW_DEFAULT_SECS,
                    "invalid MIKA_DEV_REPEAT_ACTION_WINDOW_SECS; falling back to default"
                );
                REPEAT_ACTION_WINDOW_DEFAULT_SECS
            }
        },
    }
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

    // -- detect_doctrine_public_promo tests (mika#1814) --

    #[test]
    fn test_doctrine_public_promo_show_hn_french_proposal() {
        // Founding incident (Al B, 2026-07-20) verbatim shape.
        let text = "on avait convenu que la prochaine étape était de rédiger le \
                    brouillon pour Show HN — tu veux qu'on s'y attaque ensemble ?";
        let m = detect_doctrine_public_promo(text).expect("should fire on FR proposal");
        assert!(
            m.subject.to_lowercase().contains("show hn"),
            "expected 'Show HN' subject, got {:?}",
            m.subject
        );
        assert!(
            !m.verb.is_empty(),
            "expected non-empty proposal verb, got {:?}",
            m.verb
        );
    }

    #[test]
    fn test_doctrine_public_promo_product_hunt_english_proposal() {
        let text = "Let's draft a Product Hunt launch post — I'll write the first pass now.";
        let m = detect_doctrine_public_promo(text).expect("should fire on EN proposal");
        assert!(m.subject.to_lowercase().contains("product hunt"));
        assert!(m.verb.to_lowercase().starts_with("let"));
    }

    #[test]
    fn test_doctrine_public_promo_reddit_growth_hack_shape() {
        let text = "I can help you with a Reddit launch thread and a growth-hack angle for it.";
        let m = detect_doctrine_public_promo(text).expect("should fire on Reddit-launch proposal");
        // Layer A can pick either surface — assert the shape catches SOMETHING
        // rather than the specific first-match ordering.
        let subj = m.subject.to_lowercase();
        assert!(
            subj.contains("reddit launch") || subj.contains("growth"),
            "expected reddit-launch or growth-hack subject, got {:?}",
            m.subject
        );
    }

    #[test]
    fn test_doctrine_public_promo_educational_answer_does_not_fire() {
        // Layer A hit ("Show HN") but no proposal verb — legitimate education.
        let text = "Mika does not do Show HN; she grows via personal invitation.";
        assert!(
            detect_doctrine_public_promo(text).is_none(),
            "educational answer should NOT fire the guard"
        );
    }

    #[test]
    fn test_doctrine_public_promo_no_subject_match_does_not_fire() {
        // Layer B hit ("let's draft") but no prohibited surface — legitimate.
        let text = "Let's draft the PR description together — I can start now.";
        assert!(
            detect_doctrine_public_promo(text).is_none(),
            "proposal verb without prohibited surface should NOT fire the guard"
        );
    }

    #[test]
    fn test_doctrine_public_promo_ambient_reddit_does_not_fire() {
        // Common false-positive class: "Reddit" without "launch" is not a
        // prohibited surface (Reddit search discussion, Reddit article link,
        // etc.).
        let text = "We were discussing how the Reddit search algorithm works — \
                    can you look it up in memory?";
        assert!(
            detect_doctrine_public_promo(text).is_none(),
            "ambient Reddit mention should NOT fire the guard"
        );
    }

    #[test]
    fn test_doctrine_public_promo_case_insensitive() {
        let text = "on va rédiger un post SHOW HN dès que possible.";
        let m =
            detect_doctrine_public_promo(text).expect("should fire regardless of subject casing");
        assert_eq!(m.subject.to_lowercase().replace(' ', ""), "showhn");
    }

    // -- mika#1814 adversarial-review bypass-shape coverage --

    #[test]
    fn test_doctrine_public_promo_bare_draft_verb() {
        // Bare `draft` (not `drafting`) — adversarial P0.
        let text = "I'll draft a Product Hunt launch post now.";
        detect_doctrine_public_promo(text).expect("bare 'draft' verb should fire (adversarial P0)");
    }

    #[test]
    fn test_doctrine_public_promo_write_verb() {
        // Bare `write` verb — adversarial P0.
        let text = "I'll write the Show HN copy this afternoon.";
        detect_doctrine_public_promo(text).expect("'I'll write' verb should fire (adversarial P0)");
    }

    #[test]
    fn test_doctrine_public_promo_help_you_write() {
        // `help you write` verb — adversarial P0.
        let text = "I'd love to help you write the Show HN piece.";
        detect_doctrine_public_promo(text)
            .expect("'help you write' verb should fire (adversarial P0)");
    }

    #[test]
    fn test_doctrine_public_promo_bare_hn_with_noun() {
        // Bare `HN` abbreviation with launch-context noun — adversarial P0.
        let text = "Je vais préparer un post pour HN dès mardi.";
        detect_doctrine_public_promo(text)
            .expect("bare 'HN' with 'post' noun should fire (adversarial P0)");
    }

    #[test]
    fn test_doctrine_public_promo_reversed_launch_on_hacker_news() {
        // Reversed word order — adversarial P0.
        let text = "Let's plan a launch on Hacker News for next Tuesday.";
        detect_doctrine_public_promo(text)
            .expect("'launch on Hacker News' should fire (adversarial P0)");
    }

    #[test]
    fn test_doctrine_public_promo_reversed_thread_on_reddit() {
        let text = "Let's write a thread on Reddit to promote the launch.";
        detect_doctrine_public_promo(text)
            .expect("'thread on Reddit' should fire (adversarial P0)");
    }

    #[test]
    fn test_doctrine_public_promo_twitter_thread_bare() {
        // Bare `Twitter thread` (no `promo` qualifier) — adversarial P0.
        let text = "Let's write a Twitter thread announcing the launch.";
        detect_doctrine_public_promo(text)
            .expect("bare 'Twitter thread' should fire (adversarial P0)");
    }

    #[test]
    fn test_doctrine_public_promo_hyphenated_show_hn() {
        // Punctuation-tolerant separator class — adversarial P1.
        let text = "I'll draft the Show-HN post today.";
        detect_doctrine_public_promo(text)
            .expect("'Show-HN' (hyphen) should fire (adversarial P1)");
    }

    #[test]
    fn test_doctrine_public_promo_hyphenated_product_hunt() {
        let text = "Let's prepare a Product-Hunt piece.";
        detect_doctrine_public_promo(text)
            .expect("'Product-Hunt' (hyphen) should fire (adversarial P1)");
    }

    #[test]
    fn test_doctrine_public_promo_je_vais_ecrire_hn_post() {
        // FR `je vais écrire` shape — bilingual coverage.
        let text = "Je vais écrire un post pour HN cette semaine.";
        detect_doctrine_public_promo(text)
            .expect("'je vais écrire' + 'post pour HN' should fire (bilingual)");
    }

    #[test]
    fn test_doctrine_public_promo_double_space_hacker_news() {
        // Fast-path whitespace-normalization: double-space `hacker  news launch`.
        let text = "Let's plan the hacker  news launch for Tuesday morning.";
        detect_doctrine_public_promo(text)
            .expect("double-space between 'hacker' and 'news' should still fire");
    }

    #[test]
    fn test_doctrine_alignment_override_suppresses_educational_answer() {
        // Adversarial P1 FP: `i can` + `show hn` shape that is an
        // educational answer — must NOT fire because the response also
        // carries the invitation-chain redirect fragment (compliant shape).
        let text = "I can explain why Mika does not do Show HN — Mika \
                    grows through personal invitation between people who \
                    know each other.";
        assert!(
            detect_doctrine_public_promo(text).is_none(),
            "response citing 'personal invitation' should suppress the guard"
        );
    }

    #[test]
    fn test_doctrine_alignment_override_french_redirect_suppresses() {
        // Same shape, French — the FR invitation-chain fragment suppresses.
        let text = "Alors bien sûr on peut préparer un post pour HN, mais \
                    Mika grandit par invitation entre proches — je ne le \
                    ferai pas.";
        assert!(
            detect_doctrine_public_promo(text).is_none(),
            "FR response citing invitation-chain fragment should suppress"
        );
    }

    #[test]
    fn test_doctrine_alignment_override_still_fires_on_plain_violation() {
        // Sanity — the alignment override must not become a bypass shape.
        // A plain violation without any invitation-chain fragment still fires.
        let text = "Let's draft a Show HN post right now — I'll write the \
                    title and the three bullets.";
        detect_doctrine_public_promo(text)
            .expect("plain violation without alignment signal must still fire");
    }

    // -- mika#1646 destructive-action grounding tests --

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn detects_pr_close_with_number() {
        let a = detect_destructive_action(&argv(&["pr", "close", "1644"])).expect("detected");
        assert_eq!(a.kind, DestructiveTargetKind::Pr);
        assert_eq!(a.number, "1644");
        assert_eq!(a.target_key(), "pr:close:1644");
    }

    #[test]
    fn detects_issue_close_with_hash_prefix() {
        let a = detect_destructive_action(&argv(&["issue", "close", "#42"])).expect("detected");
        assert_eq!(a.kind, DestructiveTargetKind::Issue);
        assert_eq!(a.number, "42");
        assert_eq!(a.target_key(), "issue:close:42");
    }

    #[test]
    fn captures_comment_from_separate_flag() {
        let a = detect_destructive_action(&argv(&[
            "pr",
            "close",
            "1644",
            "--comment",
            "duplicate of #1638",
        ]))
        .expect("detected");
        assert_eq!(a.comment.as_deref(), Some("duplicate of #1638"));
    }

    #[test]
    fn captures_comment_from_inline_equals() {
        let a = detect_destructive_action(&argv(&["pr", "close", "7", "--comment=see the diff"]))
            .expect("detected");
        assert_eq!(a.comment.as_deref(), Some("see the diff"));
    }

    /// The founding-incident shape: a number inside the comment must not be
    /// mistaken for the target when the target comes later in the argv.
    #[test]
    fn comment_number_is_not_mistaken_for_target() {
        let a = detect_destructive_action(&argv(&[
            "pr",
            "close",
            "--comment",
            "duplicate of 1638",
            "1644",
        ]))
        .expect("detected");
        assert_eq!(a.number, "1644");
    }

    #[test]
    fn repo_flag_value_is_not_the_target() {
        let a = detect_destructive_action(&argv(&[
            "issue",
            "close",
            "--repo",
            "senara-solutions/mika",
            "1646",
        ]))
        .expect("detected");
        assert_eq!(a.number, "1646");
    }

    /// Detection is fail-open: anything not recognized as a close is none of
    /// the gate's business, so `gh` at large keeps working.
    #[test]
    fn non_close_commands_are_ignored() {
        for cmd in [
            argv(&["pr", "view", "1644", "--json", "files"]),
            argv(&["pr", "list", "--state", "open"]),
            argv(&["issue", "comment", "10", "--body", "hi"]),
            argv(&["pr", "merge", "12"]),
            argv(&["api", "repos/x/y"]),
        ] {
            assert!(
                detect_destructive_action(&cmd).is_none(),
                "should not detect: {cmd:?}"
            );
        }
    }

    #[test]
    fn close_without_a_number_is_not_detected() {
        assert!(detect_destructive_action(&argv(&["pr", "close"])).is_none());
    }

    #[test]
    fn grounding_satisfied_by_a_view_of_the_target() {
        let action = detect_destructive_action(&argv(&["pr", "close", "1644"])).expect("detected");
        let calls = vec![(
            "run_gh",
            r#"{"command":["pr","view","1644","--json","files"]}"#,
        )];
        assert!(destructive_grounding_satisfied(&action, calls.into_iter()));
    }

    /// The close call itself must not satisfy the guard that governs it.
    #[test]
    fn grounding_not_satisfied_by_the_close_call_itself() {
        let action = detect_destructive_action(&argv(&["pr", "close", "1644"])).expect("detected");
        let calls = vec![("run_gh", r#"{"command":["pr","close","1644"]}"#)];
        assert!(!destructive_grounding_satisfied(&action, calls.into_iter()));
    }

    /// Reading a *different* PR does not ground closing this one.
    #[test]
    fn grounding_not_satisfied_by_a_view_of_another_target() {
        let action = detect_destructive_action(&argv(&["pr", "close", "1644"])).expect("detected");
        let calls = vec![(
            "run_gh",
            r#"{"command":["pr","view","1638","--json","files"]}"#,
        )];
        assert!(!destructive_grounding_satisfied(&action, calls.into_iter()));
    }

    #[test]
    fn grounding_not_satisfied_by_an_empty_turn() {
        let action = detect_destructive_action(&argv(&["pr", "close", "1644"])).expect("detected");
        assert!(!destructive_grounding_satisfied(
            &action,
            std::iter::empty()
        ));
    }

    #[test]
    fn comment_evidence_requires_something_checkable() {
        assert!(destructive_comment_cites_evidence(Some(
            "File list shows crates/mika-agent/src/calibration/roles/mika_qa.rs — zero overlap."
        )));
        assert!(destructive_comment_cites_evidence(Some(
            "gh pr view --json files: no overlap with #1638"
        )));
        // The founding incident's actual comment: a paraphrase of an upstream
        // verdict, citing nothing checkable.
        assert!(!destructive_comment_cites_evidence(Some(
            "Closing as duplicate of mika#1638 (merged 2026-06-29T09:58Z). \
             QA review confirmed content is identical."
        )));
        assert!(!destructive_comment_cites_evidence(None));
    }

    #[test]
    fn repeat_acknowledgment_requires_naming_the_prior_action() {
        assert!(destructive_repeat_acknowledged(Some(
            "This PR was reopened after my earlier close; I reviewed the comments since then."
        )));
        assert!(destructive_repeat_acknowledged(Some(
            "Re-closing after reviewing the prior close."
        )));
        // The founding incident's second comment — byte-identical rationale to
        // the first, no acknowledgment that a first even happened.
        assert!(!destructive_repeat_acknowledged(Some(
            "Closing as duplicate of mika#1638 (merged 2026-06-29T09:58Z). \
             All content identical — calibration scenarios, fixtures, and \
             source changes already on main via PR #1638."
        )));
        assert!(!destructive_repeat_acknowledged(None));
    }

    /// `--delete-branch` takes no value. Treating it as value-taking swallowed
    /// the target number and silently disabled the gate.
    #[test]
    fn boolean_flag_does_not_swallow_the_target() {
        let a = detect_destructive_action(&argv(&["pr", "close", "--delete-branch", "1644"]))
            .expect("detected");
        assert_eq!(a.number, "1644");
    }

    /// #1644 must not be grounded by a read of #16440.
    #[test]
    fn grounding_not_satisfied_by_a_superstring_number() {
        let action = detect_destructive_action(&argv(&["pr", "close", "1644"])).expect("detected");
        let calls = vec![(
            "run_gh",
            r#"{"command":["pr","view","16440","--json","files"]}"#,
        )];
        assert!(!destructive_grounding_satisfied(&action, calls.into_iter()));
    }

    /// An administrative close with no diff to cite is still allowed — as long
    /// as it reports an observed state rather than paraphrasing a verdict.
    #[test]
    fn read_back_state_counts_as_evidence() {
        assert!(destructive_comment_cites_evidence(Some(
            "gh issue view: state: OPEN, labels: p3-nice-to-have — superseded, closing."
        )));
        assert!(destructive_comment_cites_evidence(Some(
            "PR already merged at 2026-06-29T09:58Z; branch deleted."
        )));
        // Still not enough: naming another ticket is the founding-incident shape.
        assert!(!destructive_comment_cites_evidence(Some(
            "Closing as duplicate of mika#1638."
        )));
    }

    #[test]
    fn window_parse_is_three_tier() {
        assert_eq!(parse_repeat_window(None), REPEAT_ACTION_WINDOW_DEFAULT_SECS);
        assert_eq!(
            parse_repeat_window(Some("  ")),
            REPEAT_ACTION_WINDOW_DEFAULT_SECS
        );
        assert_eq!(parse_repeat_window(Some("10")), 10);
        assert_eq!(parse_repeat_window(Some(" 45 ")), 45);
        assert_eq!(
            parse_repeat_window(Some("banana")),
            REPEAT_ACTION_WINDOW_DEFAULT_SECS
        );
        // A non-positive value must NOT disable the check — on a destructive
        // action an operator typo cannot be allowed to reopen the hole.
        assert_eq!(
            parse_repeat_window(Some("0")),
            REPEAT_ACTION_WINDOW_DEFAULT_SECS
        );
        assert_eq!(
            parse_repeat_window(Some("-1")),
            REPEAT_ACTION_WINDOW_DEFAULT_SECS
        );
    }
}
