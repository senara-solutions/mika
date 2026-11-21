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
//!
//! **A fenced block longer than the budget is cut inside it, and the cut is
//! repaired** (vol 2 of mika#3114): the chunk ends with an added `"\n```"` and the
//! remainder is reopened with ``` ```{label} ```, the same label as the original
//! opening, empty label included. Without that repair the block had no neutral
//! candidate at all, so the floor split its backticks across two chunks and the
//! reader saw apparent fences around unrendered code.
//!
//! **The oracle validates the repair, so no grammar is replicated here.** The same
//! equality is checked on the two *repaired* halves ([`repair_is_neutral`]), which is
//! why the label can be a guess: a wrong one, a closing fence mistaken for an
//! opening, or a fence that was never open all fail the equality and fall back to
//! today's floor. The reopening rides on the remainder rather than being carried
//! separately, so the next turn measures it as ordinary text; the budget reserves the
//! closing fence before choosing the cut, which is how the added characters count
//! against the 4096-unit limit instead of overflowing it.

use mika_common::telegram::text_len_utf16;

use crate::routes::format_outbound_text;
use crate::telegram_markdown::{render_plain, tokenize};

/// The fence marker, `` ``` ``.
const FENCE: &str = "```";

/// What a chunk cut inside a fenced block ends with. The newline is **not**
/// cosmetic: `strip_fence_decoration` drops exactly one trailing `\n`, so a close
/// written without it would eat the block's last line.
const FENCE_CLOSE: &str = "\n```";

/// `FENCE_CLOSE` in UTF-16 units — the room the budget reserves before choosing a
/// cut, which is what makes the added characters count against the 4096-unit limit.
const CLOSE_UNITS: usize = 4;

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
    // `rest` owns its bytes because a repaired cut *prepends* the reopening to the
    // remainder: one working text, so the budget and the neutrality oracle keep
    // measuring exactly what the next chunk will carry.
    let mut rest = text.to_string();
    while !rest.is_empty() {
        let cut = choose_cut(&rest, budget);
        if cut == 0 {
            chunks.push(format_outbound_text(agent_name, &rest));
            break;
        }
        // Reaching the floor is what "nothing here is neutral" looks like, and a
        // fenced block longer than the budget is the case that produces it.
        if cut < rest.len()
            && cut == hard_boundary(&rest, budget)
            && let Some((fence_at, reopen)) = fence_cut(&rest, budget)
        {
            chunks.push(format_outbound_text(
                agent_name,
                &format!("{}{FENCE_CLOSE}", &rest[..fence_at]),
            ));
            rest = format!("{reopen}{}", &rest[fence_at..]);
            continue;
        }
        chunks.push(format_outbound_text(agent_name, &rest[..cut]));
        rest = rest[cut..].to_string();
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

    let hard = hard_boundary(rest, budget);
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

/// The last byte offset of `rest` holding at most `budget` UTF-16 units. Accumulating
/// `c.len_utf16()` keeps the offset out of a character *and* of a surrogate pair.
fn hard_boundary(rest: &str, budget: usize) -> usize {
    let mut units = 0usize;
    for (idx, c) in rest.char_indices() {
        if units + c.len_utf16() > budget {
            return idx;
        }
        units += c.len_utf16();
    }
    rest.len()
}

/// Whether cutting `rest` at `cut` leaves the recognizer's reading unchanged.
/// `baseline` is `render_plain(tokenize(rest))`, passed in so one slice's candidates
/// share a single rendering.
fn cut_is_neutral(rest: &str, cut: usize, baseline: &str) -> bool {
    repair_is_neutral(&rest[..cut], &rest[cut..], baseline)
}

/// The same oracle, over two pieces that are no longer a partition of the input:
/// `left` carries an added closing fence, `right` an added opening one. **That is the
/// whole of the fenced-block design** — a repair is kept only when the recognizer
/// reads the two repaired pieces as it read the whole, so a wrong label guess, a
/// fence that was never open, or any other fabrication is refused here rather than
/// prevented by a second grammar nobody could keep in step with `telegram_markdown`.
fn repair_is_neutral(left: &str, right: &str, baseline: &str) -> bool {
    let mut joined = render_plain(&tokenize(left));
    joined.push_str(&render_plain(&tokenize(right)));
    joined == baseline
}

/// The label of the fenced block `head` ends inside, if it ends inside one.
///
/// A guess, deliberately: the last `` ``` `` run, then the remainder of its line,
/// which `parse_fence` accepts as an info string only when it carries no whitespace.
/// A closing fence, a block that ended, or four backticks all produce a *plausible*
/// label here and a non-neutral repair in [`repair_is_neutral`], which is what
/// refuses them.
fn fence_label_before(head: &str) -> Option<String> {
    let run = head.rfind(FENCE)?;
    let first_line = head[run + FENCE.len()..].split('\n').next().unwrap_or("");
    (!first_line.contains(char::is_whitespace)).then(|| first_line.to_string())
}

/// Where to cut *inside* a fenced block, and the opening the next chunk reopens with.
///
/// The budget reserves the closing fence **before** the cut is chosen
/// ([`CLOSE_UNITS`]), so the added characters count against the 4096-unit limit
/// rather than overflowing it; the reopening is reserved by riding on the remainder,
/// which the next turn measures as ordinary text.
fn fence_cut(rest: &str, budget: usize) -> Option<(usize, String)> {
    let body_budget = budget.checked_sub(CLOSE_UNITS)?;
    let body_hard = hard_boundary(rest, body_budget);
    let baseline = render_plain(&tokenize(rest));

    // A line end is preferred — but only past half the budget. The reopening
    // *regenerates* a newline at a low fixed offset, so a line end near the start of
    // a block would be rediscovered turn after turn and the split would crawl.
    let candidates = [
        rest[..body_hard]
            .rfind('\n')
            .map(|i| i + 1)
            .filter(|&c| text_len_utf16(&rest[..c]) * 2 >= body_budget),
        Some(body_hard),
    ];

    for cand in candidates.into_iter().flatten() {
        let Some(label) = fence_label_before(&rest[..cand]) else {
            continue;
        };
        let reopen = format!("{FENCE}{label}\n");
        // Strict progress, and termination with it: the remainder carries the
        // reopening, so a cut at or before its length would not shorten anything.
        if cand <= reopen.len() {
            continue;
        }
        let left = format!("{}{FENCE_CLOSE}", &rest[..cand]);
        let right = format!("{reopen}{}", &rest[cand..]);
        if repair_is_neutral(&left, &right, &baseline) {
            return Some((cand, reopen));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::telegram_markdown::SegmentKind;

    use super::*;

    /// The assertions every fenced-block case shares: each chunk fits the limit with
    /// its added fences, each chunk's fences are balanced, and the chunks rejoined —
    /// prefixes and added fences removed — are the input, in order and without loss.
    ///
    /// An added close is recognized by what follows it: the *next* chunk reopens a
    /// fence. An original closing fence is followed by prose or by nothing.
    fn assert_chunks_wellformed(
        chunks: &[String],
        prefix: Option<&str>,
        limit: usize,
        input: &str,
    ) {
        let strip = |chunk: &String| match prefix {
            Some(p) => chunk.strip_prefix(p).expect("agent prefix").to_string(),
            None => chunk.clone(),
        };

        let mut rejoined = String::new();
        let mut reopening = false;
        for (i, chunk) in chunks.iter().enumerate() {
            let units = text_len_utf16(chunk);
            assert!(units <= limit, "chunk {i} of {units} units exceeds {limit}");

            let owned = strip(chunk);
            let mut body = owned.as_str();
            assert_eq!(
                body.matches(FENCE).count() % 2,
                0,
                "chunk {i} has unbalanced fences: {body:?}"
            );

            if reopening {
                assert!(
                    body.starts_with(FENCE),
                    "chunk {i} continues a block without reopening it: {body:?}"
                );
                let line = body.find('\n').expect("a reopening ends its own line") + 1;
                body = &body[line..];
            }

            reopening = body.ends_with(FENCE_CLOSE)
                && chunks
                    .get(i + 1)
                    .is_some_and(|next| strip(next).starts_with(FENCE));
            if reopening {
                body = body.strip_suffix(FENCE_CLOSE).expect("just matched");
            }
            rejoined.push_str(body);
        }
        assert_eq!(rejoined, input, "the chunks rejoined are not the input");
    }

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

    /// T1, AC1(c)(a) — a ` ```rust ` block longer than the limit is cut *inside*,
    /// closed, and reopened with its own label.
    #[test]
    fn mika3114_a_fenced_block_over_the_limit_is_closed_and_reopened_labelled() {
        let text = format!("{FENCE}rust\n{}{FENCE}", "let x = 1;\n".repeat(600));
        assert!(text_len_utf16(&text) > 4096, "the fixture must overflow");

        let chunks = split_outbound_text(Some("mika-dev"), &text, 4096);
        assert!(chunks.len() > 1, "a 6 600-unit block must be split");
        assert_chunks_wellformed(&chunks, Some("[mika-dev] "), 4096, &text);
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.contains(&format!("{FENCE}rust\n")),
                "chunk {i} carries a part of the block and must open it labelled"
            );
        }

        // Negative control — `origin/main`'s shape at ebf7e6e0, verified and not just
        // commented: the floor alone leaves the first piece with one lone fence.
        let floor = choose_cut(&text, 4096 - "[mika-dev] ".len());
        assert!(floor > 0 && floor < text.len(), "the floor cuts the block");
        assert_eq!(
            text[..floor].matches(FENCE).count() % 2,
            1,
            "without the repair the fences are unbalanced — the defect this fixes"
        );
    }

    /// T2, AC1(c)(b) — a block spanning at least three chunks: every intermediate
    /// chunk is closed *and* reopened, not just the first and the last.
    #[test]
    fn mika3114_a_fenced_block_over_three_chunks_is_reopened_each_time() {
        let text = format!("{FENCE}rust\n{}{FENCE}", "let y = 2;\n".repeat(1200));
        let chunks = split_outbound_text(None, &text, 4096);
        assert!(chunks.len() >= 3, "got {} chunks", chunks.len());
        assert_chunks_wellformed(&chunks, None, 4096, &text);

        for middle in &chunks[1..chunks.len() - 1] {
            assert!(middle.starts_with(&format!("{FENCE}rust\n")));
            assert!(middle.ends_with(FENCE_CLOSE));
        }
    }

    /// T3, AC1(c)(c) — an unlabelled block reopens unlabelled. The empty label is a
    /// value the repair carries, never a default it guesses.
    #[test]
    fn mika3114_an_unlabelled_fenced_block_reopens_unlabelled() {
        let text = format!("{FENCE}\n{}{FENCE}", "sans etiquette\n".repeat(400));
        let chunks = split_outbound_text(None, &text, 4096);
        assert!(chunks.len() > 1);
        assert_chunks_wellformed(&chunks, None, 4096, &text);

        for chunk in &chunks[1..] {
            assert!(chunk.starts_with(&format!("{FENCE}\n")), "{chunk:.20?}");
            assert!(!chunk.starts_with(&format!("{FENCE}rust")));
        }
    }

    /// T4, AC1(c)(d) — a block that fits is never cut, and gets no added fence: the
    /// repair fires on the floor, and prose has neutral candidates long before it.
    #[test]
    fn mika3114_a_fenced_block_that_fits_is_left_whole() {
        let block = format!("{FENCE}rust\n{}{FENCE}", "let z = 3;\n".repeat(50));
        let text = format!(
            "{}\n\n{block}\n\n{}",
            "Avant. ".repeat(400),
            "Apres. ".repeat(400)
        );

        let chunks = split_outbound_text(None, &text, 4096);
        assert!(chunks.len() > 1, "the surrounding prose overflows");
        assert_chunks_wellformed(&chunks, None, 4096, &text);
        assert_eq!(
            chunks.iter().filter(|c| c.contains(&block)).count(),
            1,
            "exactly one chunk carries the block, whole"
        );
        assert_eq!(
            chunks
                .iter()
                .map(|c| c.matches(FENCE).count())
                .sum::<usize>(),
            2,
            "no fence was added anywhere"
        );
    }

    /// T5, AC1(c)(e) — an unclosed fence is not a block for `tokenize`
    /// (`mika2291_unclosed_fence_stays_verbatim`), so the split adds nothing to it.
    ///
    /// **This is the test of the oracle**, not of a special case: the second half
    /// asserts the illegitimate repair is what gets refused.
    #[test]
    fn mika3114_an_unclosed_fence_receives_no_added_fence() {
        let text = format!("{FENCE}rust\n{}", "B".repeat(5000));
        let chunks = split_outbound_text(None, &text, 4096);

        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), text, "not a byte added, not one removed");
        assert_eq!(
            chunks
                .iter()
                .map(|c| c.matches(FENCE).count())
                .sum::<usize>(),
            1,
            "the lone unclosed fence stays lone"
        );

        let baseline = render_plain(&tokenize(&text));
        let cut = hard_boundary(&text, 4096);
        assert!(
            !repair_is_neutral(
                &format!("{}{FENCE_CLOSE}", &text[..cut]),
                &format!("{FENCE}rust\n{}", &text[cut..]),
                &baseline
            ),
            "the oracle must refuse closing a fence that was never open"
        );
    }

    /// T6, AC1(c)(f) — each chunk's part of the block is read as a `Pre` segment: the
    /// code is rendered, not shown raw with apparent fences.
    #[test]
    fn mika3114_every_chunk_of_a_block_is_read_as_a_pre_segment() {
        let text = format!("{FENCE}rust\n{}{FENCE}", "let w = 4;\n".repeat(600));
        let chunks = split_outbound_text(None, &text, 4096);
        assert!(chunks.len() > 1);

        for (i, chunk) in chunks.iter().enumerate() {
            let segments = tokenize(chunk);
            assert!(
                segments.iter().any(|s| s.kind == SegmentKind::Pre),
                "chunk {i} holds no Pre segment — the reader would see raw backticks"
            );
        }
    }
}
