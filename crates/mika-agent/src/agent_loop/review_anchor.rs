//! Review-anchor verification — the non-terminal half of the disposition guard family.
//!
//! mika#901 requires an F-list when the architect returns a *terminal* disposition
//! (`ITERATE` / `ESCALATE`). The non-terminal half (`READY` / `Verdict: GROOMED`) was
//! deliberately exempt: a plan with no objection needs no findings. That exemption is the
//! hole mika#2037 measured — the one disposition that advances the chain was the only one
//! requiring no evidence, so a 302-byte acknowledgement carrying the keyword forged an
//! architect attestation on a 10 KB brief it had not read.
//!
//! This module decides whether a response carries an attestation only a real review can
//! produce: **anchor lines that quote the brief verbatim, at distinct positions**.
//!
//! Why verbatim quotation and not the two alternatives considered:
//!
//! - *Answering the brief's numbered questions* depends on the brief being numbered. Free-text
//!   briefs are not, so the guard would be inert exactly where it is most needed.
//! - *A per-criterion verdict* is trivially satisfiable by a grid of "OK" with no content.
//!
//! Verbatim quotation is the only candidate the engine can check against a source it already
//! holds. Dispersion — non-overlapping regions — is what gives it bite: one quote is crossable
//! by copying the brief's first line; three quotes spread across a multi-kilobyte document are
//! not a by-product of an acknowledgement.
//!
//! The matcher is deliberately regex-free. mika#864 established the precedent for this family
//! ("regex is a footgun — silent failure to fire when pattern is malformed"), and a guard that
//! silently stops firing is the defect class being closed here, not an acceptable cost.

/// Why an attestation was rejected. Carried into the corrective re-prompt so the model is told
/// which condition it missed rather than the whole contract again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnchorMissReason {
    /// No line in the message body starts with a declared anchor prefix.
    NoAnchorLine,
    /// Anchor lines exist, but none carries a quote long enough to check.
    QuoteTooShort,
    /// Anchor lines carry long-enough text, but it does not appear in the brief.
    QuoteNotInBrief,
    /// Enough anchors quote the brief, but they land on the same region of it.
    OverlappingRegions,
    /// Anchors are individually valid but fewer than the declared minimum.
    TooFewAnchors,
}

impl AnchorMissReason {
    /// One-line description used in the corrective re-prompt.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::NoAnchorLine => {
                "the response body carries no anchor line at all — no line starts with a \
                 declared anchor prefix"
            }
            Self::QuoteTooShort => {
                "the anchor lines are too short to carry a checkable quote of the reviewed brief"
            }
            Self::QuoteNotInBrief => {
                "the anchor lines do not quote the reviewed brief verbatim — paraphrase does \
                 not satisfy the contract, the quoted span must appear in the brief exactly"
            }
            Self::OverlappingRegions => {
                "the anchor lines all quote the same region of the brief — each anchor must \
                 quote a distinct part of it"
            }
            Self::TooFewAnchors => "fewer valid anchor lines than the contract requires",
        }
    }
}

/// Outcome of verifying a response against the review-anchor contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnchorVerdict {
    /// The response carries a valid attestation.
    Satisfied,
    /// The response does not. `anchors_found` counts prefixed lines; `anchors_valid` counts
    /// those that survived every check.
    Missing {
        anchors_found: usize,
        anchors_valid: usize,
        reason: AnchorMissReason,
    },
}

impl AnchorVerdict {
    /// Test-only convenience — production code matches on `Missing` to read the reason.
    #[cfg(test)]
    pub(crate) fn is_satisfied(&self) -> bool {
        matches!(self, Self::Satisfied)
    }
}

/// Collapse every run of whitespace into a single space and trim the ends.
///
/// Applied to both the brief and each anchor line before comparison. A reviewer quoting a plan
/// re-flows the text they lift — the line break that sat mid-sentence in a 10 KB brief does not
/// survive into a one-line citation. Without this, a genuine quote fails on the reviewer's line
/// wrapping, which is a false rejection of exactly the response the guard must let through.
fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The message body, up to (exclusive of) the disposition-line landmark.
///
/// Same scan window as the mika#901 F-list guard, deliberately: the two guards must not
/// disagree about what "the body" is. An anchor placed after the disposition line does not
/// count — otherwise the contract could be satisfied below the line the operator reads.
fn body_lines<'a>(text: &'a str, required_suffix_lines: &[String]) -> Vec<&'a str> {
    let lines: Vec<&str> = text.lines().collect();
    let suffix_line_idx = lines.iter().position(|line| {
        let trimmed = line.trim();
        required_suffix_lines
            .iter()
            .any(|req| trimmed == req.as_str())
    });
    let scan_end = suffix_line_idx.unwrap_or(lines.len());
    lines[..scan_end].to_vec()
}

/// Find a span of at least `min_quote_chars` characters of `line` that occurs verbatim in
/// `brief`, returning its byte range within `brief`.
///
/// Slides a fixed `min_quote_chars`-character window over the line rather than searching for the
/// longest common substring: the contract asks whether a checkable quote exists, not how long
/// the longest one is, and the fixed window keeps the cost linear in the line length.
///
/// Character-indexed throughout (`Vec<char>`, never byte slicing of `&str`) — a multi-byte quote
/// is the ordinary case in this repo's French briefs, and byte slicing panics on it. See
/// `scripts/check-byte-slices.sh`.
fn find_brief_quote_range(
    line: &str,
    brief: &str,
    min_quote_chars: usize,
) -> Option<std::ops::Range<usize>> {
    if min_quote_chars == 0 {
        return None;
    }
    let chars: Vec<char> = line.chars().collect();
    if chars.len() < min_quote_chars {
        return None;
    }
    for start in 0..=(chars.len() - min_quote_chars) {
        let window: String = chars[start..start + min_quote_chars].iter().collect();
        if window.trim().is_empty() {
            continue;
        }
        if let Some(pos) = brief.find(&window) {
            return Some(pos..pos + window.len());
        }
    }
    None
}

/// Verify a response against a skill's review-anchor contract.
///
/// Returns `Satisfied` only when at least `min_count` anchor lines each quote at least
/// `min_quote_chars` characters of the brief verbatim, at mutually non-overlapping regions of it.
/// Every other outcome is a `Missing` carrying the reason — there is no third, permissive answer:
/// a response the engine cannot validate as a review is an absence of verdict, not an approval.
pub(crate) fn verify_review_anchors(
    text: &str,
    brief: &str,
    prefixes: &[String],
    required_suffix_lines: &[String],
    min_count: usize,
    min_quote_chars: usize,
) -> AnchorVerdict {
    let normalized_brief = normalize_whitespace(brief);

    // Keep the anchor's *content*, not its label. The declared prefix (`A1:`) is the marker
    // that identifies the line, never part of the quote — counting it toward
    // `min_quote_chars` would let a longer prefix buy a shorter quote.
    let anchor_lines: Vec<&str> = body_lines(text, required_suffix_lines)
        .into_iter()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            prefixes
                .iter()
                .find_map(|p| trimmed.strip_prefix(p.as_str()))
        })
        .collect();

    let anchors_found = anchor_lines.len();
    if anchors_found == 0 {
        return AnchorVerdict::Missing {
            anchors_found: 0,
            anchors_valid: 0,
            reason: AnchorMissReason::NoAnchorLine,
        };
    }

    // An empty brief can never ground a quote. Fail closed rather than vacuously pass —
    // a guard that approves when it has nothing to check against is the failure it prevents.
    if normalized_brief.is_empty() {
        return AnchorVerdict::Missing {
            anchors_found,
            anchors_valid: 0,
            reason: AnchorMissReason::QuoteNotInBrief,
        };
    }

    let mut too_short = 0usize;
    let mut not_in_brief = 0usize;
    let mut overlapping = 0usize;
    let mut claimed: Vec<std::ops::Range<usize>> = Vec::new();

    for line in &anchor_lines {
        let normalized_line = normalize_whitespace(line);
        if normalized_line.chars().count() < min_quote_chars {
            too_short += 1;
            continue;
        }
        match find_brief_quote_range(&normalized_line, &normalized_brief, min_quote_chars) {
            None => not_in_brief += 1,
            Some(range) => {
                // Greedy non-overlap: an anchor may not re-use a region an earlier anchor
                // already claimed. This is what makes three anchors mean three reads of the
                // brief rather than one line copied three times.
                if claimed
                    .iter()
                    .any(|c| range.start < c.end && c.start < range.end)
                {
                    overlapping += 1;
                } else {
                    claimed.push(range);
                }
            }
        }
    }

    let anchors_valid = claimed.len();
    if anchors_valid >= min_count {
        return AnchorVerdict::Satisfied;
    }

    // Report the reason that actually blocked the response. Ordered by how far the anchors got:
    // naming "too short" when the real problem is overlap would send the model the wrong fix.
    let reason = if overlapping > 0 {
        AnchorMissReason::OverlappingRegions
    } else if not_in_brief > 0 {
        AnchorMissReason::QuoteNotInBrief
    } else if too_short > 0 {
        AnchorMissReason::QuoteTooShort
    } else {
        AnchorMissReason::TooFewAnchors
    };

    AnchorVerdict::Missing {
        anchors_found,
        anchors_valid,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefixes() -> Vec<String> {
        (1..=10).map(|n| format!("A{n}:")).collect()
    }

    fn suffix_lines() -> Vec<String> {
        vec![
            "Disposition: READY".to_string(),
            "Disposition: ITERATE".to_string(),
            "Disposition: ESCALATE".to_string(),
            "Verdict: GROOMED".to_string(),
            "Verdict: ESCALATE".to_string(),
        ]
    }

    /// A brief with three clearly distinct regions, standing in for a real grooming brief.
    fn brief() -> String {
        "Le plan propose de re-résoudre le token avant chaque cycle du manager de milestones.\n\
         La question 2 porte sur l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ?\n\
         Je n'ai pas fixé N pour le seuil de détection des échecs d'authentification répétés.\n\
         La mise hors périmètre du code 403 doit être justifiée ou retirée du plan.\n"
            .to_string()
    }

    fn verify(text: &str) -> AnchorVerdict {
        verify_review_anchors(text, &brief(), &prefixes(), &suffix_lines(), 3, 40)
    }

    /// The measured response from mika#2037, verbatim.
    const STUB_2037: &str = "Préférence stockée — le pattern de re-résolution par cycle et le seuil N=3 pour mika#2013.\n\nDisposition: READY\n";

    #[test]
    fn stub_from_2037_is_rejected() {
        let verdict = verify(STUB_2037);
        assert_eq!(
            verdict,
            AnchorVerdict::Missing {
                anchors_found: 0,
                anchors_valid: 0,
                reason: AnchorMissReason::NoAnchorLine,
            },
            "the founding incident must not pass the guard built to catch it"
        );
    }

    #[test]
    fn three_distinct_quotes_are_satisfied() {
        let text = "A1: le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                    A2: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui, un INFO suffit.\n\
                    A3: « Je n'ai pas fixé N pour le seuil de détection des échecs » — je propose 30 minutes.\n\
                    \n\
                    Disposition: READY\n";
        assert_eq!(verify(text), AnchorVerdict::Satisfied);
    }

    #[test]
    fn three_quotes_of_the_same_region_are_rejected() {
        let line = "re-résoudre le token avant chaque cycle du manager de milestones";
        let text = format!("A1: {line}\nA2: {line}\nA3: {line}\n\nDisposition: READY\n");
        match verify(&text) {
            AnchorVerdict::Missing {
                anchors_valid,
                reason,
                ..
            } => {
                assert_eq!(anchors_valid, 1, "only the first claim on a region counts");
                assert_eq!(reason, AnchorMissReason::OverlappingRegions);
            }
            other => panic!("expected overlap rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_quote_absent_from_the_brief_is_rejected() {
        let text = "A1: le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                    A2: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui.\n\
                    A3: cette phrase n'apparaît nulle part dans le brief soumis à la revue architecte.\n\
                    \n\
                    Disposition: READY\n";
        match verify(text) {
            AnchorVerdict::Missing { reason, .. } => {
                assert_eq!(reason, AnchorMissReason::QuoteNotInBrief)
            }
            other => panic!("expected not-in-brief rejection, got {other:?}"),
        }
    }

    #[test]
    fn quote_length_boundary_is_inclusive_at_the_threshold() {
        // A span of the brief cut so neither end lands on whitespace — otherwise normalization
        // would trim it and the test would measure trimming rather than the threshold.
        let source = "Je n'ai pas fixé N pour le seuil de détection des échecs";
        let exact: String = source.chars().take(40).collect();
        let short: String = source.chars().take(39).collect();
        assert_eq!(exact.chars().count(), 40);
        assert!(!exact.ends_with(' ') && !short.ends_with(' '));

        let at_threshold = format!("A1: {exact}\n\nDisposition: READY\n");
        let below_threshold = format!("A1: {short}\n\nDisposition: READY\n");

        // min_count 1 isolates the length check from the count check.
        let at =
            verify_review_anchors(&at_threshold, &brief(), &prefixes(), &suffix_lines(), 1, 40);
        assert_eq!(at, AnchorVerdict::Satisfied, "40 chars must pass at n=40");

        let below = verify_review_anchors(
            &below_threshold,
            &brief(),
            &prefixes(),
            &suffix_lines(),
            1,
            40,
        );
        assert!(
            !below.is_satisfied(),
            "39 chars must fail at n=40, got {below:?}"
        );
    }

    #[test]
    fn two_valid_anchors_do_not_meet_a_minimum_of_three() {
        let text = "A1: le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                    A2: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui.\n\
                    \n\
                    Disposition: READY\n";
        match verify(text) {
            AnchorVerdict::Missing {
                anchors_found,
                anchors_valid,
                reason,
            } => {
                assert_eq!(anchors_found, 2);
                assert_eq!(anchors_valid, 2);
                assert_eq!(reason, AnchorMissReason::TooFewAnchors);
            }
            other => panic!("expected too-few rejection, got {other:?}"),
        }
    }

    #[test]
    fn anchors_after_the_disposition_line_do_not_count() {
        let text = "Rien à signaler.\n\
                    \n\
                    Disposition: READY\n\
                    A1: le plan propose bien de « re-résoudre le token avant chaque cycle du manager ».\n\
                    A2: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? ».\n\
                    A3: « Je n'ai pas fixé N pour le seuil de détection des échecs » — noté.\n";
        match verify(text) {
            AnchorVerdict::Missing { reason, .. } => {
                assert_eq!(reason, AnchorMissReason::NoAnchorLine)
            }
            other => panic!("expected body-scope rejection, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_brief_never_passes_vacuously() {
        let text = "A1: une ligne d'ancrage suffisamment longue pour franchir le seuil de citation.\n\
                    A2: une deuxième ligne d'ancrage suffisamment longue pour franchir ce seuil.\n\
                    A3: une troisième ligne d'ancrage suffisamment longue pour franchir ce seuil.\n\
                    \n\
                    Disposition: READY\n";
        let verdict = verify_review_anchors(text, "", &prefixes(), &suffix_lines(), 3, 40);
        assert!(!verdict.is_satisfied(), "empty brief must fail closed");
    }

    #[test]
    fn line_wrapping_in_the_brief_does_not_break_a_genuine_quote() {
        // The brief holds the sentence across a line break; the reviewer quotes it on one line.
        let wrapped_brief =
            "Le plan propose de re-résoudre\nle token avant chaque cycle du manager de milestones.";
        let text = "A1: le plan propose de re-résoudre le token avant chaque cycle du manager\n\n\
                    Disposition: READY\n";
        let verdict =
            verify_review_anchors(text, wrapped_brief, &prefixes(), &suffix_lines(), 1, 40);
        assert_eq!(
            verdict,
            AnchorVerdict::Satisfied,
            "a reviewer re-flows what they quote; the guard must not reject that"
        );
    }

    #[test]
    fn multibyte_quotes_do_not_panic() {
        // Accented French and emoji at the window boundary — byte slicing would panic here.
        let unicode_brief = "Décision — la référence « éàüñ 🔒 » doit être conservée telle quelle dans le rapport final.";
        let text = "A1: la référence « éàüñ 🔒 » doit être conservée telle quelle dans le rapport\n\n\
                    Disposition: READY\n";
        let verdict =
            verify_review_anchors(text, unicode_brief, &prefixes(), &suffix_lines(), 1, 40);
        assert!(verdict.is_satisfied(), "got {verdict:?}");
    }

    #[test]
    fn whitespace_only_windows_are_not_quotes() {
        // A line of padding must not satisfy the contract by matching a run of spaces.
        let padded_brief =
            "début                                                                    fin";
        let text = "A1:                                                                          \n\n\
                    Disposition: READY\n";
        let verdict =
            verify_review_anchors(text, padded_brief, &prefixes(), &suffix_lines(), 1, 40);
        assert!(!verdict.is_satisfied(), "got {verdict:?}");
    }
}
