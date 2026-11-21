//! Split outbound Telegram text at the UTF-16 limit (mika#3114).
//!
//! Telegram caps `sendMessage`'s `text` field at 4096 UTF-16 units and answers
//! 400 "message is too long" beyond it. `handle_send` sent the text it received
//! in a single call, so the gateway answered 502 and **the message was lost** —
//! 418 of them since 2026-09-20, plus 420 `telegram_html_fallback_failed` lines
//! carrying the same description, the plain-text fallback of mika#2291 being
//! just as long as the HTML it replaces.
//!
//! This module is the one place that decides where the text is cut. The send
//! loop lives in `routes::send_in_chunks`; the rendering of each piece is
//! unchanged and stays in `telegram::send_message_impl`.
//!
//! **The cut predicate reuses the recognizer instead of asking it for offsets.**
//! `telegram_markdown::Segment` carries no position in the source text and the
//! markers have been stripped, so the position is not reconstructible — and
//! `telegram_markdown.rs` is out of scope for this fix, so no offset function can
//! be added there. Instead, a cut at index `k` is *neutral* when
//!
//! ```text
//! render_plain(tokenize(&s[..k])) + render_plain(tokenize(&s[k..]))
//!     == render_plain(tokenize(s))
//! ```
//!
//! A cut inside `**bold**` leaves `**bo` on the left (unpaired markers, so
//! verbatim) and `ld**` on the right: the equality breaks and the cut is
//! refused. A clean cut leaves the segments intact on both sides and the
//! equality holds. The predicate therefore refuses, by construction, any cut
//! that changes what the recognizer sees — which is exactly the invariant asked
//! for, without a second recognition.
//!
//! **Named degradation, tracked on senara-solutions/mika#2983.** A fenced code
//! block longer than the limit has no neutral cut candidate at all (the
//! predicate refuses every cut inside a fence), so the split falls back on the
//! progress floor and the backticks end up unbalanced across the pieces.
//! `tokenize` does not recognize an unclosed fence — its backticks stay `Plain`
//! verbatim — so Telegram accepts the send and the reader sees apparent ```.
//! That is cosmetic: the message is no longer lost, which is the measured
//! defect. Re-opening the fence with its language tag is the follow-up.

use mika_common::telegram::text_len_utf16;

use crate::routes::format_outbound_text;
use crate::telegram_markdown::{render_plain, tokenize};

/// Split `text` into pieces that each fit `limit` UTF-16 units, prefix included.
///
/// `limit` is a parameter rather than a constant read at the site so tests can
/// exercise the split on short texts; production passes
/// `mika_common::telegram::MAX_TEXT_UTF16_UNITS`.
///
/// **The prefix has a single composition site.** Every piece — including the
/// single piece of a text that already fits — is composed by
/// [`format_outbound_text`], so a text under the limit returns *literally*
/// `vec![format_outbound_text(agent_name, text)]` and the identity with today's
/// path is guaranteed by construction rather than by inspection. The prefix is
/// repeated on every piece so `resolve_reply_agent` routes a reply made to any
/// one of them.
///
/// The concatenation of the pieces, prefixes removed, is always the input: no
/// loss, no reordering. Even the degenerate case where the budget cannot hold
/// the first `char` emits the remainder whole rather than dropping it — an
/// oversized send that Telegram refuses is still accounted for, a silent
/// truncation would not be.
pub(crate) fn split_outbound_text(
    agent_name: Option<&str>,
    text: &str,
    limit: usize,
) -> Vec<String> {
    let full = format_outbound_text(agent_name, text);
    let full_units = text_len_utf16(&full);
    if full_units <= limit {
        return vec![full];
    }

    // The prefix is measured through the composition itself rather than
    // re-derived from `agent_name`: a second formula is a second chance to
    // disagree with `format_outbound_text` about what the prefix is.
    let prefix_units = full_units.saturating_sub(text_len_utf16(text));
    let budget = limit.saturating_sub(prefix_units);

    let mut chunks: Vec<String> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let cut = choose_cut(rest, budget);
        if cut == 0 {
            chunks.push(format_outbound_text(agent_name, rest));
            break;
        }
        chunks.push(format_outbound_text(agent_name, &rest[..cut]));
        rest = &rest[cut..];
    }
    chunks
}

/// Where to cut `rest` so the piece holds `budget` UTF-16 units.
///
/// Preference order: end of paragraph, then end of line, then a space, then the
/// hard boundary. A candidate is retained only when it is neutral for the
/// recognizer ([`cut_is_neutral`]); the hard boundary is the **progress floor**
/// that guarantees termination when nothing is neutral.
fn choose_cut(rest: &str, budget: usize) -> usize {
    if text_len_utf16(rest) <= budget {
        return rest.len();
    }

    // The largest `char`-boundary index whose accumulated UTF-16 units stay
    // within budget. Accumulating `c.len_utf16()` over `char_indices()` is what
    // keeps a cut out of the middle of a character *and* out of the middle of a
    // surrogate pair: a non-BMP `char` contributes its 2 units in one piece.
    let mut hard = rest.len();
    let mut units = 0usize;
    for (idx, c) in rest.char_indices() {
        if units + c.len_utf16() > budget {
            hard = idx;
            break;
        }
        units += c.len_utf16();
    }

    let head = &rest[..hard];
    // Computed once per slice, outside the candidate loop: the predicate is the
    // only expensive part of this function, and the three candidates share the
    // same reference rendering.
    let baseline = render_plain(&tokenize(rest));

    // Each candidate cuts *after* its separator, so the right-hand piece starts
    // at a line start where the separator was a newline — which is what keeps
    // the heading and bullet rules of `tokenize` firing identically on both
    // sides of the cut.
    let candidates = [
        head.rfind("\n\n").map(|i| i + 2),
        head.rfind('\n').map(|i| i + 1),
        head.rfind(' ').map(|i| i + 1),
    ];
    for cand in candidates.into_iter().flatten() {
        if cand > 0 && cut_is_neutral(rest, cand, &baseline) {
            return cand;
        }
    }

    hard
}

/// Whether cutting `rest` at `cut` leaves the recognizer's reading unchanged.
///
/// `baseline` is `render_plain(tokenize(rest))`, passed in rather than computed
/// here so the three candidates of one slice share a single rendering.
fn cut_is_neutral(rest: &str, cut: usize, baseline: &str) -> bool {
    let mut joined = render_plain(&tokenize(&rest[..cut]));
    joined.push_str(&render_plain(&tokenize(&rest[cut..])));
    joined == baseline
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A text under the limit yields one piece, identical to today's path.
    #[test]
    fn mika3114_a_text_under_the_limit_is_one_untouched_piece() {
        let chunks = split_outbound_text(None, "bonjour", 4096);
        assert_eq!(chunks, vec!["bonjour".to_string()]);

        let chunks = split_outbound_text(Some("mika-dev"), "bonjour", 4096);
        assert_eq!(
            chunks,
            vec![format_outbound_text(Some("mika-dev"), "bonjour")]
        );
    }

    /// 10 000 characters in paragraphs: every piece fits, prefix included, and
    /// the concatenation is the input — no loss, no reordering.
    #[test]
    fn mika3114_ten_thousand_characters_split_without_loss() {
        let mut text = String::new();
        while text.len() < 10_000 {
            text.push_str("Paragraphe de remplissage pour le test de decoupage.\n\n");
        }
        text.truncate(10_000); // ASCII filler: bytes == chars == UTF-16 units.
        assert_eq!(text_len_utf16(&text), 10_000);

        let prefix = "[mika-dev] ";
        let chunks = split_outbound_text(Some("mika-dev"), &text, 4096);
        assert!(chunks.len() > 1, "a 10 000-unit text must be split");

        let mut rejoined = String::new();
        for chunk in &chunks {
            assert!(
                text_len_utf16(chunk) <= 4096,
                "chunk of {} UTF-16 units exceeds the limit",
                text_len_utf16(chunk)
            );
            let body = chunk
                .strip_prefix(prefix)
                .expect("every chunk carries the agent prefix");
            rejoined.push_str(body);
        }
        assert_eq!(rejoined, text);
    }

    /// Non-BMP emoji: the limit is counted in UTF-16 units, not in bytes and not
    /// in `char`s, and no cut falls inside a surrogate pair.
    #[test]
    fn mika3114_non_bmp_emoji_are_counted_in_utf16_units() {
        let text = "👆".repeat(50);
        let limit = 20;

        // The three counts disagree, which is what makes the assertion below a
        // measurement of the UTF-16 one rather than a coincidence.
        assert_eq!(text.len(), 200);
        assert_eq!(text.chars().count(), 50);
        assert_eq!(text_len_utf16(&text), 100);

        let chunks = split_outbound_text(None, &text, limit);
        assert_eq!(chunks.len(), 5, "100 UTF-16 units / 20 = 5 pieces");
        assert_ne!(chunks.len(), text.len().div_ceil(limit));
        assert_ne!(chunks.len(), text.chars().count().div_ceil(limit));

        for chunk in &chunks {
            assert_eq!(text_len_utf16(chunk), limit);
            assert!(
                !chunk.contains('\u{fffd}'),
                "a cut inside a surrogate pair would surface as U+FFFD"
            );
        }
        assert_eq!(chunks.concat(), text);
    }

    /// A `**bold**` straddling the limit: the cut falls before it.
    #[test]
    fn mika3114_a_cut_never_falls_inside_a_bold_run() {
        // The hard boundary at 20 units lands inside `**gras**`; the space just
        // before it is the candidate the preference order retains.
        let text = "0123456789 0123 **gras** fin";
        let chunks = split_outbound_text(None, text, 20);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "0123456789 0123 ");
        assert_eq!(chunks[1], "**gras** fin");
        assert!(
            !chunks[0].ends_with('*'),
            "the first piece must not end on a prefix of the bold run"
        );
        assert!(chunks[1].starts_with("**"));

        // And the retained cut satisfies the invariance predicate itself.
        let baseline = render_plain(&tokenize(text));
        assert!(cut_is_neutral(text, chunks[0].len(), &baseline));
        // Control: a cut inside the run is refused, so the assertion above is
        // not vacuously true of every index.
        assert!(!cut_is_neutral(text, 18, &baseline));
    }
}
