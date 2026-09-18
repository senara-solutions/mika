//! Outbound Telegram text rendering — one recognizer, two renderings (mika#2291).
//!
//! **The defect this closes, measured 2026-09-11 on Al's cloud tenant.** Telegram
//! replies showed the markdown **raw**: `**gras**` arrived literally instead of
//! rendered. Not a regression of mika#2126 — the complement of its perimeter, which
//! that ticket left open and named (`strip_markdown_around_urls`'s doc-comment: *"the
//! perimeter is URLs, not markdown rendering"*).
//!
//! **The structural property the whole design rests on: one recognition, two
//! emissions.**
//!
//! ```text
//! tokenize(text) -> Vec<Segment>        // recognition, once
//!     render_html(&segments)  -> String // armed mode
//!     render_plain(&segments) -> String // floor, fallback, and disarmed mode
//! ```
//!
//! Three consequences, all of them the point:
//!
//! 1. **Disarming and the fallback produce the same byte.** The kill-switch is not a
//!    parallel untested code path: it *is* the fallback path, exercised by the
//!    fallback's own tests. A production rollback takes a road that already runs.
//! 2. **AC3 is structural, not a coincidence of tests.** If `tokenize` returns a
//!    single [`SegmentKind::Plain`] covering the whole input, [`render_plain`]
//!    restores the input byte-for-byte *by construction*. The byte-for-byte
//!    preservation frozen by mika#2126's negative controls becomes a property of the
//!    type rather than of a test corpus.
//! 3. **The two renderings cannot diverge on what they recognize**, only on what they
//!    emit. The repo has had to impose that shape twice with structural guards
//!    (mika#2158 "one reader of the grooming verdict", mika#2363 "one predicate");
//!    here the signature gives it for free.
//!
//! **HTML, never MarkdownV2.** MarkdownV2 requires escaping eighteen characters
//! across the *entire* text, URLs included; Telegram's HTML mode requires three —
//! `<`, `>`, `&` — and only outside tags. The error surface is an order of magnitude
//! smaller, and it is *local* (escape the text content, emit the tags ourselves)
//! instead of *global*.
//!
//! **No CommonMark parser, and the reason is a tested property rather than a
//! preference.** A parse → AST → render round-trip normalizes whitespace,
//! reinterprets four-space indentation as a code block, `1. ` as an ordered list and
//! `---` as a rule — it *over-interprets* conversational prose. mika#2126 froze
//! `"Ligne un\n\n  Va voir  https://example.com/a\tfin"` **with its double spaces and
//! its tab**; a dependency would make that test red. Revision criterion, written so
//! it need not be rediscovered: the day the output must carry tables, nested lists or
//! structured block quotes, a targeted recognizer stops being the right tool and a
//! parser becomes justifiable — but the byte-for-byte constraint must then be
//! renegotiated explicitly, because it and a parser are incompatible.
//!
//! **Infallible by construction**, at mika#2126's own standard: no `Result`, no
//! `unwrap`, no `expect`, no `panic!`, no raw byte indexing — every split is over a
//! `Vec<char>`, never `&text[i..j]`. Pinned by a source scan
//! (`mika2291_s3_render_is_infallible_by_construction`) rather than by a behavioural
//! test, because the regression would not produce a wrong output: it would produce a
//! **panicable** send.

use std::sync::OnceLock;

use tracing::info;

/// What a [`Segment`] is, flat and never nested.
///
/// Telegram accepts nested entities; agent prose practically never produces them,
/// and staying flat keeps the transformer total and testable. The one nesting shape
/// worth handling — emphasis wrapped around a whole markdown link,
/// `**[label](url)**`, which mika#2126 already has a test for — collapses to
/// [`SegmentKind::Link`]: a link that works beats a link that is bold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SegmentKind {
    /// Verbatim text. Everything the recognition table below does not name.
    Plain,
    Bold,
    Italic,
    Code,
    Pre,
    Strike,
    /// `text` carries the label; `url` the target, already validated by
    /// [`crate::telegram::parse_markdown_link`]'s grammar.
    Link {
        url: String,
    },
}

/// One recognized run of outbound text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    pub(crate) kind: SegmentKind,
    pub(crate) text: String,
}

// -- kill switch --

/// `MIKA_TELEGRAM_HTML_RENDER`, resolved once per process.
static HTML_RENDER: OnceLock<bool> = OnceLock::new();

/// Record the resolved kill-switch state. Called once from `main.rs` after
/// `GatewaySettings::load`.
///
/// Same contract as `MIKA_AGENT_TIER` and `MIKA_DEPLOYMENT`: read once per process,
/// **not hot-swappable**, to be placed in the service EnvironmentFile / ConfigMap
/// **before** startup. A second call is ignored — the first resolution wins, so a
/// stray re-init cannot silently flip the rendering mid-process.
pub(crate) fn init_html_render(enabled: bool) {
    let _ = HTML_RENDER.set(enabled);
    if !enabled {
        // Without this line, disarming is invisible: an operator reading "zero
        // fallbacks" would conclude the rendering works when it is not running at
        // all. The silence of a disarmed detector looks exactly like the silence of
        // a healthy one (mika#2205 doctrine).
        info!(
            event = "telegram_html_render_disabled",
            "MIKA_TELEGRAM_HTML_RENDER is disabled — outbound Telegram text is sent as plain text (markdown markers stripped, no parse_mode)"
        );
    }
}

/// Whether HTML rendering is armed. **Default armed**, including when
/// [`init_html_render`] was never called (test binaries, the `github.rs`
/// construction sites).
///
/// Armed by default against reflex caution, for a reason measured elsewhere:
/// mika#2272 had to observe that a detector shipped disarmed behind a flip
/// condition produces a counter that *structurally cannot* move — "zero was the
/// absence of measurement, not the presence of caution". Here the caution is paid by
/// the fallback, which is a mechanism and not an intention, and by the fallback event
/// that makes the failure rate countable from day one.
pub(crate) fn html_render_enabled() -> bool {
    match HTML_RENDER.get() {
        Some(armed) => *armed,
        None => true,
    }
}

// -- recognition --

/// Recognize the markdown an agent writes, and nothing else.
///
/// Recognized, per the closed table below; **everything else stays
/// [`SegmentKind::Plain`] verbatim** — `> quote`, `1. ` ordered lists, `---`,
/// four-space indentation, tables. That is the direct application of mika#2126's AC3
/// doctrine: a fix that rewrites a healthy message has repaired nothing, it has added
/// a second way to break it.
///
/// | Form | Segment | Condition |
/// |---|---|---|
/// | `**bold**` / `__bold__` | `Bold` | paired run, non-empty content |
/// | `*ital*` | `Italic` | paired run |
/// | `_ital_` | `Italic` | paired run **and** not intra-word |
/// | `~~struck~~` | `Strike` | paired run |
/// | `` `code` `` | `Code` | paired run |
/// | ` ```…``` ` | `Pre` | closing fence present; info-string dropped |
/// | `[label](url)` | `Link` | delegated to `parse_markdown_link`'s grammar |
/// | `# ` … `###### ` at line start | `Plain` minus the prefix | 1–6 `#` then a space |
/// | `* ` / `+ ` at line start | `Plain` with `- ` | bullet, normalized |
///
/// **The intra-word guard on `_` is load-bearing, not cosmetic.** Without it
/// `mon_fichier_test` becomes `monfichiertest`: user-data corruption shipped by a
/// cosmetic fix. The rule is CommonMark's — `_` is an emphasis delimiter only when it
/// is not flanked by alphanumerics on both sides.
pub(crate) fn tokenize(text: &str) -> Vec<Segment> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Segment> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;
    // Per-call memo of emphasis delimiters already proven to have no closer in the
    // rest of the text. See [`parse_emphasis`] for why this is what keeps the walk
    // linear rather than quadratic.
    let mut exhausted = [false; EXHAUSTED_SLOTS];

    while i < chars.len() {
        if i == 0 || chars[i - 1] == '\n' {
            if let Some(next) = heading_prefix_end(&chars, i) {
                i = next;
                continue;
            }
            if let Some(next) = bullet_prefix_end(&chars, i) {
                plain.push_str("- ");
                i = next;
                continue;
            }
        }

        if let Some((content, next)) = parse_fence(&chars, i) {
            flush_plain(&mut out, &mut plain);
            out.push(Segment {
                kind: SegmentKind::Pre,
                text: content,
            });
            i = next;
            continue;
        }

        if let Some((content, next)) = parse_inline_code(&chars, i) {
            flush_plain(&mut out, &mut plain);
            out.push(Segment {
                kind: SegmentKind::Code,
                text: content,
            });
            i = next;
            continue;
        }

        if chars[i] == '['
            && let Some((label, url, next)) = crate::telegram::parse_markdown_link(&chars, i)
        {
            flush_plain(&mut out, &mut plain);
            out.push(Segment {
                kind: SegmentKind::Link { url },
                text: label,
            });
            i = next;
            continue;
        }

        if let Some((kind, content, next)) = parse_emphasis(&chars, i, &mut exhausted) {
            flush_plain(&mut out, &mut plain);
            out.push(Segment {
                kind,
                text: content,
            });
            i = next;
            continue;
        }

        plain.push(chars[i]);
        i += 1;
    }

    flush_plain(&mut out, &mut plain);
    out
}

/// Move the accumulated verbatim run into a `Plain` segment. An empty run produces
/// no segment, so `tokenize("")` is `[]` and no rendering has to special-case it.
fn flush_plain(out: &mut Vec<Segment>, plain: &mut String) {
    if !plain.is_empty() {
        out.push(Segment {
            kind: SegmentKind::Plain,
            text: std::mem::take(plain),
        });
    }
}

/// `# ` … `###### ` at line start: index just past the space, or `None`.
///
/// Seven or more `#`, or a `#` not followed by a space (`#hashtag`), is not a
/// heading and stays verbatim.
fn heading_prefix_end(chars: &[char], start: usize) -> Option<usize> {
    let hashes = chars[start..].iter().take_while(|c| **c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    if chars.get(start + hashes) == Some(&' ') {
        Some(start + hashes + 1)
    } else {
        None
    }
}

/// `* ` or `+ ` at line start: index just past the space, or `None`. `- ` is already
/// the target form and needs no recognition.
fn bullet_prefix_end(chars: &[char], start: usize) -> Option<usize> {
    if matches!(chars.get(start), Some('*' | '+')) && chars.get(start + 1) == Some(&' ') {
        Some(start + 2)
    } else {
        None
    }
}

/// ```` ```…``` ```` starting at `start`: `(content, index just past the closing
/// fence)`.
///
/// The optional info-string (` ```rs `) is dropped, as is the newline that follows it
/// and one trailing newline before the closing fence. **No closing fence means no
/// recognition** — the backticks stay verbatim rather than swallowing the rest of the
/// message.
fn parse_fence(chars: &[char], start: usize) -> Option<(String, usize)> {
    if !is_run_of(chars, start, '`', 3) {
        return None;
    }
    let body_start = start + 3;
    // `is_run_of` bounds-checks with `.get()`, so the loop needs no arithmetic of
    // its own — one less off-by-one to get wrong.
    let close = (body_start..chars.len()).find(|j| is_run_of(chars, *j, '`', 3))?;
    let raw: String = chars[body_start..close].iter().collect();
    Some((strip_fence_decoration(&raw), close + 3))
}

/// Drop a fenced block's info-string and its bounding newlines.
fn strip_fence_decoration(raw: &str) -> String {
    let body = match raw.split_once('\n') {
        // A first line with no whitespace is an info-string (`rs`) or empty — drop
        // it. A first line carrying a space is code, so keep everything.
        Some((first, rest)) if !first.contains(char::is_whitespace) => rest,
        _ => raw,
    };
    body.strip_suffix('\n').unwrap_or(body).to_string()
}

/// `` `code` `` starting at `start`: `(content, index just past the closing
/// backtick)`. Empty content is refused, so a bare `` ` `` or a ``` ``` ``` with no
/// closer stays verbatim.
fn parse_inline_code(chars: &[char], start: usize) -> Option<(String, usize)> {
    if chars.get(start) != Some(&'`') {
        return None;
    }
    let content_start = start + 1;
    let close = chars
        .iter()
        .enumerate()
        .skip(content_start)
        .find(|(_, c)| **c == '`')
        .map(|(i, _)| i)?;
    if close == content_start {
        return None;
    }
    Some((chars[content_start..close].iter().collect(), close + 1))
}

/// A paired emphasis run starting at `start`: `(kind, content, index just past the
/// closing run)`.
///
/// Two flanking rules, which together are what keeps `2 * 3 * 4 = 24` and
/// `mon_fichier_test.rs` verbatim:
///
/// - an **opening** run must be immediately followed by a non-whitespace character;
/// - a **closing** run must be immediately preceded by a non-whitespace character.
///
/// A run longer than the delimiter it would open (`***x***`) is ambiguous and is
/// refused outright — inventing nesting is how a cosmetic fix rewrites a healthy
/// message. A single `~` is not a marker at all (`…/~vincent` is a legal URL path).
/// The search never crosses a blank line: emphasis that would span a paragraph break
/// is not emphasis, it is two unpaired markers.
///
/// **`exhausted` is what keeps this linear, and it is not an optimization.**
/// [`tokenize`] advances one character on a refusal, so without it a message made of
/// unpairable markers calls this function at every one of their positions and each
/// call scans to the end of the text: O(n²). The binding message length is **50 000
/// bytes**, not Telegram's 4096 — `handle_send` is the only length check and it caps
/// there (the 4096 mirror guard `mika-common` claims does not exist; see the gateway
/// `CLAUDE.md`). At that size the quadratic shape is seconds of synchronous CPU on
/// the runtime thread, per outbound message.
///
/// The memo is sound because the closing predicate reads only a candidate `j` and its
/// immediate neighbours, never the opener: for a later opener `start' > start` the
/// candidate range `start' + len .. n` is a **subset** of this one's. It is therefore
/// recorded **only when the scan exhausted the text** — a scan cut short by a
/// paragraph break says nothing about openers beyond that break, and memoizing it
/// would swallow legitimate emphasis in a later paragraph.
fn parse_emphasis(
    chars: &[char],
    start: usize,
    exhausted: &mut [bool; EXHAUSTED_SLOTS],
) -> Option<(SegmentKind, String, usize)> {
    let marker = *chars.get(start)?;
    if !matches!(marker, '*' | '_' | '~') {
        return None;
    }
    // Capped at 3: the match below distinguishes only 1 and 2 and refuses everything
    // else, so counting a 10 000-character run to the end just to reject it is work
    // whose result is already known.
    let run = chars[start..]
        .iter()
        .take(3)
        .take_while(|c| **c == marker)
        .count();
    let (len, kind) = match (marker, run) {
        ('*', 1) => (1, SegmentKind::Italic),
        ('*', 2) => (2, SegmentKind::Bold),
        ('_', 1) => (1, SegmentKind::Italic),
        ('_', 2) => (2, SegmentKind::Bold),
        ('~', 2) => (2, SegmentKind::Strike),
        _ => return None,
    };

    let slot = exhausted_slot(marker, len);
    if exhausted[slot] {
        return None;
    }

    // Left-flanking, plus the intra-word guard for `_`.
    let after_open = *chars.get(start + len)?;
    if after_open.is_whitespace() {
        return None;
    }
    if marker == '_' && is_intra_word(chars, start, len) {
        return None;
    }

    let content_start = start + len;
    let mut j = content_start;
    while j < chars.len() {
        if chars[j] == '\n' && chars.get(j + 1) == Some(&'\n') {
            return None; // paragraph break — refuse
        }
        if is_run_of(chars, j, marker, len)
            && chars.get(j + len) != Some(&marker)
            && j > content_start
            && !chars[j - 1].is_whitespace()
            && !(marker == '_' && is_intra_word(chars, j, len))
        {
            let content: String = chars[content_start..j].iter().collect();
            // `**[label](url)**` — mika#2126 has a test for this shape. Emphasis
            // wrapped around one whole link collapses to the link: in HTML mode a
            // `<b>` around raw `[label](url)` would leave the autolinker to swallow
            // the closing paren into the href.
            if let Some((label, url, next)) = whole_link(&content)
                && next == content.chars().count()
            {
                return Some((SegmentKind::Link { url }, label, j + len));
            }
            return Some((kind, content, j + len));
        }
        j += 1;
    }
    // Exhausted the text without a closer: no later opener of this (marker, len) can
    // find one either, since its candidate range is a subset of this one's.
    exhausted[slot] = true;
    None
}

/// Number of `(marker, len)` pairs `parse_emphasis` can open: `*`/`_`/`~` × len 1/2.
const EXHAUSTED_SLOTS: usize = 6;

/// Index of a `(marker, len)` pair in the `exhausted` memo. Total by construction —
/// an unexpected pair lands on slot 0, which can only cost a re-scan, never a wrong
/// recognition.
fn exhausted_slot(marker: char, len: usize) -> usize {
    let marker_index = match marker {
        '*' => 0,
        '_' => 1,
        _ => 2,
    };
    marker_index * 2 + usize::from(len == 2)
}

/// Whether `chars[at..at + len]` is exactly `len` copies of `marker`, with no
/// further copy immediately before.
fn is_run_of(chars: &[char], at: usize, marker: char, len: usize) -> bool {
    if at > 0 && chars.get(at - 1) == Some(&marker) {
        return false;
    }
    (0..len).all(|k| chars.get(at + k) == Some(&marker))
}

/// CommonMark's intra-word rule: a run flanked by alphanumerics on both sides is not
/// a delimiter. This is what keeps `mon_fichier_test` and
/// `…/path_with_underscore_` intact.
fn is_intra_word(chars: &[char], at: usize, len: usize) -> bool {
    let before = if at == 0 { None } else { chars.get(at - 1) };
    let after = chars.get(at + len);
    matches!((before, after), (Some(b), Some(a)) if b.is_alphanumeric() && a.is_alphanumeric())
}

/// Parse `text` as a single markdown link through the canonical grammar.
fn whole_link(text: &str) -> Option<(String, String, usize)> {
    let inner: Vec<char> = text.chars().collect();
    if inner.first() != Some(&'[') {
        return None;
    }
    crate::telegram::parse_markdown_link(&inner, 0)
}

// -- rendering --

/// The plain-text floor: concatenate the recognized text, markers gone.
///
/// A `Link` renders as `label : url` — **the exact form `rewrite_markdown_links`
/// already produces**, preserved deliberately: that form is frozen by mika#2126's
/// tests and the pipeline emits it today, so a second spelling of the same fact would
/// be a divergence in waiting. An empty label, or one identical to the URL, yields
/// the bare URL, again as today.
///
/// When `tokenize` returned a single `Plain`, this restores the input byte-for-byte —
/// by construction, not by luck. That is AC3.
pub(crate) fn render_plain(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match &segment.kind {
            SegmentKind::Link { url } => {
                if segment.text.is_empty() || segment.text == *url {
                    out.push_str(url);
                } else {
                    out.push_str(&segment.text);
                    out.push_str(" : ");
                    out.push_str(url);
                }
            }
            _ => out.push_str(&segment.text),
        }
    }
    out
}

/// The armed rendering: Telegram HTML.
///
/// Every `text` is escaped for `<`, `>` and `&` — **including inside `code` and
/// `pre`, which Telegram requires** — and so is a link's `href`. That escaping is a
/// security property, not a cosmetic one: without it, user text relayed by the agent
/// could plant arbitrary Telegram entities (`<b>déjà</b>` written by the agent must
/// arrive literal).
///
/// > **A healthy message is not byte-identical in HTML mode when it carries `&`, `<`
/// > or `>`** — it becomes `&amp;`, `&lt;`, `&gt;`. That is not an AC3 violation: the
/// > Telegram client restores the original character, so **what the user sees** is
/// > unchanged. AC3 applies to the perceived rendering, and the byte-for-byte control
/// > stays where it belongs, on [`render_plain`].
pub(crate) fn render_html(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        let text = escape_html(&segment.text);
        match &segment.kind {
            SegmentKind::Plain => out.push_str(&text),
            SegmentKind::Bold => push_wrapped(&mut out, "b", &text),
            SegmentKind::Italic => push_wrapped(&mut out, "i", &text),
            SegmentKind::Code => push_wrapped(&mut out, "code", &text),
            SegmentKind::Pre => push_wrapped(&mut out, "pre", &text),
            SegmentKind::Strike => push_wrapped(&mut out, "s", &text),
            SegmentKind::Link { url } => {
                let href = escape_html(url);
                // An empty (or url-equal) label would give a clickable nothing; use
                // the URL as its own label, which is what the plain floor's "bare
                // url" achieves.
                let label = if text.is_empty() || segment.text == *url {
                    href.clone()
                } else {
                    text
                };
                out.push_str("<a href=\"");
                out.push_str(&href);
                out.push_str("\">");
                out.push_str(&label);
                out.push_str("</a>");
            }
        }
    }
    out
}

fn push_wrapped(out: &mut String, tag: &str, text: &str) {
    out.push('<');
    out.push_str(tag);
    out.push('>');
    out.push_str(text);
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// Escape the three characters Telegram's HTML mode reserves. Single pass, so there
/// is no `&` / `&amp;` ordering hazard to get wrong.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// AC3's negative corpus (mika#2291 N1–N11): inputs a healthy pipeline must return
/// **byte-for-byte**.
///
/// Lives outside `mod tests` so `telegram.rs` can replay the same list through the
/// composition that is actually emitted (N12). One list, two readers — the same
/// reason `parse_markdown_link` was widened rather than copied: a second corpus would
/// drift, and the half that drifted would be the half nobody reruns.
#[cfg(test)]
pub(crate) const AC3_CORPUS: &[(&str, &str)] = &[
    ("N1", "Bonjour Sonia 🌸"),
    ("N2", "Ligne un\n\n  Va voir  https://example.com/a\tfin"),
    ("N3", "https://example.com/path_with_underscore_"),
    ("N4", "2 * 3 * 4 = 24"),
    ("N5", "> une citation"),
    ("N6", "1. premier\n2. second"),
    ("N7", "mon_fichier_test.rs"),
    ("N8a", "`"),
    ("N8b", "**"),
    ("N8c", "[label]("),
    ("N9", ""),
    ("N10", "Éh 🌸 https://example.com/été — ça va ?"),
    ("N11", "[x](https://example.com/a b)"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram::parse_agent_prefix;

    /// **No test calls [`init_html_render`], and that is deliberate.** `HTML_RENDER`
    /// is a process-wide `OnceLock` and the whole test binary is one process, so a
    /// single `init_html_render(false)` anywhere would poison
    /// [`html_render_enabled`] for every other test, with an outcome depending on
    /// test order. The disarmed path is covered where it is observable without a
    /// global: `telegram_html_render_is_enabled` in `settings.rs` (C1–C3) for the
    /// parse, and `mika2291_s1_*` in `telegram.rs` for the emitted bytes.
    ///
    /// C4 — never initialized (test binary) ⇒ armed, and no panic.
    #[test]
    fn mika2291_c4_uninitialized_kill_switch_is_armed() {
        assert!(
            html_render_enabled(),
            "the kill-switch must default to armed when never initialized"
        );
    }

    // -- Positives: recognition and both renderings (P1–P8) --

    #[test]
    fn mika2291_positive_recognition_table() {
        // (id, input, expected render_plain, expected render_html)
        let cases: &[(&str, &str, &str, &str)] = &[
            (
                "P1",
                "C'est **important** de le savoir.",
                "C'est important de le savoir.",
                "C'est <b>important</b> de le savoir.",
            ),
            ("P2", "un *mot* ital", "un mot ital", "un <i>mot</i> ital"),
            (
                "P3",
                "code `foo()` ici",
                "code foo() ici",
                "code <code>foo()</code> ici",
            ),
            ("P4", "~~annulé~~", "annulé", "<s>annulé</s>"),
            (
                "P5",
                "[le dépôt](https://example.com/a)",
                "le dépôt : https://example.com/a",
                "<a href=\"https://example.com/a\">le dépôt</a>",
            ),
            ("P6", "# Titre\ncorps", "Titre\ncorps", "Titre\ncorps"),
            (
                "P7",
                "* premier\n* second",
                "- premier\n- second",
                "- premier\n- second",
            ),
            (
                "P8",
                "```rs\nlet x = 1;\n```",
                "let x = 1;",
                "<pre>let x = 1;</pre>",
            ),
            // `__bold__` is the second spelling of bold the table names.
            ("P1b", "__gras__ ici", "gras ici", "<b>gras</b> ici"),
        ];
        for (id, input, plain, html) in cases {
            let segments = tokenize(input);
            assert_eq!(
                render_plain(&segments),
                *plain,
                "{id}: render_plain mismatch for {input:?}"
            );
            assert_eq!(
                render_html(&segments),
                *html,
                "{id}: render_html mismatch for {input:?}"
            );
        }
    }

    // -- Negatives (AC3): byte-for-byte on render_plain (N1–N11) --

    /// Each case asserts `render_plain(tokenize(x)) == x`, not "looks ok".
    ///
    /// A fix that rewrites a correct message has repaired nothing: it has added a
    /// second way to break it (mika#2126's AC3 doctrine). N2 is the one that rules a
    /// CommonMark parser out — it freezes double spaces **and** a tab.
    #[test]
    fn mika2291_ac3_healthy_message_passes_byte_for_byte() {
        for (id, input) in AC3_CORPUS {
            let out = render_plain(&tokenize(input));
            assert_eq!(out, *input, "{id}: AC3 violated on {input:?}");
        }
    }

    /// N11, separately: a link shape the grammar **refused** must not become an
    /// `<a href>` either. A link the recognizer invented on a form
    /// `parse_markdown_link` rejected would be worse than a raw marker.
    #[test]
    fn mika2291_n11_refused_link_grammar_yields_no_anchor() {
        let input = "[x](https://example.com/a b)";
        let segments = tokenize(input);
        assert!(
            !segments
                .iter()
                .any(|s| matches!(s.kind, SegmentKind::Link { .. })),
            "a refused link grammar produced a Link segment: {segments:?}"
        );
        assert!(!render_html(&segments).contains("<a "));
    }

    /// N13 — the `[agent] ` prefix survives both renderings (F9).
    ///
    /// `routes.rs` composes `format!("[{name}] {text}")` **before** the send, so the
    /// prefix traverses the tokenizer, and `resolve_reply_agent` reads it back off
    /// the quoted text to route the user's reply. The assertion goes through
    /// [`parse_agent_prefix`] itself rather than a hand-rewritten prefix equality,
    /// because it is *that* grammar — `strip_prefix('[')` then `split_once("] ")` —
    /// which decides the routing.
    ///
    /// This freezes a property that is **already true**, which is the whole point: it
    /// catches nothing today and turns red the day someone widens the recognizer to
    /// bare `[text]`. Without it, that widening would drop
    /// `resolve_reply_agent`'s primary route onto its DB fallback — a degradation
    /// that breaks nothing visible.
    #[test]
    fn mika2291_n13_agent_prefix_survives_both_renderings() {
        let input = "[mika-dev] C'est **important** de le savoir.";
        let segments = tokenize(input);
        for (mode, out) in [
            ("plain", render_plain(&segments)),
            ("html", render_html(&segments)),
        ] {
            assert!(
                out.starts_with("[mika-dev] "),
                "{mode}: agent prefix lost — {out:?}"
            );
            assert_eq!(
                parse_agent_prefix(&out).as_deref(),
                Some("mika-dev"),
                "{mode}: parse_agent_prefix no longer routes — {out:?}"
            );
        }
    }

    // -- HTML escaping (H1–H4) --

    #[test]
    fn mika2291_h1_reserved_characters_are_escaped() {
        let out = render_html(&tokenize("a < b & c > d"));
        assert_eq!(out, "a &lt; b &amp; c &gt; d");
        assert!(!out.contains("<b>"));
    }

    #[test]
    fn mika2291_h2_escaping_happens_inside_code() {
        let out = render_html(&tokenize("`if (a<b) {}`"));
        assert_eq!(out, "<code>if (a&lt;b) {}</code>");
    }

    #[test]
    fn mika2291_h3_ampersand_escaped_in_href_and_label() {
        let out = render_html(&tokenize("[a&b](https://x.test/?q=1&r=2)"));
        assert_eq!(out, "<a href=\"https://x.test/?q=1&amp;r=2\">a&amp;b</a>");
    }

    /// H4 is a **security** property, not a cosmetic one: without it, user text
    /// relayed by the agent could plant arbitrary Telegram entities.
    #[test]
    fn mika2291_h4_agent_written_tags_arrive_literal() {
        let out = render_html(&tokenize("<b>déjà</b>"));
        assert_eq!(out, "&lt;b&gt;déjà&lt;/b&gt;");
    }

    // -- Structural properties (S2, S3) --

    /// S2 — `tokenize` neither loses nor duplicates a non-marker character.
    #[test]
    fn mika2291_s2_tokenize_loses_no_content() {
        let cases: &[(&str, &str)] = &[
            (
                "C'est **important** de le savoir.",
                "C'est important de le savoir.",
            ),
            ("un *mot* et ~~autre~~", "un mot et autre"),
            ("`code` et **gras**", "code et gras"),
            ("# Titre\ncorps", "Titre\ncorps"),
        ];
        for (input, expected) in cases {
            let joined: String = tokenize(input).iter().map(|s| s.text.as_str()).collect();
            assert_eq!(joined, *expected, "S2 violated on {input:?}");
        }
    }

    /// S3 — a source scan, not a behavioural test.
    ///
    /// The regression this catches would not produce a wrong output; it would
    /// produce a **panicable send**, and every assertion about the output would stay
    /// green while an outbound message could abort the process.
    ///
    /// Two scoping rules, each of which the first run of this test proved necessary:
    ///
    /// - **The production half only** (everything above `mod tests`). A test
    ///   asserting with `assert_eq!` is not a fallibility in the send path.
    /// - **Code, never prose.** Line comments are stripped first, because the module
    ///   doc-comment *names* the constructs it forbids — a guard that could not
    ///   tolerate being described would force the documentation to go quiet about
    ///   the very rule it carries. (Block comments `/* … */` are not stripped; this
    ///   file uses none, and a scan that tried to parse them would be a tokenizer.)
    #[test]
    fn mika2291_s3_render_is_infallible_by_construction() {
        let source = include_str!("telegram_markdown.rs");
        let production = match source.find("\n#[cfg(test)]") {
            Some(at) => &source[..at], // safe-byte-slice: `at` comes from str::find, which only ever returns a char boundary
            None => source,
        };
        let code: String = production
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        for forbidden in [".unwrap()", ".expect(", "panic!", "unreachable!", "todo!"] {
            assert!(
                !code.contains(forbidden),
                "telegram_markdown.rs must be infallible by construction, found {forbidden:?}"
            );
        }
        // Raw byte indexing into text. Every split in this module is over a
        // `Vec<char>`; a `&str` range slice is the mika#764 hazard.
        for forbidden in ["&text[", "&raw[", "&body[", "text[..", "text[0.."] {
            assert!(
                !code.contains(forbidden),
                "telegram_markdown.rs must not index text by byte offset, found {forbidden:?}"
            );
        }

        // Anti-vacuity: the scan must be able to see a violation at all. Without
        // this, a stripper bug that emptied `code` would leave every assertion
        // above trivially green.
        assert!(
            code.contains("fn tokenize"),
            "the scan lost the production code it is supposed to read"
        );
        assert!(
            format!("{code}\nlet _ = x.unwrap();").contains(".unwrap()"),
            "the scan cannot detect the pattern it forbids"
        );
    }

    // -- Edge shapes the recognition table refuses on purpose --

    #[test]
    fn mika2291_unclosed_fence_stays_verbatim() {
        let input = "```rs\nlet x = 1;";
        assert_eq!(render_plain(&tokenize(input)), input);
    }

    #[test]
    fn mika2291_triple_emphasis_run_is_ambiguous_and_refused() {
        // `***x***` — inventing nesting is how a cosmetic fix rewrites a healthy
        // message. Refuse rather than guess.
        let input = "***x***";
        assert_eq!(render_plain(&tokenize(input)), input);
    }

    #[test]
    fn mika2291_hashtag_and_seven_hashes_are_not_headings() {
        for input in ["#hashtag", "####### sept"] {
            assert_eq!(render_plain(&tokenize(input)), input, "on {input:?}");
        }
    }

    #[test]
    fn mika2291_emphasis_never_crosses_a_paragraph_break() {
        let input = "un *mot\n\nautre* fin";
        assert_eq!(render_plain(&tokenize(input)), input);
    }

    /// `**[label](url)**` — the shape mika#2126 already has a test for. Emphasis
    /// wrapped around one whole link collapses to the link, because a `<b>` around
    /// raw `[label](url)` would leave Telegram's autolinker to swallow the closing
    /// paren into the href.
    #[test]
    fn mika2291_bold_wrapped_link_collapses_to_the_link() {
        let segments = tokenize("**[le dépôt](https://example.com/a)**");
        assert_eq!(
            render_html(&segments),
            "<a href=\"https://example.com/a\">le dépôt</a>"
        );
        assert_eq!(render_plain(&segments), "le dépôt : https://example.com/a");
    }

    #[test]
    fn mika2291_empty_and_url_equal_labels_render_the_bare_url() {
        let url = "https://example.com/a";
        for input in [format!("[]({url})"), format!("[{url}]({url})")] {
            let segments = tokenize(&input);
            assert_eq!(render_plain(&segments), url, "on {input:?}");
            assert_eq!(
                render_html(&segments),
                format!("<a href=\"{url}\">{url}</a>"),
                "on {input:?}"
            );
        }
    }

    /// The residual risk mika#2126 named and accepted — `_texte https://url_`, whose
    /// run is unpaired at *token* level so its auxiliary leaves a `_` glued. The
    /// recognizer pairs it at *text* level, so both renderings now hand Telegram a
    /// bounded URL. Recorded because it is an improvement on a documented limit, not
    /// a new requirement.
    #[test]
    fn mika2291_phrase_wide_italics_around_a_url_is_now_paired() {
        let segments = tokenize("_texte https://example.com/a_");
        assert_eq!(render_plain(&segments), "texte https://example.com/a");
        assert_eq!(render_html(&segments), "<i>texte https://example.com/a</i>");
    }

    /// The `exhausted` memo must not swallow legitimate emphasis — and the case that
    /// decides it is the paragraph break.
    ///
    /// `*a` opens, its scan stops at the blank line, and that refusal says **nothing**
    /// about openers past the break. Memoizing it (instead of only memoizing a scan
    /// that reached the end of the text) would lose the `*b*` below — a false negative
    /// created by a performance fix, which is the worst shape such a fix can take.
    #[test]
    fn mika2291_exhausted_memo_does_not_swallow_later_emphasis() {
        let segments = tokenize("*a\n\nun *mot* fin");
        assert_eq!(render_plain(&segments), "*a\n\nun mot fin");
        assert_eq!(render_html(&segments), "*a\n\nun <i>mot</i> fin");
    }

    /// The same, for an opener whose scan really does exhaust the text: nothing after
    /// it can pair, so nothing is lost.
    #[test]
    fn mika2291_exhausted_memo_is_transparent_when_nothing_can_pair() {
        let input = "un *mot et *encore et *toujours";
        assert_eq!(render_plain(&tokenize(input)), input);
    }

    /// The walk is linear, not quadratic, on markers that never pair.
    ///
    /// **Why this guard exists at all.** `tokenize` advances one character when it
    /// refuses, so before the `exhausted` memo a text of unpairable markers scanned to
    /// the end of the string at every marker position. The binding length here is
    /// **50 000 bytes** — `handle_send` is the only length check in the gateway and it
    /// caps there, Telegram's 4096 being enforced nowhere (see the gateway
    /// `CLAUDE.md`) — so the quadratic shape was seconds of synchronous CPU on the
    /// runtime thread, per outbound message.
    ///
    /// It asserts correctness first (the pathological inputs still round-trip
    /// byte-for-byte) and then a wall-clock ceiling. **The ceiling is a complexity
    /// smoke-test, not a benchmark**, and it is placed against a measurement rather
    /// than a feeling: on this corpus, a debug build takes **0.04 s** with the memo
    /// and **11.6 s** with it disabled — a factor of ~290. The 2 s ceiling therefore
    /// sits ~50× above the linear cost and ~6× below the quadratic one, which
    /// separates the two behaviours without being sensitive to machine speed or to a
    /// loaded CI runner.
    ///
    /// Those same figures are the measurement that makes this a defect rather than a
    /// tidy-up: 11.6 s for five messages is ~2.3 s of synchronous CPU on the runtime
    /// thread for **one** 50 000-character message.
    #[test]
    fn mika2291_tokenize_stays_linear_on_unpairable_markers() {
        let n = 50_000;
        let pathological = [
            "*".repeat(n),
            "_".repeat(n),
            "~".repeat(n),
            "a* ".repeat(n / 3),
            "_a ".repeat(n / 3),
        ];
        let started = std::time::Instant::now();
        for input in &pathological {
            assert_eq!(
                render_plain(&tokenize(input)),
                *input,
                "a pathological marker run must still round-trip byte-for-byte"
            );
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "tokenize looks superlinear on unpairable markers: {elapsed:?} for \
             {} inputs of ~{n} chars",
            pathological.len()
        );
    }

    /// The founding case of mika#2291, frozen. The input carries `**gras**`; neither
    /// rendering may leave a `*` behind. This is the assertion that describes the
    /// measured symptom and that turns red if the fix is undone.
    #[test]
    fn mika2291_reported_case_raw_bold_is_no_longer_visible() {
        let input = "C'est **important** de le savoir.";
        let segments = tokenize(input);
        let html = render_html(&segments);
        let plain = render_plain(&segments);
        assert!(!html.contains('*'), "html still carries a marker: {html:?}");
        assert!(
            !plain.contains('*'),
            "plain still carries a marker: {plain:?}"
        );
        assert_eq!(html, "C'est <b>important</b> de le savoir.");
        assert_eq!(plain, "C'est important de le savoir.");
    }
}
