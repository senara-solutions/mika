use regex::Regex;
use std::sync::LazyLock;
use tracing::info;

/// Parsed PR review event from gateway-formatted text.
#[derive(Debug, Clone)]
pub(crate) struct PrReviewEvent {
    pub state: String,
    pub repo: String,
    pub pr_number: u64,
    #[allow(dead_code)] // Parsed for completeness; used in Debug output and tests
    pub title: String,
    pub reviewer: String,
    pub review_url: String,
    pub body: String,
}

/// Review depth parsed from a verdict body (mika#275).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ReviewDepth {
    CodeLevel,
    CodeLevelPartial,
    MetadataOnly,
    /// DEPTH line was present but had an unrecognized value.
    Unknown(String),
}

impl PrReviewEvent {
    /// Construct the PR HTML URL from repo and number.
    pub fn pr_url(&self) -> String {
        format!("https://github.com/{}/pull/{}", self.repo, self.pr_number)
    }
}

/// Parsed verdict from a review body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Verdict {
    Pass,
    Block(String),
    Hold(String),
    Missing { truncated: bool },
}

static HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[GitHub\] PR review \(([^)]+)\) on ([^#]+)#(\d+) \((.+)\) by @(\S+)$")
        .expect("header regex")
});

// mika#1828: VERDICT_RE tolerates markdown-emphasis wrappers observed in the
// wild (`**Verdict: REQUEST CHANGES**` on PR #1821, also `*Verdict:*`, `__X__`).
// Permissive leading emphasis run (any mix of `*`/`_`); captures the rest of
// the line. Trailing-emphasis peel + trailing-content truncation happens in
// `parse_verdict` — the regex just gets us to the value.
static VERDICT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?mi)^\s*[*_]*\s*VERDICT:\s*(.+)$").expect("verdict regex"));

static DEPTH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?mi)^DEPTH:\s*(.+)$").expect("depth regex"));

static BLOCK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^block\[([^\]]+)\]$").expect("block regex"));

static HOLD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^hold\[([^\]]+)\]$").expect("hold regex"));

/// Parse a gateway-formatted PR review event into structured data.
///
/// Returns `None` if the text does not match the expected format.
pub(crate) fn parse_pr_review_event(text: &str) -> Option<PrReviewEvent> {
    let mut lines = text.lines();

    let first_line = lines.next()?;
    let caps = HEADER_RE.captures(first_line)?;

    let state = caps[1].to_string();
    let repo = caps[2].to_string();
    let pr_number: u64 = caps[3].parse().ok()?;
    let title = caps[4].to_string();
    let reviewer = caps[5].to_string();

    let review_url = lines
        .next()
        .map(|l| l.trim().to_string())
        .unwrap_or_default();

    // Body is everything after the first blank line
    let body = {
        // Skip until we find a blank line
        let mut found_blank = false;
        let mut body_lines = Vec::new();
        for line in lines {
            if !found_blank {
                if line.trim().is_empty() {
                    found_blank = true;
                }
            } else {
                body_lines.push(line);
            }
        }
        body_lines.join("\n")
    };

    Some(PrReviewEvent {
        state,
        repo,
        pr_number,
        title,
        reviewer,
        review_url,
        body,
    })
}

/// Strip a leading/trailing run of `*`/`_` characters (defense-in-depth for the
/// single-emphasis `*Verdict:*` and stray-asterisk shapes that VERDICT_RE's
/// double-emphasis alternation misses). mika#1828.
fn strip_md_emphasis(value: &str) -> &str {
    value.trim_matches(['*', '_'])
}

/// Normalize an alias-candidate value: lowercase, collapse `[_\-\s]+` runs to a
/// single space, then trim. mika#1828 AC2. Maps `REQUEST_CHANGES`,
/// `changes-requested`, `Request  Changes` all to `request changes`.
fn normalize_alias(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut in_sep = false;
    for c in lower.chars() {
        if c == '_' || c == '-' || c.is_whitespace() {
            if !in_sep && !out.is_empty() {
                out.push(' ');
            }
            in_sep = true;
        } else {
            out.push(c);
            in_sep = false;
        }
    }
    // Trailing separator run left a trailing space.
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Map a normalized alias to a canonical Verdict. Returns `None` for
/// unrecognized values (which fall through to `Verdict::Missing`).
///
/// mika#1828 AC2 alias table. Default-safer target for CHANGES aliases is
/// `Block("ac")` (AC-fix path is retriable; security/pipeline would misroute).
fn alias_to_verdict(normalized: &str) -> Option<Verdict> {
    match normalized {
        "request changes" | "request change" | "changes requested" => {
            Some(Verdict::Block("ac".to_string()))
        }
        "approve" | "approved" => Some(Verdict::Pass),
        _ => None,
    }
}

/// Classify a normalized verdict value into a canonical `Verdict`.
///
/// The cascade is the mika#1821/#1828 pipeline, unchanged: exact canonical
/// `pass` → `BLOCK_RE` → `HOLD_RE` → alias table. Returns `None` for values
/// this cascade does not recognize.
fn classify_value(value: &str) -> Option<Verdict> {
    if value.eq_ignore_ascii_case("pass") {
        return Some(Verdict::Pass);
    }

    if let Some(bcaps) = BLOCK_RE.captures(value) {
        return Some(Verdict::Block(bcaps[1].to_string()));
    }

    if let Some(hcaps) = HOLD_RE.captures(value) {
        return Some(Verdict::Hold(hcaps[1].to_string()));
    }

    // mika#1828 AC2: alias fallback. GitHub-review-state-adjacent tokens
    // (`REQUEST CHANGES`, `REQUEST_CHANGES`, `CHANGES_REQUESTED`, `APPROVE`,
    // `APPROVED`) map to canonical Verdicts. Runs after the exact
    // canonical checks so a legitimate `block[ac]` is never rewritten.
    let normalized = normalize_alias(value);
    if let Some(mapped) = alias_to_verdict(&normalized) {
        info!(
            event = "verdict_alias_normalized",
            raw_value = value,
            normalized = normalized.as_str(),
            mapped_to = ?mapped,
            "verdict: normalized non-canonical alias to canonical verdict (mika#1828)"
        );
        return Some(mapped);
    }

    None
}

/// Retire un suffixe décoratif d'une valeur de verdict (mika#2239).
///
/// Les reviewers décorent : `pass ✅`, `block[ac] ❌`, `hold[review] ⏸️`. La décoration
/// est toujours une queue de caractères non-alphanumériques.
///
/// L'ensemble d'exemption est DÉRIVÉ DE LA GRAMMAIRE, pas choisi : `]` est le seul
/// caractère non-alphanumérique porteur de signal, parce qu'il termine les deux seules
/// formes canoniques à bracket — `block[…]` (`BLOCK_RE`) et `hold[…]` (`HOLD_RE`), toutes
/// deux ancrées `^…$`. Toute nouvelle forme de verdict à bracket DOIT étendre la regex ET
/// cette exemption dans le même changement, sinon le retrait la mutile en silence.
/// N'exemptez PAS `)`, `}`, `>` : aucune grammaire derrière, et exempter un caractère
/// AFFAIBLIT le retrait (`pass 🎉)` s'arrêterait sur le `)` et resterait `Missing`).
///
/// Volontairement conservateur — la queue s'arrête au premier alphanumérique ASCII, donc
/// un vrai commentaire de fin (`pass — but see findings`) n'est PAS avalé et continue de
/// classer `Missing`. C'est la borne de mika#1821, inchangée.
///
/// La décoration de TÊTE (`VERDICT: ✅ pass`) est hors périmètre (mika#2239 D-D) : jamais
/// mesurée. Sa condition de réveil est un `verdict_approved_but_unclassified` dont le
/// champ `verdict_value` ne commence pas par un alphanumérique ASCII.
fn strip_trailing_decoration(value: &str) -> &str {
    value.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != ']')
}

/// La valeur brute de la ligne `VERDICT:` si une telle ligne existe (mika#2239).
///
/// `None` ⇔ aucune ligne `VERDICT:` dans le corps. Sert à distinguer, côté handler,
/// « pas de ligne » de « ligne présente, valeur non reconnue » — la distinction que
/// `Verdict::Missing` ne porte pas, et dont l'absence a produit un diagnostic faux
/// mesuré sur mika#2236.
pub(crate) fn verdict_raw_value(body: &str) -> Option<String> {
    VERDICT_RE
        .captures(body)
        .map(|caps| caps[1].trim().to_string())
}

/// Parse a verdict from a review body.
pub(crate) fn parse_verdict(body: &str) -> Verdict {
    if let Some(caps) = VERDICT_RE.captures(body) {
        // The regex captured to end-of-line. Two-step normalization
        // (mika#1828 D1):
        // 1. Strip leading emphasis (`**`, `__`, `*`, `_` runs) — handles
        //    `**block[ac]` (unbalanced) and `*value*` (single-emphasis).
        // 2. Then find the FIRST trailing `**`/`__` in the remaining string
        //    and truncate at it — handles `X** — trailing comment` shape from
        //    PR #1821. Truncating first would over-eat `**value` cases.
        let raw = caps[1].trim();
        let stripped_leading = raw.trim_start_matches(['*', '_']);
        let truncated_at_close = stripped_leading
            .find("**")
            .or_else(|| stripped_leading.find("__"))
            .map(|pos| &stripped_leading[..pos])
            .unwrap_or(stripped_leading);
        let value = strip_md_emphasis(truncated_at_close.trim()).trim();

        // Passe primaire — pipeline mika#1821/#1828 inchangé.
        if let Some(v) = classify_value(value) {
            return v;
        }

        // mika#2239 : repli décoration. Atteint uniquement quand la passe primaire a
        // déjà échoué → aucune forme actuellement reconnue ne change de sens.
        let undecorated = strip_trailing_decoration(value).trim_end();
        if undecorated != value
            && let Some(v) = classify_value(undecorated)
        {
            info!(
                event = "verdict_decoration_stripped",
                raw_value = value,
                undecorated = undecorated,
                mapped_to = ?v,
                "verdict: suffixe décoratif retiré avant classification (mika#2239)"
            );
            return v;
        }

        // Unrecognized verdict value
        return Verdict::Missing {
            truncated: body.contains("[truncated]"),
        };
    }

    Verdict::Missing {
        truncated: body.contains("[truncated]"),
    }
}

/// Parse review depth from a verdict body (mika#275).
///
/// Returns `None` if no `DEPTH:` line is present (backward compat).
pub(crate) fn parse_review_depth(body: &str) -> Option<ReviewDepth> {
    let caps = DEPTH_RE.captures(body)?;
    let value = caps[1].trim();
    Some(match value.to_lowercase().as_str() {
        "code-level" => ReviewDepth::CodeLevel,
        "code-level (partial)" => ReviewDepth::CodeLevelPartial,
        "metadata-only" => ReviewDepth::MetadataOnly,
        _ => ReviewDepth::Unknown(value.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_APPROVED: &str = "\
[GitHub] PR review (approved) on senara-solutions/mika#522 (fix: something) by @mika-qa
https://github.com/senara-solutions/mika/pull/522#pullrequestreview-12345

VERDICT: pass

Some review body text here...";

    #[test]
    fn parse_pr_review_approved_with_verdict_pass() {
        let event = parse_pr_review_event(SAMPLE_APPROVED).unwrap();
        assert_eq!(event.state, "approved");
        assert_eq!(event.repo, "senara-solutions/mika");
        assert_eq!(event.pr_number, 522);
        assert_eq!(event.title, "fix: something");
        assert_eq!(event.reviewer, "mika-qa");
        assert_eq!(
            event.review_url,
            "https://github.com/senara-solutions/mika/pull/522#pullrequestreview-12345"
        );
        assert_eq!(
            event.pr_url(),
            "https://github.com/senara-solutions/mika/pull/522"
        );
        assert!(event.body.contains("VERDICT: pass"));
        assert!(event.body.contains("Some review body text here..."));

        let verdict = parse_verdict(&event.body);
        assert_eq!(verdict, Verdict::Pass);
    }

    #[test]
    fn parse_pr_review_block_ci() {
        let body = "VERDICT: block[ci]\n\nCI checks are failing.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Block("ci".to_string()));
    }

    #[test]
    fn parse_pr_review_hold_review() {
        let body = "VERDICT: hold[review]\n\nNeeds another look.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Hold("review".to_string()));
    }

    #[test]
    fn parse_verdict_missing() {
        let body = "No verdict line here, just comments.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Missing { truncated: false });
    }

    #[test]
    fn parse_verdict_case_insensitive() {
        let body = "verdict: PASS";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Pass);

        let body2 = "VERDICT: Block[Security]";
        let verdict2 = parse_verdict(body2);
        assert_eq!(verdict2, Verdict::Block("Security".to_string()));

        let body3 = "VERDICT: HOLD[Review]";
        let verdict3 = parse_verdict(body3);
        assert_eq!(verdict3, Verdict::Hold("Review".to_string()));
    }

    #[test]
    fn parse_verdict_no_space_after_colon() {
        let body = "VERDICT:pass";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Pass);
    }

    #[test]
    fn parse_verdict_multiple_lines_first_wins() {
        let body = "VERDICT: pass\nVERDICT: block[ci]";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Pass);
    }

    #[test]
    fn parse_non_review_event_returns_none() {
        let text = "This is just a regular message, not a PR review event.";
        assert!(parse_pr_review_event(text).is_none());
    }

    #[test]
    fn parse_review_changes_requested() {
        let text = "\
[GitHub] PR review (changes_requested) on senara-solutions/mika#100 (feat: add feature) by @reviewer
https://github.com/senara-solutions/mika/pull/100#pullrequestreview-99999

VERDICT: block[pipeline]

Please fix the pipeline issues.";

        let event = parse_pr_review_event(text).unwrap();
        assert_eq!(event.state, "changes_requested");
        assert_eq!(event.pr_number, 100);
        assert_eq!(event.reviewer, "reviewer");

        let verdict = parse_verdict(&event.body);
        assert_eq!(verdict, Verdict::Block("pipeline".to_string()));
    }

    #[test]
    fn parse_verdict_truncated_body_missing() {
        let body = "Some text...\n[truncated]";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Missing { truncated: true });
    }

    #[test]
    fn parse_malformed_first_line() {
        let text = "[GitHub] PR review on something broken";
        assert!(parse_pr_review_event(text).is_none());

        let text2 = "[GitHub] PR review (approved) on repo#notanumber (title) by @user";
        assert!(parse_pr_review_event(text2).is_none());
    }

    // --- Review depth parsing tests (mika#275) ---

    #[test]
    fn parse_depth_code_level() {
        let body = "VERDICT: pass\nDEPTH: code-level\nREASON: all good";
        assert_eq!(parse_review_depth(body), Some(ReviewDepth::CodeLevel));
    }

    #[test]
    fn parse_depth_code_level_partial() {
        let body = "VERDICT: pass\nDEPTH: code-level (partial)\nREASON: truncated diff";
        assert_eq!(
            parse_review_depth(body),
            Some(ReviewDepth::CodeLevelPartial)
        );
    }

    #[test]
    fn parse_depth_metadata_only() {
        let body = "VERDICT: hold[review]\nDEPTH: metadata-only\nREASON: diff unavailable";
        assert_eq!(parse_review_depth(body), Some(ReviewDepth::MetadataOnly));
    }

    #[test]
    fn parse_depth_missing_returns_none() {
        let body = "VERDICT: pass\nREASON: all good";
        assert_eq!(parse_review_depth(body), None);
    }

    #[test]
    fn parse_depth_case_insensitive() {
        let body = "DEPTH: Code-Level";
        assert_eq!(parse_review_depth(body), Some(ReviewDepth::CodeLevel));

        let body2 = "DEPTH: METADATA-ONLY";
        assert_eq!(parse_review_depth(body2), Some(ReviewDepth::MetadataOnly));
    }

    #[test]
    fn parse_depth_unknown_value() {
        let body = "DEPTH: something-else";
        assert_eq!(
            parse_review_depth(body),
            Some(ReviewDepth::Unknown("something-else".to_string()))
        );
    }

    #[test]
    fn parse_depth_first_match_wins() {
        let body = "DEPTH: code-level\nDEPTH: metadata-only";
        assert_eq!(parse_review_depth(body), Some(ReviewDepth::CodeLevel));
    }

    // --- mika#1828: markdown-bold + alias tolerance tests -------------------

    #[test]
    fn parse_verdict_markdown_bold_pass() {
        assert_eq!(parse_verdict("**VERDICT: pass**"), Verdict::Pass);
    }

    #[test]
    fn parse_verdict_markdown_bold_all_canonical_classes() {
        assert_eq!(
            parse_verdict("**Verdict: block[ac]**"),
            Verdict::Block("ac".to_string())
        );
        assert_eq!(
            parse_verdict("**VERDICT: block[ci]**"),
            Verdict::Block("ci".to_string())
        );
        assert_eq!(
            parse_verdict("**VERDICT: block[security]**"),
            Verdict::Block("security".to_string())
        );
        assert_eq!(
            parse_verdict("**VERDICT: block[pipeline]**"),
            Verdict::Block("pipeline".to_string())
        );
        assert_eq!(
            parse_verdict("**VERDICT: hold[review]**"),
            Verdict::Hold("review".to_string())
        );
    }

    #[test]
    fn parse_verdict_underscore_bold() {
        assert_eq!(
            parse_verdict("__VERDICT: block[ci]__"),
            Verdict::Block("ci".to_string())
        );
    }

    #[test]
    fn parse_verdict_single_emphasis_strip() {
        // Single `*` isn't in the regex alternation, but `strip_md_emphasis`
        // peels it as defense-in-depth.
        assert_eq!(parse_verdict("*VERDICT: pass*"), Verdict::Pass);
    }

    #[test]
    fn parse_verdict_alias_request_changes_space() {
        assert_eq!(
            parse_verdict("VERDICT: REQUEST CHANGES"),
            Verdict::Block("ac".to_string())
        );
    }

    #[test]
    fn parse_verdict_alias_request_changes_underscore() {
        assert_eq!(
            parse_verdict("VERDICT: REQUEST_CHANGES"),
            Verdict::Block("ac".to_string())
        );
    }

    #[test]
    fn parse_verdict_alias_changes_requested() {
        assert_eq!(
            parse_verdict("VERDICT: CHANGES_REQUESTED"),
            Verdict::Block("ac".to_string())
        );
    }

    #[test]
    fn parse_verdict_alias_hyphen_lowercase() {
        assert_eq!(
            parse_verdict("VERDICT: changes-requested"),
            Verdict::Block("ac".to_string())
        );
    }

    #[test]
    fn parse_verdict_alias_approve() {
        assert_eq!(parse_verdict("VERDICT: APPROVE"), Verdict::Pass);
    }

    #[test]
    fn parse_verdict_alias_approved_lowercase() {
        assert_eq!(parse_verdict("VERDICT: approved"), Verdict::Pass);
    }

    /// mika#1828 founding regression — PR mika#1821 review body shape
    /// (`**Verdict: REQUEST CHANGES**`). Both deviations (markdown-bold wrapper
    /// AND non-canonical class token) combined. Must map to Block("ac") so the
    /// handler dispatches the AC-fix path instead of `hold[review]` silent-drop.
    #[test]
    fn parse_verdict_pr1821_combined_shape() {
        let body = "## QA Review — mika#1821\n\n\
                    **Verdict: REQUEST CHANGES** — one blocking finding.";
        assert_eq!(parse_verdict(body), Verdict::Block("ac".to_string()));
    }

    #[test]
    fn parse_verdict_regression_no_emphasis_still_parses() {
        // Regression guard for the non-greedy capture change — the plain
        // `VERDICT: block[ac]` form must still parse (no emphasis to peel).
        assert_eq!(
            parse_verdict("VERDICT: block[ac]"),
            Verdict::Block("ac".to_string())
        );
    }

    #[test]
    fn parse_verdict_regression_frobnicate_still_missing() {
        // Genuinely-unrecognized values (not in canonical set, not in alias
        // table) must still return Missing — the alias fallback must not be a
        // catch-all.
        assert_eq!(
            parse_verdict("VERDICT: frobnicate"),
            Verdict::Missing { truncated: false }
        );
    }

    #[test]
    fn parse_verdict_unbalanced_emphasis_strips_gracefully() {
        // `**block[ac]` (unbalanced opening emphasis, no closing). The strip
        // helper peels the leading `**`, parser sees `block[ac]`, canonical
        // block form matches.
        assert_eq!(
            parse_verdict("VERDICT: **block[ac]"),
            Verdict::Block("ac".to_string())
        );
    }

    // --- mika#2239: trailing-decoration tolerance -------------------------

    #[test]
    fn parse_verdict_emoji_suffix_pass() {
        assert_eq!(parse_verdict("VERDICT: pass ✅"), Verdict::Pass);
    }

    #[test]
    fn parse_verdict_emoji_suffix_block() {
        assert_eq!(
            parse_verdict("VERDICT: block[ac] ❌"),
            Verdict::Block("ac".to_string())
        );
    }

    #[test]
    fn parse_verdict_emoji_suffix_hold() {
        assert_eq!(
            parse_verdict("VERDICT: hold[review] ⏸️"),
            Verdict::Hold("review".to_string())
        );
    }

    #[test]
    fn parse_verdict_emoji_suffix_alias() {
        assert_eq!(parse_verdict("VERDICT: approved ✅"), Verdict::Pass);
    }

    #[test]
    fn parse_verdict_bold_plus_emoji() {
        // Cumul mika#1828 (emphase) + mika#2239 (décoration).
        assert_eq!(parse_verdict("**VERDICT: pass ✅**"), Verdict::Pass);
    }

    /// mika#2239 founding regression — the review-body shape `mika-platform-qa`
    /// emitted on all four of its reviews of PR mika#2236 (three APPROVED at
    /// 08:25:08Z / 08:47:25Z / 09:29:39Z, plus a COMMENTED at 08:03:23Z), each
    /// classifying as `Missing` and so never arming the autonomous merge.
    ///
    /// The field shape (line 1 `VERDICT:`, then `DEPTH:`, then `REASON:`) is
    /// the one measured on the PR and recorded in the ticket; the REASON prose
    /// is illustrative — only the VERDICT line's decoration is under test.
    #[test]
    fn parse_verdict_field_shape_pr2236() {
        let body = "VERDICT: pass ✅\n\
                    DEPTH: code-level\n\
                    REASON: all acceptance criteria satisfied, CI clean.";
        assert_eq!(parse_verdict(body), Verdict::Pass);
    }

    #[test]
    fn parse_verdict_trailing_comment_still_missing() {
        // Un commentaire de fin n'est PAS une décoration — la queue s'arrête au
        // premier alphanumérique ASCII. Borne mika#1821, inchangée.
        assert_eq!(
            parse_verdict("VERDICT: pass — but see findings below"),
            Verdict::Missing { truncated: false }
        );
    }

    #[test]
    fn parse_verdict_unknown_token_with_emoji_still_missing() {
        // Un jeton inconnu décoré reste inconnu.
        assert_eq!(
            parse_verdict("VERDICT: frobnicate ✅"),
            Verdict::Missing { truncated: false }
        );
    }

    #[test]
    fn strip_trailing_decoration_edge_cases() {
        assert_eq!(strip_trailing_decoration("pass ✅").trim_end(), "pass");
        assert_eq!(
            strip_trailing_decoration("block[ac] ❌").trim_end(),
            "block[ac]"
        );
        assert_eq!(
            strip_trailing_decoration("hold[review] ⏸️").trim_end(),
            "hold[review]"
        );
        // No-ops — rien à retirer.
        assert_eq!(strip_trailing_decoration("pass"), "pass");
        // La garde `]` : le bracket fermant porte du signal, il ne part pas.
        assert_eq!(strip_trailing_decoration("block[ac]"), "block[ac]");
        assert_eq!(strip_trailing_decoration(""), "");
    }

    #[test]
    fn verdict_raw_value_reports_the_line_when_present() {
        assert_eq!(
            verdict_raw_value("VERDICT: pass ✅\nDEPTH: code-level"),
            Some("pass ✅".to_string())
        );
        // Aucune ligne VERDICT: → None. C'est la distinction que
        // `Verdict::Missing` ne porte pas.
        assert_eq!(
            verdict_raw_value("No verdict line here, just comments."),
            None
        );
    }

    #[test]
    fn normalize_alias_handles_all_separators() {
        assert_eq!(normalize_alias("REQUEST_CHANGES"), "request changes");
        assert_eq!(normalize_alias("changes-requested"), "changes requested");
        assert_eq!(normalize_alias("  Request  Changes  "), "request changes");
        assert_eq!(normalize_alias("APPROVE"), "approve");
    }

    #[test]
    fn strip_md_emphasis_edge_cases() {
        assert_eq!(strip_md_emphasis("**bold**"), "bold");
        assert_eq!(strip_md_emphasis("__underscored__"), "underscored");
        assert_eq!(strip_md_emphasis("***triple***"), "triple");
        assert_eq!(strip_md_emphasis("no emphasis"), "no emphasis");
        assert_eq!(strip_md_emphasis(""), "");
        assert_eq!(strip_md_emphasis("*single*"), "single");
    }
}
