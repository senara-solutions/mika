//! Split outbound Telegram text at the UTF-16 limit (mika#3114).
//!
//! Telegram caps `text` at 4096 UTF-16 units and answers 400 beyond it, so a long
//! `/send` became a 502 and a lost message. This module decides where the text is
//! cut; `routes::send_in_chunks` sends the pieces and `telegram::send_message_impl`
//! renders each one, unchanged. Measurement, surfaces and named degradations:
//! `crates/mika-gateway/CLAUDE.md` § *Outbound Splitting*.
//!
//! **The cut predicate reuses the recognizer instead of asking it for offsets.**
//! `Segment` carries no source position and `telegram_markdown.rs` is out of scope
//! here, so a cut at `k` is kept only when it is *neutral*:
//! `render_plain(tokenize(&s[..k])) + render_plain(tokenize(&s[k..]))` equals
//! `render_plain(tokenize(s))`. A cut inside `**bold**` leaves unpaired markers on
//! both sides and the equality breaks — any cut that would change what the
//! recognizer sees is refused by construction.

use mika_common::telegram::text_len_utf16;

use crate::routes::format_outbound_text;
use crate::telegram_markdown::{render_plain, tokenize};

/// Split `text` into pieces that each fit `limit` UTF-16 units, prefix included.
///
/// **The prefix has a single composition site**: every piece goes through
/// [`format_outbound_text`], so a text under the limit returns *literally*
/// `vec![format_outbound_text(agent_name, text)]` — identity with today's path by
/// construction, not by inspection — and the prefix repeats on each piece so
/// `resolve_reply_agent` routes a reply made to any of them. The pieces concatenated,
/// prefixes removed, are always the input: even a budget too small for the first
/// `char` emits the remainder whole, since an oversized send Telegram refuses is
/// accountable where a silent truncation would not be.
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

    // Measured through the composition, never recomputed from the agent name.
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

/// Where to cut `rest` so the piece holds `budget` UTF-16 units. Preference: end of
/// paragraph, end of line, a space, then the hard boundary. A candidate is retained
/// only when [`cut_is_neutral`]; the hard boundary is the **progress floor** that
/// guarantees termination when nothing is neutral.
fn choose_cut(rest: &str, budget: usize) -> usize {
    if text_len_utf16(rest) <= budget {
        return rest.len();
    }

    // Accumulating `c.len_utf16()` keeps the cut out of a character *and* of a pair.
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
    // Once per slice: the three candidates share one rendering of the baseline.
    let baseline = render_plain(&tokenize(rest));

    // Each candidate cuts *after* its separator, so a newline separator leaves the
    // right-hand piece starting at a line start — `tokenize`'s heading and bullet
    // rules then fire identically on both sides.
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
/// `baseline` is `render_plain(tokenize(rest))`, passed in so one slice's candidates
/// share a single rendering.
fn cut_is_neutral(rest: &str, cut: usize, baseline: &str) -> bool {
    let mut joined = render_plain(&tokenize(&rest[..cut]));
    joined.push_str(&render_plain(&tokenize(&rest[cut..])));
    joined == baseline
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC1(a) — a text under the limit is one piece, identical to today's path.
    #[test]
    fn mika3114_a_text_under_the_limit_is_one_untouched_piece() {
        assert_eq!(split_outbound_text(None, "bonjour", 4096), vec!["bonjour"]);
        assert_eq!(
            split_outbound_text(Some("mika-dev"), "bonjour", 4096),
            vec![format_outbound_text(Some("mika-dev"), "bonjour")]
        );
    }

    /// AC1(b) — 10 000 characters in paragraphs: every piece fits, prefix included,
    /// and the concatenation is the input. No loss, no reordering.
    #[test]
    fn mika3114_ten_thousand_characters_split_without_loss() {
        let mut text = "Paragraphe de remplissage pour le test de decoupage.\n\n".repeat(200);
        text.truncate(10_000); // safe-byte-slice: pure-ASCII filler, every offset is a char boundary — the assertion below measures it.
        assert_eq!(text_len_utf16(&text), 10_000);

        let chunks = split_outbound_text(Some("mika-dev"), &text, 4096);
        assert!(chunks.len() > 1, "a 10 000-unit text must be split");

        let mut rejoined = String::new();
        for chunk in &chunks {
            let units = text_len_utf16(chunk);
            assert!(units <= 4096, "chunk of {units} units exceeds the limit");
            rejoined.push_str(chunk.strip_prefix("[mika-dev] ").expect("agent prefix"));
        }
        assert_eq!(rejoined, text);
    }

    /// AC1(d) — the limit is counted in UTF-16 units, not in bytes and not in
    /// `char`s, and no cut falls inside a surrogate pair.
    #[test]
    fn mika3114_non_bmp_emoji_are_counted_in_utf16_units() {
        let text = "👆".repeat(50);
        let limit = 20;

        // The three counts disagree, so the piece count below measures the UTF-16 one.
        assert_eq!(text.len(), 200);
        assert_eq!(text.chars().count(), 50);
        assert_eq!(text_len_utf16(&text), 100);

        let chunks = split_outbound_text(None, &text, limit);
        assert_eq!(chunks.len(), 5, "100 UTF-16 units / 20 = 5 pieces");
        assert_ne!(chunks.len(), text.len().div_ceil(limit));
        assert_ne!(chunks.len(), text.chars().count().div_ceil(limit));

        for chunk in &chunks {
            assert_eq!(text_len_utf16(chunk), limit);
            assert!(!chunk.contains('\u{fffd}'), "a split pair reads as U+FFFD");
        }
        assert_eq!(chunks.concat(), text);
    }

    /// AC1(e) — a `**gras**` straddling the limit: the cut falls before it.
    #[test]
    fn mika3114_a_cut_never_falls_inside_a_bold_run() {
        // The hard boundary at 20 units lands inside `**gras**`; the space before it
        // is the candidate the preference order retains.
        let text = "0123456789 0123 **gras** fin";
        let chunks = split_outbound_text(None, text, 20);

        assert_eq!(chunks, vec!["0123456789 0123 ", "**gras** fin"]);
        assert!(!chunks[0].ends_with('*'), "no prefix of the bold run kept");

        // The control shows the predicate is not vacuously true of every index.
        let baseline = render_plain(&tokenize(text));
        assert!(cut_is_neutral(text, chunks[0].len(), &baseline));
        assert!(!cut_is_neutral(text, 18, &baseline));
    }
}
