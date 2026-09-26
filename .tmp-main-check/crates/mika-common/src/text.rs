/// Truncate a string slice to at most `max_bytes`, rounding down to the
/// nearest char boundary. Never panics on multi-byte UTF-8.
///
/// Unlike `db::truncate_chars` (which counts characters and appends "..."),
/// this function preserves byte-budget semantics — the returned slice is
/// always `<= max_bytes` bytes long, with no suffix appended. Use this for
/// log line widths, prompt size budgets, and error message previews.
pub fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    let end = s.floor_char_boundary(s.len().min(max_bytes));
    &s[..end]
}

// ---------------------------------------------------------------------------
// mika#2247 — typographic normalisation for the family register
// ---------------------------------------------------------------------------

/// Em-dash. The character the 2026-09-06 Telegram captures measured three times
/// in one thread on the general-public tenant.
const EM_DASH: char = '\u{2014}';
/// En-dash. Same register problem, and a model that stops emitting one tends to
/// reach for the other.
const EN_DASH: char = '\u{2013}';
/// Horizontal ellipsis. Named by the operator comment of 2026-09-08 alongside
/// the dashes as the other glm-5.2 typographic tell.
const ELLIPSIS: char = '\u{2026}';

/// Punctuation that already closes a clause: an apposition dash sitting right
/// after one of these adds nothing, so it is dropped rather than doubled into
/// `« ..., , ... »`.
const CLAUSE_CLOSERS: &[char] = &[',', ';', ':', '.', '!', '?'];

/// Does this text carry anything [`normalize_typography`] would rewrite?
///
/// Public so a caller can decide without paying for a `String` it will discard,
/// and so tests can assert idempotence as *absence* rather than as a fixpoint.
#[must_use]
pub fn has_nonascii_typography(text: &str) -> bool {
    text.contains(EM_DASH) || text.contains(EN_DASH) || text.contains(ELLIPSIS)
}

/// Rewrite em-dashes, en-dashes and ellipses into plain ASCII punctuation
/// (mika#2247, AC1 structural half).
///
/// # Why a substitution and not an EndTurn guard
///
/// The guards of the mika#953 family (5c/5d/5e) hold a budget of **one**
/// re-prompt. Against a model that emits em-dashes as a matter of style, such a
/// guard would fire on every turn, spend its budget, and **let the character
/// through anyway** — the literal shape mika#2368 had to catch with an engine
/// net. An em-dash is also not a false assertion that must be *rewritten*: it is
/// a rendering defect whose correct repair is mechanical and meaning-preserving.
/// A substitution cannot fail and costs no LLM call, which is what lets AC1 be
/// delivered as a guarantee on the sites it covers rather than as a bound.
///
/// # The three rules, in order
///
/// | context | → | why |
/// |---|---|---|
/// | line-leading (only whitespace before it on the line) | `-` | list or dialogue dash |
/// | whitespace on both sides | `,`, or nothing when the clause already closed with `,;:.!?` | French apposition dash |
/// | no adjacent whitespace (`10—12`) | `-` | range |
///
/// Plus `…` → `...` unconditionally.
///
/// # Properties
///
/// - **Idempotent**, and by construction rather than by fixpoint reasoning: the
///   output carries none of the three code points, so a second pass is a no-op.
///   Pinned by `mika2247_normalizer_is_idempotent_and_utf8_safe`.
/// - **UTF-8 safe**: iterates over `char`, never over bytes. A byte-slice
///   implementation would panic mid-character and `scripts/check-byte-slices.sh`
///   would refuse it in CI.
///
/// # Named cost
///
/// It does not tell prose from a fenced code block, so an em-dash inside a code
/// fence would be rewritten too. The family tier emits no code (its persona
/// forbids all technical vocabulary), so the population is empty and a fence
/// parser would be machinery for nobody. That is accepted and written down
/// rather than discovered.
#[must_use]
pub fn normalize_typography(text: String) -> String {
    if !has_nonascii_typography(&text) {
        return text;
    }

    let mut out = String::with_capacity(text.len());
    // Is everything emitted since the last newline whitespace? Decides rule 1.
    let mut line_is_blank_so_far = true;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            ELLIPSIS => {
                out.push_str("...");
                line_is_blank_so_far = false;
            }
            EM_DASH | EN_DASH => {
                let next_is_space = chars.peek().is_some_and(|n| n.is_whitespace());
                let prev_is_space = out.chars().next_back().is_some_and(char::is_whitespace);

                if line_is_blank_so_far {
                    // Rule 1 — list or dialogue dash. The leading whitespace is
                    // already in `out` and is preserved: indentation matters.
                    out.push('-');
                } else if prev_is_space && next_is_space {
                    // Rule 2 — apposition. Drop the space run that preceded the
                    // dash so the comma lands against the word, then let the
                    // loop emit the following whitespace normally. A newline is
                    // never popped: it would join two lines.
                    while out
                        .chars()
                        .next_back()
                        .is_some_and(|p| p == ' ' || p == '\t')
                    {
                        out.pop();
                    }
                    if !out
                        .chars()
                        .next_back()
                        .is_some_and(|p| CLAUSE_CLOSERS.contains(&p))
                    {
                        out.push(',');
                    }
                } else {
                    // Rule 3 — range, or a dash glued to one side only.
                    out.push('-');
                }
                line_is_blank_so_far = false;
            }
            '\n' => {
                out.push('\n');
                line_is_blank_so_far = true;
            }
            _ => {
                out.push(c);
                if !c.is_whitespace() {
                    line_is_blank_so_far = false;
                }
            }
        }
    }

    out
}

/// Apply [`normalize_typography`] on the tiers whose register asks for it
/// (mika#2247).
///
/// **This is the single site of the persona crossing.** Three output sites call
/// it; none of them re-states the match. The match is exhaustive with no `_ =>`
/// arm (model: `prompt::hosting_ground_truth_line`, mika#2290) so the arrival of
/// a persona forces a decision here instead of inheriting one nobody took.
///
/// `Operator` is deliberately untouched: careful typography is the register
/// Vincent chose for himself, and mika#2247's acceptance criterion names the
/// general-public tenant.
#[must_use]
pub fn normalize_typography_for_persona(
    text: String,
    persona: crate::home::PersonaProfile,
) -> String {
    match persona {
        crate::home::PersonaProfile::Family => normalize_typography(text),
        crate::home::PersonaProfile::Operator => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_shorter_than_limit() {
        assert_eq!(safe_truncate("hello", 10), "hello");
    }

    #[test]
    fn ascii_longer_than_limit() {
        assert_eq!(safe_truncate("hello world", 5), "hello");
    }

    #[test]
    fn ascii_exact_limit() {
        assert_eq!(safe_truncate("hello", 5), "hello");
    }

    #[test]
    fn em_dash_inside_boundary() {
        // "abc" is 3 bytes, "\u{2014}" (em-dash) is bytes 3..6
        // Requesting 5 bytes: byte 5 is inside the em-dash, rounds down to 3
        assert_eq!(safe_truncate("abc\u{2014}def", 5), "abc");
    }

    #[test]
    fn em_dash_end_boundary() {
        // Requesting 6 bytes: byte 6 is exactly the end of the em-dash
        assert_eq!(safe_truncate("abc\u{2014}def", 6), "abc\u{2014}");
    }

    #[test]
    fn em_dash_start_boundary() {
        // Requesting 3 bytes: byte 3 is the start of the em-dash, valid boundary
        assert_eq!(safe_truncate("abc\u{2014}def", 3), "abc");
    }

    #[test]
    fn empty_string() {
        assert_eq!(safe_truncate("", 100), "");
    }

    #[test]
    fn zero_max_bytes() {
        assert_eq!(safe_truncate("hello", 0), "");
    }

    #[test]
    fn max_bytes_exceeds_length() {
        assert_eq!(safe_truncate("short", 1000), "short");
    }

    #[test]
    fn all_multibyte_chars() {
        // Each char is 3 bytes: \u{2014}=3, \u{2192}=3, \u{2501}=3 → total 9 bytes
        let s = "\u{2014}\u{2192}\u{2501}";
        assert_eq!(s.len(), 9);

        // Inside first char
        assert_eq!(safe_truncate(s, 1), "");
        assert_eq!(safe_truncate(s, 2), "");
        // End of first char
        assert_eq!(safe_truncate(s, 3), "\u{2014}");
        // Inside second char
        assert_eq!(safe_truncate(s, 4), "\u{2014}");
        assert_eq!(safe_truncate(s, 5), "\u{2014}");
        // End of second char
        assert_eq!(safe_truncate(s, 6), "\u{2014}\u{2192}");
        // Full string
        assert_eq!(safe_truncate(s, 9), s);
    }

    // -- mika#2247: typographic normalisation ------------------------------

    fn norm(s: &str) -> String {
        normalize_typography(s.to_string())
    }

    /// The three occurrences the ticket measured on the 2026-09-06 captures.
    ///
    /// Asserted on the **output**, not on a rule: what AC1 promises is that the
    /// code point is gone from what the user reads. The first two are French
    /// appositions, the third is the English one that also carried the language
    /// drift of AC2 — kept here because AC1 must hold whatever the language.
    #[test]
    fn mika2247_the_three_measured_occurrences() {
        let cases = [
            (
                "Si tu parles de moi \u{2014} je suis déjà là",
                "Si tu parles de moi, je suis déjà là",
            ),
            (
                "Pas besoin de rien connaître \u{2014} tu me parles",
                "Pas besoin de rien connaître, tu me parles",
            ),
            ("So \u{2014} who are you", "So, who are you"),
        ];
        for (input, expected) in cases {
            let got = norm(input);
            assert_eq!(got, expected, "input: {input:?}");
            assert!(
                !has_nonascii_typography(&got),
                "AC1: no U+2014 may survive in what the tenant reads"
            );
        }
    }

    /// Rule 1 — a line-leading dash is a list or dialogue marker, never an
    /// apposition. Indentation is preserved: it is what makes a nested list
    /// still read as one.
    #[test]
    fn mika2247_line_leading_dash_becomes_a_hyphen() {
        assert_eq!(norm("\u{2014} premier point"), "- premier point");
        assert_eq!(norm("Intro\n  \u{2014} point"), "Intro\n  - point");
    }

    /// Rule 2's second half — the dash is dropped, not doubled, when the clause
    /// has already closed. `« ..., , ... »` would be a new defect of its own.
    #[test]
    fn mika2247_apposition_after_a_closed_clause_drops_the_dash() {
        assert_eq!(norm("Bonjour, \u{2014} et puis"), "Bonjour, et puis");
        assert_eq!(norm("Fini. \u{2014} Ensuite"), "Fini. Ensuite");
    }

    /// Rule 3 — a range carries no adjacent space and means a hyphen.
    #[test]
    fn mika2247_range_becomes_a_hyphen() {
        assert_eq!(norm("10\u{2014}12"), "10-12");
        assert_eq!(norm("pages 3\u{2013}5"), "pages 3-5");
    }

    /// A dash at end of line must not join the two lines: the space run is
    /// popped but the newline never is.
    #[test]
    fn mika2247_end_of_line_dash_keeps_the_line_break() {
        assert_eq!(
            norm("de la personne \u{2014}\n  bref si elle est brève"),
            "de la personne,\n  bref si elle est brève"
        );
    }

    /// The ellipsis is the third code point the operator comment names.
    #[test]
    fn mika2247_ellipsis_becomes_three_dots() {
        assert_eq!(norm("Attends\u{2026} voilà"), "Attends... voilà");
    }

    /// Idempotence **and** multi-byte safety in one test, because the two share
    /// a cause: the function iterates over `char`. A byte-slicing version would
    /// panic here rather than return a wrong answer.
    #[test]
    fn mika2247_normalizer_is_idempotent_and_utf8_safe() {
        let input = "Émoji 😀 et accents àéîöû \u{2014} puis 10\u{2013}12\u{2026} \
                     et « guillemets » \u{2014} fin";
        let once = norm(input);
        let twice = normalize_typography(once.clone());
        assert_eq!(once, twice, "a second pass must be a no-op");
        assert!(
            !has_nonascii_typography(&once),
            "idempotence is obtained by absence, not by fixpoint"
        );
        // The non-target multi-byte characters are untouched.
        assert!(once.contains('😀'));
        assert!(once.contains("àéîöû"));
        assert!(once.contains("« guillemets »"));
    }

    /// The fast path must be a genuine no-op — same bytes, and the `String` is
    /// moved rather than rebuilt.
    #[test]
    fn mika2247_text_without_target_codepoints_is_returned_unchanged() {
        let input = "Bonjour ! Tout va bien, 10-12 pages.".to_string();
        assert_eq!(normalize_typography(input.clone()), input);
        assert!(!has_nonascii_typography(&input));
    }

    /// mika#2247 — the persona crossing lives at ONE site and this pins both
    /// arms. The operator tier is not a leftover: it is the register Vincent
    /// chose for himself, and the acceptance criterion names the general-public
    /// tenant only.
    #[test]
    fn mika2247_the_persona_crossing_spares_the_operator_register() {
        use crate::home::PersonaProfile;
        let input = "Un tiret \u{2014} cadratin".to_string();
        assert_eq!(
            normalize_typography_for_persona(input.clone(), PersonaProfile::Family),
            "Un tiret, cadratin"
        );
        assert_eq!(
            normalize_typography_for_persona(input.clone(), PersonaProfile::Operator),
            input,
            "operator tier keeps its typography — mika#2247 § 7"
        );
    }

    #[test]
    fn four_byte_emoji_at_boundary() {
        // "ab" = 2 bytes, 😀 = 4 bytes (U+1F600), "cd" = 2 bytes → total 8 bytes
        let s = "ab\u{1F600}cd";
        assert_eq!(s.len(), 8);

        // Inside emoji
        assert_eq!(safe_truncate(s, 3), "ab");
        assert_eq!(safe_truncate(s, 4), "ab");
        assert_eq!(safe_truncate(s, 5), "ab");
        // End of emoji
        assert_eq!(safe_truncate(s, 6), "ab\u{1F600}");
        // Full string
        assert_eq!(safe_truncate(s, 8), s);
    }
}
