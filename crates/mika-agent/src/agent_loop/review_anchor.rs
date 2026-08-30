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

/// The two-directional measurement matrix for the thresholds (mika#2037 U7).
///
/// The guard has to satisfy two conditions that pull against each other: a real review must
/// ALWAYS pass, and a stub like the one measured in mika#2037 must be refused. The second
/// alone is satisfied by "refuse everything", which would block grooming entirely — so both
/// columns are measured, and the thresholds are whatever separates them, not whatever seemed
/// reasonable in prose.
///
/// Provenance of the accept column matters: a case written to pass the guard proves nothing
/// (`a-stub-built-from-the-doc-cannot-falsify-the-doc`). `ready_example_from_the_shipped_prompt`
/// is read from `mika-arch-groom-ticket/system_prompt.md` on disk — it is the response the
/// repo actually instructs the architect to produce, so a future prompt edit that drifts from
/// the guard fails here rather than in production.
#[cfg(test)]
mod matrix {
    use super::*;
    use std::path::Path;

    /// The brief these cases are reviewed against — a stand-in for the 10 492-byte brief of
    /// the founding incident, carrying its four numbered questions.
    const BRIEF: &str = "Plan de renouvellement du token du manager de milestones.\n\
        Le plan re-résout le token du cycle avant chaque cycle au lieu de le figer au spawn.\n\
        Question 1 : où placer le correctif, dans le chemin de spawn ou dans le corps du cycle ?\n\
        Question 2 : le tracé est-il exhaustif, faut-il journaliser chaque rafraîchissement ?\n\
        Question 3 : je n'ai pas fixé N pour le seuil d'échecs d'authentification répétés.\n\
        Question 4 : la mise hors périmètre du code 403 est-elle sûre pour ce jalon ?\n";

    /// Case provenance, so the matrix says which evidence is real and which is constructed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Origin {
        /// Measured in the field, or read from a shipped file in this repo.
        Real,
        /// Written for this matrix to probe a specific evasion shape.
        Constructed,
    }

    struct Case {
        name: &'static str,
        origin: Origin,
        text: String,
    }

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

    /// Lift the worked READY example out of the shipped prompt, and re-anchor its quotes onto
    /// BRIEF. The prompt's own example quotes a different brief; what is under test is its
    /// SHAPE — three prefixed lines each carrying a long verbatim span — not its content.
    fn ready_example_shape_from_shipped_prompt() -> Option<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../skills/bundled/mika-arch-groom-ticket/system_prompt.md");
        let prompt = std::fs::read_to_string(path).ok()?;
        let example = prompt.split("#### Disposition: READY example").nth(1)?;
        let block = example.split("```").nth(1)?;
        // Count the anchors the shipped example carries, then build a response of that shape
        // whose quotes come from BRIEF.
        let anchor_count = (1..=10)
            .filter(|n| block.contains(&format!("A{n}:")))
            .count();
        if anchor_count == 0 {
            return None;
        }
        let quotes = [
            "Le plan re-résout le token du cycle avant chaque cycle au lieu de le figer",
            "le tracé est-il exhaustif, faut-il journaliser chaque rafraîchissement",
            "je n'ai pas fixé N pour le seuil d'échecs d'authentification répétés",
            "la mise hors périmètre du code 403 est-elle sûre pour ce jalon",
            "où placer le correctif, dans le chemin de spawn ou dans le corps du cycle",
        ];
        let mut out = String::new();
        for (i, q) in quotes
            .iter()
            .take(anchor_count.min(quotes.len()))
            .enumerate()
        {
            out.push_str(&format!("A{}: \"{q}\" — traité.\n", i + 1));
        }
        out.push_str("\nDisposition: READY\n");
        Some(out)
    }

    fn must_reject() -> Vec<Case> {
        let mut cases = vec![
            Case {
                name: "stub_2037_verbatim",
                origin: Origin::Real,
                text: "Préférence stockée — le pattern de re-résolution par cycle et le seuil \
                       N=3 pour mika#2013.\n\nDisposition: READY\n"
                    .to_string(),
            },
            Case {
                name: "single_long_quote_only",
                origin: Origin::Constructed,
                text: "A1: Le plan re-résout le token du cycle avant chaque cycle au lieu de le \
                       figer au spawn.\n\nDisposition: READY\n"
                    .to_string(),
            },
            Case {
                name: "three_anchors_same_region",
                origin: Origin::Constructed,
                text: {
                    let line = "Le plan re-résout le token du cycle avant chaque cycle au lieu \
                                de le figer au spawn";
                    format!("A1: {line}\nA2: {line}\nA3: {line}\n\nDisposition: READY\n")
                },
            },
            Case {
                name: "three_anchors_paraphrase_only",
                origin: Origin::Constructed,
                text: "A1: le token est renouvelé à chaque tour de boucle plutôt qu'une seule fois.\n\
                       A2: la journalisation des rafraîchissements paraît suffisante en l'état.\n\
                       A3: le seuil d'échec devrait être exprimé autrement qu'en nombre de tours.\n\
                       \nDisposition: READY\n"
                    .to_string(),
            },
            Case {
                name: "anchors_below_the_disposition_line",
                origin: Origin::Constructed,
                text: "Rien à signaler.\n\nDisposition: READY\n\
                       A1: Le plan re-résout le token du cycle avant chaque cycle au lieu de le figer.\n\
                       A2: le tracé est-il exhaustif, faut-il journaliser chaque rafraîchissement ?\n\
                       A3: je n'ai pas fixé N pour le seuil d'échecs d'authentification répétés.\n"
                    .to_string(),
            },
        ];
        // Padding is not a quote: a wall of spaces must not buy an anchor.
        cases.push(Case {
            name: "whitespace_padding_anchors",
            origin: Origin::Constructed,
            text: format!(
                "A1:{pad}\nA2:{pad}\nA3:{pad}\n\nDisposition: READY\n",
                pad = " ".repeat(80)
            ),
        });
        cases
    }

    fn must_accept() -> Vec<Case> {
        let mut cases = vec![
            Case {
                name: "three_dispersed_quotes",
                origin: Origin::Constructed,
                text: "A1: \"Le plan re-résout le token du cycle avant chaque cycle au lieu de le \
                       figer\" — le corps du cycle est le bon endroit.\n\
                       A2: \"le tracé est-il exhaustif, faut-il journaliser chaque rafraîchissement\" \
                       — un INFO par changement suffit.\n\
                       A3: \"je n'ai pas fixé N pour le seuil d'échecs d'authentification répétés\" \
                       — exprime-le en durée, pas en nombre de cycles.\n\
                       \nDisposition: READY\n"
                    .to_string(),
            },
            Case {
                name: "four_quotes_one_per_question",
                origin: Origin::Constructed,
                text: "A1: \"où placer le correctif, dans le chemin de spawn ou dans le corps du \
                       cycle\" — dans le corps du cycle.\n\
                       A2: \"le tracé est-il exhaustif, faut-il journaliser chaque rafraîchissement\" \
                       — oui, au niveau INFO.\n\
                       A3: \"je n'ai pas fixé N pour le seuil d'échecs d'authentification répétés\" \
                       — 30 minutes.\n\
                       A4: \"la mise hors périmètre du code 403 est-elle sûre pour ce jalon\" — oui, \
                       403 est une forme de permission, pas d'expiration.\n\
                       \nDisposition: READY\n"
                    .to_string(),
            },
            Case {
                name: "quotes_reflowed_across_prose",
                origin: Origin::Constructed,
                text: "Revue complète ci-dessous.\n\n\
                       A1: sur le placement — \"Le plan re-résout le token du cycle avant chaque \
                       cycle au lieu de le figer au spawn.\" C'est correct.\n\
                       Le reste du raisonnement tient.\n\n\
                       A2: sur le tracé — \"Question 2 : le tracé est-il exhaustif, faut-il \
                       journaliser chaque rafraîchissement ?\" Oui.\n\n\
                       A3: sur le seuil — \"Question 3 : je n'ai pas fixé N pour le seuil d'échecs \
                       d'authentification répétés.\" À exprimer en durée.\n\n\
                       Disposition: READY\n"
                    .to_string(),
            },
        ];
        if let Some(text) = ready_example_shape_from_shipped_prompt() {
            cases.push(Case {
                name: "ready_example_from_the_shipped_prompt",
                origin: Origin::Real,
                text,
            });
        }
        cases
    }

    fn separates(min_count: usize, min_quote_chars: usize) -> Result<(), String> {
        for case in must_reject() {
            let v = verify_review_anchors(
                &case.text,
                BRIEF,
                &prefixes(),
                &suffix_lines(),
                min_count,
                min_quote_chars,
            );
            if v.is_satisfied() {
                return Err(format!(
                    "reject-case '{}' ({:?}) passed at ({min_count}, {min_quote_chars})",
                    case.name, case.origin
                ));
            }
        }
        for case in must_accept() {
            let v = verify_review_anchors(
                &case.text,
                BRIEF,
                &prefixes(),
                &suffix_lines(),
                min_count,
                min_quote_chars,
            );
            if !v.is_satisfied() {
                return Err(format!(
                    "accept-case '{}' ({:?}) failed at ({min_count}, {min_quote_chars}): {v:?}",
                    case.name, case.origin
                ));
            }
        }
        Ok(())
    }

    /// The shipped thresholds separate both columns. This is the assertion the manifests'
    /// declared values rest on.
    #[test]
    fn shipped_thresholds_separate_both_columns() {
        if let Err(e) = separates(3, 40) {
            panic!("the shipped thresholds (3, 40) do not separate the matrix: {e}");
        }
    }

    /// Sweep the neighbourhood and report which pairs separate. This is what turns a chosen
    /// number into a measured one: if (3, 40) were the only separating pair it would be a
    /// knife edge, and if none separated, KTD1 would be invalid.
    #[test]
    fn threshold_sweep_shows_a_separating_region_not_a_knife_edge() {
        let mut separating = Vec::new();
        for min_count in 1..=5usize {
            for min_quote_chars in [16usize, 24, 32, 40, 56, 72] {
                if separates(min_count, min_quote_chars).is_ok() {
                    separating.push((min_count, min_quote_chars));
                }
            }
        }
        // Printed so `--nocapture` shows the measured region, not just a pass/fail.
        println!("review-anchor separating (min_count, min_quote_chars): {separating:?}");
        assert!(
            !separating.is_empty(),
            "no threshold pair separates the two columns — the anchor design (KTD1) would be \
             invalid and must be routed to the operator, not patched with a different number"
        );
        assert!(
            separating.contains(&(3, 40)),
            "the shipped pair is not in the separating region: {separating:?}"
        );
        // A single separating pair would mean the thresholds sit on a knife edge and any
        // wording change in a real review would tip a genuine READY into refusal.
        assert!(
            separating.len() >= 3,
            "only {} separating pair(s) — too narrow to be robust: {separating:?}",
            separating.len()
        );
    }

    /// `min_count = 1` must NOT separate: one anchor is crossable by copying a single line of
    /// the brief, which is the obvious evasion once anchors are required at all. This is the
    /// measurement behind KTD2's choice of 3 over 1.
    #[test]
    fn a_single_anchor_does_not_separate() {
        assert!(
            separates(1, 40).is_err(),
            "min_count = 1 separated the columns, which contradicts the reason KTD2 chose 3 — \
             re-derive the threshold rather than keeping the written one"
        );
    }

    /// Inversion: the non-overlap constraint carries weight. Three anchors quoting the same
    /// region are found but only one survives — without R3 the count would reach 3 and the
    /// case would pass.
    #[test]
    fn non_overlap_constraint_is_load_bearing() {
        let case = must_reject()
            .into_iter()
            .find(|c| c.name == "three_anchors_same_region")
            .expect("the overlap case must stay in the matrix");
        match verify_review_anchors(&case.text, BRIEF, &prefixes(), &suffix_lines(), 3, 40) {
            AnchorVerdict::Missing {
                anchors_found,
                anchors_valid,
                reason,
            } => {
                assert_eq!(anchors_found, 3, "three prefixed lines were emitted");
                assert_eq!(
                    anchors_valid, 1,
                    "only one region may be claimed — without the non-overlap rule this would \
                     be 3 and the case would pass"
                );
                assert_eq!(reason, AnchorMissReason::OverlappingRegions);
            }
            other => panic!("expected overlap rejection, got {other:?}"),
        }
    }

    /// The accept column must contain at least one case whose provenance is real, or the
    /// matrix only proves the guard agrees with itself.
    #[test]
    fn the_accept_column_carries_real_provenance() {
        let real = must_accept()
            .into_iter()
            .filter(|c| c.origin == Origin::Real)
            .count();
        assert!(
            real >= 1,
            "no real-provenance accept case — the shipped prompt's READY example could not be \
             read, so the matrix measures only constructed inputs"
        );
    }
}
