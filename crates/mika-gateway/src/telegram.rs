use bytes::Bytes;
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Typed error for Telegram Bot API responses, following the ClaudeApiError pattern.
#[derive(Debug, thiserror::Error)]
pub enum TelegramApiError {
    #[error("rate limited")]
    RateLimited { retry_after: Option<u64> },
    #[error("bot blocked by user")]
    BotBlocked,
    #[error("bad request: {message}")]
    BadRequest { message: String },
    #[error("unauthorized — check MIKA_TELEGRAM_BOT_TOKEN")]
    Unauthorized,
    #[error("telegram api error ({status})")]
    Other { status: u16, body: String },
    #[error("network error")]
    Network(#[from] reqwest::Error),
}

// -- Telegram Update types --

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TelegramMessage {
    pub chat: TelegramChat,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub photo: Option<Vec<PhotoSize>>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub document: Option<TelegramDocument>,
    #[serde(default)]
    pub reply_to_message: Option<ReplyToMessage>,
}

/// Replied-to message context for reply routing.
/// Telegram sends the full original message; we capture `message_id` for DB lookup
/// and `text` for parsing the `[agent_name]` prefix.
#[derive(Debug, Clone, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct ReplyToMessage {
    pub message_id: i64,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TelegramChat {
    pub id: i64,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[allow(dead_code)]
pub struct PhotoSize {
    pub file_id: String,
    pub file_unique_id: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[allow(dead_code)]
pub struct TelegramDocument {
    pub file_id: String,
    pub file_unique_id: String,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
}

// -- Parsed message result --

#[derive(Debug, PartialEq)]
pub enum ParsedMessage {
    Start {
        chat_id: i64,
        pairing_token: String,
    },
    Text {
        chat_id: i64,
        text: String,
        update_id: i64,
        reply_to_message_id: Option<i64>,
        reply_to_text: Option<String>,
    },
    Photo {
        chat_id: i64,
        file_id: String,
        caption: Option<String>,
        update_id: i64,
        reply_to_message_id: Option<i64>,
        reply_to_text: Option<String>,
    },
    Document {
        chat_id: i64,
        file_id: String,
        mime_type: String,
        caption: Option<String>,
        update_id: i64,
        reply_to_message_id: Option<i64>,
        reply_to_text: Option<String>,
    },
    BareStart {
        chat_id: i64,
    },
    /// `/unlink` — request self-unlink of the paired Telegram binding (mika#1749).
    /// Also produced when the user typed `/unlink <anything>` with a suffix we
    /// don't recognize; the handler shows the warning and prompts the user to
    /// send `/unlink confirm`.
    Unlink {
        chat_id: i64,
    },
    /// `/unlink confirm` — commit the self-unlink (mika#1749). Atomic UPDATE
    /// releases `telegram_chat_id`.
    UnlinkConfirm {
        chat_id: i64,
    },
    Unsupported {
        chat_id: i64,
    },
    NoMessage,
}

/// Image MIME types supported for forwarding to the agent.
const SUPPORTED_IMAGE_MIMES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

/// Parse `[agent_name]` prefix from message text.
/// Returns the agent name if the text starts with `[name] ` where name matches
/// the agent naming convention (lowercase alphanumeric + hyphens, 1-32 chars).
pub fn parse_agent_prefix(text: &str) -> Option<String> {
    let rest = text.strip_prefix('[')?;
    let (name, _) = rest.split_once("] ")?;
    if name.is_empty() || name.len() > 32 {
        return None;
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return None;
    }
    Some(name.to_string())
}

/// Parse a Telegram update into a structured message type.
pub fn parse_update(update: &TelegramUpdate) -> ParsedMessage {
    let message = match &update.message {
        Some(m) => m,
        None => return ParsedMessage::NoMessage,
    };

    let chat_id = message.chat.id;
    let reply_to_message_id = message.reply_to_message.as_ref().map(|r| r.message_id);
    let reply_to_text = message
        .reply_to_message
        .as_ref()
        .and_then(|r| r.text.clone());

    // Text messages (including /start commands) take priority
    if let Some(text) = &message.text {
        if text == "/start" {
            return ParsedMessage::BareStart { chat_id };
        }
        if let Some(payload) = text.strip_prefix("/start ") {
            let token = payload.trim();
            if token.is_empty() {
                return ParsedMessage::Unsupported { chat_id };
            }
            return ParsedMessage::Start {
                chat_id,
                pairing_token: token.to_string(),
            };
        }
        // /unlink family (mika#1749). Canonicalize whitespace so `/unlink   confirm`
        // parses the same as `/unlink confirm`. Only exact `/unlink` and
        // `/unlink confirm` match; a stray suffix (typo) falls back to the warning
        // path. `/unlinkxxx` (no space after `/unlink`) does NOT match — it's not
        // our command and gets forwarded to the agent as free text.
        if text == "/unlink" || text.starts_with("/unlink ") {
            let canonical: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if canonical == "/unlink confirm" {
                return ParsedMessage::UnlinkConfirm { chat_id };
            }
            return ParsedMessage::Unlink { chat_id };
        }
        return ParsedMessage::Text {
            chat_id,
            text: text.clone(),
            update_id: update.update_id,
            reply_to_message_id,
            reply_to_text: reply_to_text.clone(),
        };
    }

    // Photo messages: pick the largest photo (last in the array)
    if let Some(photos) = &message.photo
        && let Some(largest) = photos.last()
    {
        return ParsedMessage::Photo {
            chat_id,
            file_id: largest.file_id.clone(),
            caption: message.caption.clone(),
            update_id: update.update_id,
            reply_to_message_id,
            reply_to_text: reply_to_text.clone(),
        };
    }

    // Document messages: only forward image documents
    if let Some(doc) = &message.document
        && let Some(mime) = &doc.mime_type
        && SUPPORTED_IMAGE_MIMES.contains(&mime.as_str())
    {
        return ParsedMessage::Document {
            chat_id,
            file_id: doc.file_id.clone(),
            mime_type: mime.clone(),
            caption: message.caption.clone(),
            update_id: update.update_id,
            reply_to_message_id,
            reply_to_text,
        };
    }

    ParsedMessage::Unsupported { chat_id }
}

// -- Telegram API response types --

#[derive(Debug, Deserialize)]
struct TelegramResponse {
    ok: bool,
    description: Option<String>,
    parameters: Option<TelegramResponseParameters>,
}

#[derive(Debug, Deserialize)]
struct TelegramResponseParameters {
    retry_after: Option<u64>,
}

/// Response from Telegram `sendMessage` API (success path).
#[derive(Debug, Deserialize)]
struct TelegramSendResponse {
    result: Option<TelegramSendResult>,
}

#[derive(Debug, Deserialize)]
struct TelegramSendResult {
    message_id: i64,
}

/// Response from Telegram `getFile` API.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GetFileResponse {
    ok: bool,
    result: Option<TelegramFile>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramFile {
    pub file_path: Option<String>,
}

/// Maximum image file size we'll download (5 MB).
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Result of downloading and validating an image from Telegram.
#[derive(Debug)]
pub struct DownloadedImage {
    pub data: Bytes,
    pub media_type: String,
}

/// Detect media type from magic bytes. Returns None if unrecognized.
pub fn detect_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 12 {
        return None;
    }
    if bytes[0..3] == [0xFF, 0xD8, 0xFF] {
        Some("image/jpeg")
    } else if bytes[0..4] == [0x89, 0x50, 0x4E, 0x47] {
        Some("image/png")
    } else if bytes[0..4] == [0x47, 0x49, 0x46, 0x38] {
        Some("image/gif")
    } else if bytes[0..4] == [0x52, 0x49, 0x46, 0x46] && bytes[8..12] == [0x57, 0x45, 0x42, 0x50] {
        Some("image/webp")
    } else {
        None
    }
}

// -- sendMessage payload --

/// **`parse_mode` is `Some("HTML")` or nothing, never MarkdownV2 (mika#2126, then
/// mika#2291).**
///
/// **mika#2126's argument is preserved here because it is still correct**, and
/// erasing it would make the next attempt at MarkdownV2 as expensive as it was the
/// first time. It ran: MarkdownV2 requires escaping
/// `_ * [ ] ( ) ~ ` > # + - = | { } . !` throughout the *entire* text, URLs
/// included; a single unescaped character makes the Telegram API reject the **whole
/// message** with a 400; we would then have traded a broken link for an **absent
/// message** — a clear regression, because today the user at least receives the text.
/// That argument is unchanged, and MarkdownV2 stays rejected on it.
///
/// **What mika#2291 changed is its premise, not its reasoning.** The argument holds
/// against `parse_mode` **bare**. Three things make HTML tenable where MarkdownV2 was
/// not:
///
/// 1. **Escaping surface.** Telegram's HTML mode reserves three characters — `<`,
///    `>`, `&` — and only outside tags. Escaping is *local* (escape the text content,
///    emit the tags ourselves) instead of *global*, so the error surface is an order
///    of magnitude smaller.
/// 2. **The fallback.** On a 400 while `parse_mode` was set, [`send_message_impl`]
///    re-sends **once, without `parse_mode`, with the plain text**. So the worst case
///    of the HTML path is *exactly* today's behaviour, minus the raw markers: a
///    message can no longer be lost because of a rendering. The trade mika#2126
///    refused no longer exists — a raw marker is traded for a raw marker.
/// 3. **The kill-switch.** `MIKA_TELEGRAM_HTML_RENDER=0` restores a byte-identical
///    plain path with no redeploy, and it is the *same* code path as the fallback,
///    so a rollback takes a road that already runs.
///
/// Recognition and both renderings live in [`crate::telegram_markdown`];
/// [`strip_markdown_around_urls`] is unchanged and kept as a second pass on the plain
/// path only (see [`plain_body`]).
///
/// Changing this field still means owning the escaping of every outbound message.
/// Take a *third* mode back through grooming, not here.
#[derive(Debug, Serialize)]
struct SendMessagePayload {
    chat_id: i64,
    text: String,
    /// `&'static str` rather than `String`: the only possible values are `"HTML"`
    /// and absence, and the type forbids inventing a third one at a call site.
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<&'static str>,
}

// -- setWebhook payload --

#[derive(Debug, Serialize)]
struct SetWebhookPayload {
    url: String,
    secret_token: String,
    allowed_updates: Vec<String>,
    max_connections: u32,
}

// -- Shared helpers used by both TelegramClient and CustomerTelegramClient --

/// Build a Telegram Bot API method URL for the given token and method.
fn api_url(bot_token: &str, method: &str) -> String {
    format!("https://api.telegram.org/bot{bot_token}/{method}")
}

/// Validate a file_path returned by Telegram's `getFile` API.
///
/// Rejects empty paths, leading slashes, path traversal (`..`), and URL
/// manipulation characters (`@`, `?`, `#`) that could redirect the download
/// request and leak the bot token embedded in the URL.
fn validate_file_path(file_path: &str) -> Result<(), TelegramApiError> {
    if file_path.is_empty() {
        return Err(TelegramApiError::BadRequest {
            message: "invalid file_path from Telegram API: empty".to_string(),
        });
    }
    if file_path.starts_with('/') {
        return Err(TelegramApiError::BadRequest {
            message: "invalid file_path from Telegram API: starts with /".to_string(),
        });
    }
    if file_path.contains("..") {
        return Err(TelegramApiError::BadRequest {
            message: "invalid file_path from Telegram API: contains ..".to_string(),
        });
    }
    for ch in ['@', '?', '#'] {
        if file_path.contains(ch) {
            return Err(TelegramApiError::BadRequest {
                message: format!("invalid file_path from Telegram API: contains '{ch}'"),
            });
        }
    }
    Ok(())
}

/// Strip markdown decoration that is glued to URLs in outgoing Telegram text (mika#2126).
///
/// **Why this exists.** We send plain text — [`SendMessagePayload`] deliberately has no
/// `parse_mode` (see the note there). The agent writes markdown because that is its
/// default register, so `**https://…/mika**` reaches Telegram verbatim. Telegram's
/// autolinker stops at whitespace, not at markdown, so it swallows the trailing
/// asterisks into the link and the user lands on `…/mika**` → 404.
///
/// **Anchor.** The perimeter is *URLs*, not markdown rendering. Nothing happens to a
/// message that carries no `http://` / `https://` scheme, and nothing happens to
/// decoration that is not glued to a scheme-bearing token. A cleaner that rewrote
/// healthy messages would have repaired nothing — it would have added a second way to
/// break them.
///
/// **The rule**, in two passes:
///
/// 1. **Markdown links.** `[label](url)` where `url` starts with a scheme, has no
///    whitespace and no nested brackets or parentheses → rewritten as `label : url`,
///    so the URL ends the sequence and is bounded by whitespace. An empty label, or a
///    label identical to the url, yields the bare url. Any other shape is left intact.
/// 2. **Border decoration.** On each whitespace-delimited token that carries a scheme,
///    remove at the borders:
///    - `*` and `` ` `` — **unconditionally**. A real URL practically never ends in an
///      asterisk, and a backtick would have to be percent-encoded anyway.
///    - `_` and `~` — **only when paired**, i.e. when the same run borders both ends
///      (`_url_`, `~~url~~`). Both are legal URL characters, so a lone trailing `_`
///      (`…/foo_`) is part of the URL and stays.
///
/// **The ambiguity is irreducible, and it is the whole reason for the bug.** `*`, `_`
/// and `~` are legal in a URL (`*` is a sub-delimiter, `_` and `~` are unreserved), so
/// no URL grammar can tell `…/mika**` (URL + decoration) from `…/mika**` (a URL that
/// genuinely ends in two asterisks) — which is precisely why Telegram's autolinker
/// gets it wrong too. No algorithm here can be both complete and safe; this one
/// chooses safe.
///
/// **Named residual risk.** `_texte https://url_` — italics wrapped around a whole
/// phrase — leaves a `_` glued, because the run is not paired at the *token* level.
/// Accepted: the observed and overwhelmingly common shape is `**url**`, covered
/// unconditionally. Widening this would require a real markdown parser, which is
/// exactly what "out of scope" rules out.
///
/// **Infallible by construction.** No `Result`, no `unwrap`, no panic, no raw byte
/// indexing (all slicing is over `Vec<char>`, never `&text[i..j]`). A cleaner that
/// could fail could block a send, and a message that never arrives is strictly worse
/// than a broken link — that is the defect for which the MarkdownV2 route was rejected.
fn strip_markdown_around_urls(text: &str) -> String {
    if !text.contains("http://") && !text.contains("https://") {
        return text.to_string();
    }
    strip_border_decoration(&rewrite_markdown_links(text))
}

/// Pass 1 of [`strip_markdown_around_urls`]: rewrite `[label](url)` as `label : url`.
fn rewrite_markdown_links(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '['
            && let Some((label, url, next)) = parse_markdown_link(&chars, i)
        {
            if label.is_empty() || label == url {
                out.push_str(&url);
            } else {
                out.push_str(&label);
                out.push_str(" : ");
                out.push_str(&url);
            }
            i = next;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Parse `[label](url)` starting at `open` (which must be `[`).
///
/// Returns `(label, url, index just past the closing paren)`, or `None` for any shape
/// that is not an unambiguous scheme-bearing link — nesting, whitespace in the url, a
/// relative target. Refusing is always safe here; rewriting a shape we misread is not.
///
/// **Visibility widened to `pub(crate)` by mika#2291, body byte-identical.**
/// `telegram_markdown::tokenize` delegates its `[label](url)` recognition here rather
/// than re-spelling the grammar: a second copy is the divergence-in-waiting the repo
/// has had to undo twice (mika#2158, mika#2120). One grammar, two consumers.
pub(crate) fn parse_markdown_link(chars: &[char], open: usize) -> Option<(String, String, usize)> {
    let label_start = open + 1;
    let mut close_bracket = None;
    for (i, c) in chars.iter().enumerate().skip(label_start) {
        match c {
            '[' => return None, // nested bracket — refuse
            ']' => {
                close_bracket = Some(i);
                break;
            }
            _ => {}
        }
    }
    let close_bracket = close_bracket?;
    if chars.get(close_bracket + 1) != Some(&'(') {
        return None;
    }

    let url_start = close_bracket + 2;
    let mut close_paren = None;
    for (i, c) in chars.iter().enumerate().skip(url_start) {
        match c {
            '(' => return None, // nested paren — refuse
            ')' => {
                close_paren = Some(i);
                break;
            }
            _ => {}
        }
    }
    let close_paren = close_paren?;

    let url: String = chars[url_start..close_paren].iter().collect();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return None;
    }
    if url
        .chars()
        .any(|c| c.is_whitespace() || c == '[' || c == ']')
    {
        return None;
    }

    let label: String = chars[label_start..close_bracket].iter().collect();
    Some((label, url, close_paren + 1))
}

/// Pass 2 of [`strip_markdown_around_urls`]: strip border decoration from every
/// whitespace-delimited token that carries a scheme.
///
/// Whitespace is copied through verbatim, character by character, so a message with
/// nothing to clean comes out byte-for-byte identical (AC3).
fn strip_border_decoration(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        out.push_str(&strip_token_borders(&chars[start..i]));
    }
    out
}

/// Strip decoration from the borders of one token, per pass 2 of the rule.
///
/// Loops to a fixed point so nesting resolves in either order (`**_url_**` and
/// `_**url**_` both unwrap). Termination is guaranteed: each round either removes at
/// least one character or stops.
fn strip_token_borders(token: &[char]) -> String {
    if !token.contains(&':') {
        return token.iter().collect();
    }
    let flat: String = token.iter().collect();
    if !flat.contains("http://") && !flat.contains("https://") {
        return flat;
    }

    let mut t: Vec<char> = token.to_vec();
    loop {
        let before = t.len();

        // `*` and backtick: unconditional at both borders.
        while matches!(t.first(), Some('*' | '`')) {
            t.remove(0);
        }
        while matches!(t.last(), Some('*' | '`')) {
            t.pop();
        }

        // `_` and `~`: only when the same run borders both ends.
        for marker in ['_', '~'] {
            let lead = t.iter().take_while(|c| **c == marker).count();
            let trail = t.iter().rev().take_while(|c| **c == marker).count();
            let n = lead.min(trail);
            if n > 0 && lead + trail <= t.len() {
                t.drain(0..n);
                t.truncate(t.len() - n); // safe-byte-slice: t is a Vec<char> (`let mut t: Vec<char> = token.to_vec()` above) — Vec::truncate cuts by element and t.len() counts chars, not bytes; there is no UTF-8 boundary to violate
            }
        }

        if t.len() == before {
            break;
        }
    }
    t.iter().collect()
}

/// The plain-text floor — **the single construction site of the plain body**
/// (mika#2291).
///
/// Both the disarmed mode and the fallback path call this one function, which is what
/// makes "disarming and the fallback produce the same byte" a property of the code
/// rather than of a test corpus. A second spelling of this composition would be the
/// divergence that makes a rollback take an untested road.
///
/// `strip_markdown_around_urls` runs **after** `render_plain`, and only here. By then
/// the recognized markers are gone, so it almost never finds anything to do (it
/// returns early without a URL scheme). It stays for the residue its own doc-comment
/// names — `_texte https://url_`, decoration unpaired at token level — that the
/// recognizer deliberately leaves `Plain`. Cost: one pass over a string.
///
/// > **Why it does not also run before `render_html`.** That would be the symmetric
/// > reflex and it is wrong: the auxiliary rewrites `[label](url)` as `label : url`,
/// > which would destroy the `Link` that `render_html` must emit as `<a href>`. The
/// > two transformations overlap instead of composing — hence one recognizer
/// > upstream and the net downstream of the only rendering that tolerates it.
/// >
/// > Consequence, stated because it is narrower than it looks: the benefit "a
/// > recognizer regression cannot reopen mika#2126" holds for the **floor**, not for
/// > the armed mode — and the armed mode is the default. In HTML mode what holds the
/// > founding defect is the recognizer itself (paired decoration around a URL becomes
/// > a tag, so the link's boundary is the tag; unpaired decoration stays `Plain`, so
/// > the URL passes intact). `mika2291_r5_*` controls those two properties for
/// > themselves.
///
/// `chat_id` is carried for the metrics line only. It is a parameter rather than
/// something this function could derive, because the pre-mika#2291 `debug!` lived
/// inline in [`send_message_impl`] where `chat_id` was in scope: dropping the field
/// when the composition was extracted would have quietly made the counter
/// unattributable to a chat.
fn plain_body(chat_id: i64, text: &str) -> String {
    let plain = crate::telegram_markdown::render_plain(&crate::telegram_markdown::tokenize(text));
    let cleaned = strip_markdown_around_urls(&plain);
    if cleaned != plain {
        // Metrics only — never the message body (user data). Without this counter,
        // "the rule stopped firing because the agent stopped decorating" and "the
        // rule stopped matching" look identical from the gateway. `len_before` is the
        // text as the auxiliary received it (post-`render_plain`), which is what
        // keeps the metric meaning "the URL auxiliary acted".
        let url_tokens = plain
            .split_whitespace()
            .filter(|t| t.contains("http://") || t.contains("https://"))
            .count();
        debug!(
            chat_id,
            len_before = plain.len(),
            len_after = cleaned.len(),
            url_tokens,
            "stripped markdown decoration glued to outbound URL(s)"
        );
    }
    cleaned
}

/// Whether a failed HTML send may be replayed as plain text (mika#2291).
///
/// **The trigger is the status, never a substring of the `description`.** The repo
/// has already had to settle this class: mika#2179's error classes come from the
/// `LlmError` *variant* via `downcast_ref`, "never from a substring match on the
/// rendered message".
///
/// A 400 while `parse_mode` was set is *by definition* a case where removing
/// `parse_mode` cannot hurt: if the cause was elsewhere (chat not found, text too
/// long), the second attempt fails identically and that second error is returned —
/// cost one API call, no degradation. 401 / 403 / 429 / 5xx are returned as-is:
/// replaying them without `parse_mode` would double a doomed call and, on 429, worsen
/// the rate limit.
///
/// Decoupled from the call on purpose. `api_url` hard-codes `https://api.telegram.org`
/// and making it injectable for a mock server would widen the production surface for
/// test-only observability (declared out of scope); the decision is what needs
/// pinning, and it is pinned here.
fn should_fall_back_to_plain(err: &TelegramApiError) -> bool {
    matches!(err, TelegramApiError::BadRequest { .. })
}

/// Send a text message to a chat via the Telegram Bot API.
///
/// Shared implementation for both client types — the **single** `sendMessage` call
/// site of the crate, which is the property mika#2126 built deliberately ("every
/// present and future caller inherits the cleaning here") and that mika#2291 plugs
/// into without moving.
///
/// One recognition, then either rendering:
///
/// - **armed** (default): `render_html` with `parse_mode = Some("HTML")`; on a 400,
///   warn and fall through to the floor. Any other error is returned.
/// - **floor** (disarmed mode *and* the fallback, byte-identical): [`plain_body`]
///   with no `parse_mode`.
///
/// Returns the Telegram message_id on success.
async fn send_message_impl(
    client: &Client,
    bot_token: &str,
    chat_id: i64,
    text: &str,
) -> Result<i64, TelegramApiError> {
    let mut html_failed_with: Option<String> = None;

    if crate::telegram_markdown::html_render_enabled() {
        let html = crate::telegram_markdown::render_html(&crate::telegram_markdown::tokenize(text));
        // `len_html` is captured up front (O(1)) rather than keeping `html` alive for
        // the log: the fallback arm is the expected-empty population, so cloning the
        // whole rendered body to serve it would allocate and copy on 100 % of
        // successful sends for a field that almost never gets read.
        let len_html = html.len();
        match post_send_message(client, bot_token, chat_id, html, Some("HTML")).await {
            Ok(message_id) => return Ok(message_id),
            Err(err) if should_fall_back_to_plain(&err) => {
                let description = match &err {
                    TelegramApiError::BadRequest { message } => message.clone(),
                    _ => String::new(),
                };
                // Expected regime: zero lines. Any occurrence is a message the HTML
                // rendering broke and the fallback saved — both proof the net works
                // and a recognizer case to fix. The body is NEVER logged (user
                // data), at mika#2126's standard.
                warn!(
                    event = "telegram_html_render_fallback",
                    chat_id,
                    description = %description,
                    len_html,
                    "telegram rejected the HTML rendering — re-sending as plain text"
                );
                html_failed_with = Some(description);
            }
            Err(err) => return Err(err),
        }
    }

    let body = plain_body(chat_id, text);
    let len_plain = body.len();
    let result = post_send_message(client, bot_token, chat_id, body, None).await;

    if let (Some(description), Err(err)) = (&html_failed_with, &result) {
        // Expected regime: zero lines. This population is the one where the user
        // actually loses the message; without this event it would be
        // indistinguishable from an ordinary 502.
        warn!(
            event = "telegram_html_fallback_failed",
            chat_id,
            description = %description,
            len_plain,
            error = %err,
            "plain-text fallback failed too — the user received nothing"
        );
    }

    result
}

/// POST one `sendMessage` and map the Telegram status to a [`TelegramApiError`].
///
/// Extracted from [`send_message_impl`] by mika#2291 so the HTML attempt and the
/// plain one share a single status-mapping: two copies would let the fallback and the
/// nominal path disagree about what a 429 is.
async fn post_send_message(
    client: &Client,
    bot_token: &str,
    chat_id: i64,
    text: String,
    parse_mode: Option<&'static str>,
) -> Result<i64, TelegramApiError> {
    let payload = SendMessagePayload {
        chat_id,
        text,
        parse_mode,
    };

    let resp = client
        .post(api_url(bot_token, "sendMessage"))
        .json(&payload)
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status == 200 {
        let send_resp: TelegramSendResponse =
            resp.json().await.map_err(|e| TelegramApiError::Other {
                status: 200,
                body: format!("failed to parse sendMessage response: {e}"),
            })?;
        let message_id = match send_resp.result {
            Some(r) => r.message_id,
            None => {
                warn!(
                    chat_id,
                    "telegram sendMessage returned 200 but no result — using message_id 0, reply routing will not work"
                );
                0
            }
        };
        return Ok(message_id);
    }

    let body: TelegramResponse = resp.json().await.unwrap_or(TelegramResponse {
        ok: false,
        description: None,
        parameters: None,
    });

    match status {
        401 => Err(TelegramApiError::Unauthorized),
        403 => Err(TelegramApiError::BotBlocked),
        429 => Err(TelegramApiError::RateLimited {
            retry_after: body.parameters.and_then(|p| p.retry_after),
        }),
        400 => Err(TelegramApiError::BadRequest {
            message: body.description.unwrap_or_default(),
        }),
        _ => Err(TelegramApiError::Other {
            status,
            body: body.description.unwrap_or_default(),
        }),
    }
}

/// Resolve a file_id to a file_path via Telegram's `getFile` API.
async fn get_file_impl(
    client: &Client,
    bot_token: &str,
    file_id: &str,
) -> Result<String, TelegramApiError> {
    let url = api_url(bot_token, "getFile");
    let resp = client
        .get(&url)
        .query(&[("file_id", file_id)])
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status != 200 {
        let body: TelegramResponse = resp.json().await.unwrap_or(TelegramResponse {
            ok: false,
            description: None,
            parameters: None,
        });
        return match status {
            401 => Err(TelegramApiError::Unauthorized),
            429 => Err(TelegramApiError::RateLimited {
                retry_after: body.parameters.and_then(|p| p.retry_after),
            }),
            _ => Err(TelegramApiError::Other {
                status,
                body: body.description.unwrap_or_default(),
            }),
        };
    }

    let file_resp: GetFileResponse = resp.json().await.map_err(|e| TelegramApiError::Other {
        status: 200,
        body: format!("failed to parse getFile response: {e}"),
    })?;

    file_resp
        .result
        .and_then(|f| f.file_path)
        .ok_or(TelegramApiError::Other {
            status: 200,
            body: "getFile returned no file_path".to_string(),
        })
}

/// Download file bytes from Telegram's file server.
///
/// Validates `file_path` against traversal/URL-manipulation attacks, then
/// checks the `Content-Length` header before reading the body to reject
/// oversized files early (avoids buffering up to 20 MB only to discard).
async fn download_file_bytes_impl(
    client: &Client,
    bot_token: &str,
    file_path: &str,
) -> Result<Bytes, TelegramApiError> {
    validate_file_path(file_path)?;

    let url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        bot_token, file_path
    );
    let resp = client.get(&url).send().await?;

    let status = resp.status().as_u16();
    if status != 200 {
        return Err(TelegramApiError::Other {
            status,
            body: format!("file download returned {status}"),
        });
    }

    if let Some(content_length) = resp.content_length()
        && content_length as usize > MAX_IMAGE_BYTES
    {
        return Err(TelegramApiError::BadRequest {
            message: format!(
                "file too large ({:.1} MB, max {} MB)",
                content_length as f64 / 1_048_576.0,
                MAX_IMAGE_BYTES / 1_048_576
            ),
        });
    }

    let bytes = resp.bytes().await?;
    Ok(bytes)
}

/// Download an image by file_id: resolves path, downloads bytes, validates magic bytes, enforces size limit.
async fn download_image_impl(
    client: &Client,
    bot_token: &str,
    file_id: &str,
) -> Result<DownloadedImage, TelegramApiError> {
    let file_path = get_file_impl(client, bot_token, file_id).await?;
    let bytes = download_file_bytes_impl(client, bot_token, &file_path).await?;

    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(TelegramApiError::BadRequest {
            message: format!(
                "image too large ({:.1} MB, max {} MB)",
                bytes.len() as f64 / 1_048_576.0,
                MAX_IMAGE_BYTES / 1_048_576
            ),
        });
    }

    let media_type = detect_media_type(&bytes).ok_or(TelegramApiError::BadRequest {
        message: "unsupported image format".to_string(),
    })?;

    Ok(DownloadedImage {
        data: bytes,
        media_type: media_type.to_string(),
    })
}

/// Response from Telegram `getMe` API.
#[derive(Debug, Deserialize)]
struct GetMeResponse {
    ok: bool,
    result: Option<GetMeResult>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GetMeResult {
    username: Option<String>,
}

/// Validate a bot token by calling Telegram's `getMe` endpoint.
/// Returns the bot's username on success.
pub(crate) async fn get_me(client: &Client, bot_token: &str) -> Result<String, TelegramApiError> {
    let resp = client.get(api_url(bot_token, "getMe")).send().await?;

    let status = resp.status().as_u16();
    if status != 200 {
        let body: TelegramResponse = resp.json().await.unwrap_or(TelegramResponse {
            ok: false,
            description: None,
            parameters: None,
        });
        return match status {
            401 => Err(TelegramApiError::Unauthorized),
            _ => Err(TelegramApiError::Other {
                status,
                body: body.description.unwrap_or_default(),
            }),
        };
    }

    let me_resp: GetMeResponse = resp.json().await.map_err(|e| TelegramApiError::Other {
        status: 200,
        body: format!("failed to parse getMe response: {e}"),
    })?;

    if !me_resp.ok {
        return Err(TelegramApiError::Other {
            status: 200,
            body: me_resp
                .description
                .unwrap_or_else(|| "getMe returned ok=false".to_string()),
        });
    }

    me_resp
        .result
        .and_then(|r| r.username)
        .ok_or(TelegramApiError::Other {
            status: 200,
            body: "getMe returned no username".to_string(),
        })
}

/// Register the webhook URL with Telegram. Verifies `ok: true` response.
async fn set_webhook_impl(
    client: &Client,
    bot_token: &str,
    webhook_url: &str,
    webhook_secret: &str,
) -> anyhow::Result<()> {
    let payload = SetWebhookPayload {
        url: webhook_url.to_string(),
        secret_token: webhook_secret.to_string(),
        allowed_updates: vec!["message".to_string()],
        max_connections: 30,
    };

    let resp = client
        .post(api_url(bot_token, "setWebhook"))
        .json(&payload)
        .send()
        .await
        // Do not interpolate the reqwest error: its Display includes the request URL,
        // which embeds `bot<TOKEN>` and would leak the bot token into logs (mika#1612).
        .map_err(|_| anyhow::anyhow!("setWebhook network request failed"))?;

    let body: TelegramResponse = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("setWebhook response parse failed: {e}"))?;

    if !body.ok {
        anyhow::bail!(
            "setWebhook failed: {}",
            body.description
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }

    Ok(())
}

// -- TelegramClient (global single-bot mode) --

/// Telegram API client wrapper for the global single-bot mode.
///
/// Bot token stored as SecretString — never logged or displayed.
#[derive(Clone)]
pub struct TelegramClient {
    client: Client,
    bot_token: SecretString,
}

impl TelegramClient {
    pub fn new(client: Client, bot_token: SecretString) -> Self {
        Self { client, bot_token }
    }

    /// Clone the bot token (for constructing a `CustomerTelegramClient` from the global token).
    pub fn bot_token_cloned(&self) -> SecretString {
        SecretString::from(self.bot_token.expose_secret().to_string())
    }

    /// Register the webhook URL with Telegram. Verifies `ok: true` response.
    pub async fn set_webhook(&self, webhook_url: &str, webhook_secret: &str) -> anyhow::Result<()> {
        set_webhook_impl(
            &self.client,
            self.bot_token.expose_secret(),
            webhook_url,
            webhook_secret,
        )
        .await
    }
}

impl std::fmt::Debug for TelegramClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramClient")
            .field("bot_token", &"[REDACTED]")
            .finish()
    }
}

// -- CustomerTelegramClient (per-customer bot token) --

/// Lightweight Telegram API client for per-customer bot tokens.
///
/// Shares the `reqwest::Client` connection pool from `AppState` but carries
/// a customer-specific bot token. Constructed per-request from the customer's
/// `bot_token` column in the `customers` table.
#[derive(Clone)]
pub struct CustomerTelegramClient {
    client: Client,
    bot_token: SecretString,
}

impl CustomerTelegramClient {
    pub fn new(client: Client, bot_token: SecretString) -> Self {
        Self { client, bot_token }
    }

    /// Send a text message to a chat. Returns the Telegram message_id on success.
    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<i64, TelegramApiError> {
        send_message_impl(&self.client, self.bot_token.expose_secret(), chat_id, text).await
    }

    /// Download an image by file_id: resolves path, downloads bytes, validates magic bytes, enforces size limit.
    pub async fn download_image(&self, file_id: &str) -> Result<DownloadedImage, TelegramApiError> {
        download_image_impl(&self.client, self.bot_token.expose_secret(), file_id).await
    }

    /// Register the webhook URL with Telegram for this customer's bot.
    pub async fn set_webhook(&self, webhook_url: &str, webhook_secret: &str) -> anyhow::Result<()> {
        set_webhook_impl(
            &self.client,
            self.bot_token.expose_secret(),
            webhook_url,
            webhook_secret,
        )
        .await
    }

    /// Validate the bot token by calling Telegram's `getMe` endpoint.
    /// Returns the bot's username on success.
    pub async fn get_me(&self) -> Result<String, TelegramApiError> {
        get_me(&self.client, self.bot_token.expose_secret()).await
    }
}

impl std::fmt::Debug for CustomerTelegramClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomerTelegramClient")
            .field("bot_token", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a text-only TelegramMessage.
    fn text_msg(chat_id: i64, text: Option<&str>) -> TelegramMessage {
        TelegramMessage {
            chat: TelegramChat { id: chat_id },
            text: text.map(|s| s.to_string()),
            photo: None,
            caption: None,
            document: None,
            reply_to_message: None,
        }
    }

    /// Helper to build a photo TelegramMessage.
    fn photo_msg(chat_id: i64, photos: Vec<PhotoSize>, caption: Option<&str>) -> TelegramMessage {
        TelegramMessage {
            chat: TelegramChat { id: chat_id },
            text: None,
            photo: Some(photos),
            caption: caption.map(|s| s.to_string()),
            document: None,
            reply_to_message: None,
        }
    }

    /// Helper to build a document TelegramMessage.
    fn document_msg(
        chat_id: i64,
        file_id: &str,
        mime_type: Option<&str>,
        caption: Option<&str>,
    ) -> TelegramMessage {
        TelegramMessage {
            chat: TelegramChat { id: chat_id },
            text: None,
            photo: None,
            caption: caption.map(|s| s.to_string()),
            document: Some(TelegramDocument {
                file_id: file_id.to_string(),
                file_unique_id: "unique_1".to_string(),
                file_name: None,
                mime_type: mime_type.map(|s| s.to_string()),
                file_size: None,
            }),
            reply_to_message: None,
        }
    }

    fn make_photo_size(file_id: &str, width: u32, height: u32) -> PhotoSize {
        PhotoSize {
            file_id: file_id.to_string(),
            file_unique_id: format!("uniq_{file_id}"),
            width,
            height,
            file_size: None,
        }
    }

    #[test]
    fn test_parse_text_message() {
        let update = TelegramUpdate {
            update_id: 100,
            message: Some(text_msg(42, Some("Hello Mika!"))),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Text {
                chat_id: 42,
                text: "Hello Mika!".to_string(),
                update_id: 100,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_parse_start_command() {
        let token = "a1b2c3d4e5f6";
        let update = TelegramUpdate {
            update_id: 101,
            message: Some(text_msg(42, Some(&format!("/start {token}")))),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Start {
                chat_id: 42,
                pairing_token: token.to_string(),
            }
        );
    }

    #[test]
    fn test_parse_start_with_whitespace() {
        let update = TelegramUpdate {
            update_id: 102,
            message: Some(text_msg(42, Some("/start  abc123  "))),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Start {
                chat_id: 42,
                pairing_token: "abc123".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_start_empty_payload() {
        let update = TelegramUpdate {
            update_id: 103,
            message: Some(text_msg(42, Some("/start "))),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Unsupported { chat_id: 42 }
        );
    }

    #[test]
    fn test_parse_non_text_message() {
        let update = TelegramUpdate {
            update_id: 104,
            message: Some(text_msg(42, None)),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Unsupported { chat_id: 42 }
        );
    }

    #[test]
    fn test_parse_no_message() {
        let update = TelegramUpdate {
            update_id: 105,
            message: None,
        };
        assert_eq!(parse_update(&update), ParsedMessage::NoMessage);
    }

    #[test]
    fn test_parse_bare_start() {
        let update = TelegramUpdate {
            update_id: 106,
            message: Some(text_msg(42, Some("/start"))),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::BareStart { chat_id: 42 }
        );
    }

    // /unlink command family (mika#1749)

    /// Exact `/unlink` produces `Unlink` — the warning-then-confirm entry point.
    #[test]
    fn test_parse_unlink_bare() {
        let update = TelegramUpdate {
            update_id: 200,
            message: Some(text_msg(42, Some("/unlink"))),
        };
        assert_eq!(parse_update(&update), ParsedMessage::Unlink { chat_id: 42 });
    }

    /// `/unlink confirm` produces `UnlinkConfirm` — the atomic release.
    #[test]
    fn test_parse_unlink_confirm() {
        let update = TelegramUpdate {
            update_id: 201,
            message: Some(text_msg(42, Some("/unlink confirm"))),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::UnlinkConfirm { chat_id: 42 }
        );
    }

    /// Whitespace canonicalization: `/unlink   confirm` (extra spaces) still
    /// resolves to `UnlinkConfirm`.
    #[test]
    fn test_parse_unlink_confirm_extra_whitespace() {
        let update = TelegramUpdate {
            update_id: 202,
            message: Some(text_msg(42, Some("/unlink   confirm"))),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::UnlinkConfirm { chat_id: 42 }
        );
    }

    /// Unknown suffix (typo, e.g. `/unlink now`) falls back to `Unlink` — the
    /// handler shows the warning path. Better than silently no-oping.
    #[test]
    fn test_parse_unlink_unknown_suffix_falls_to_warning() {
        let update = TelegramUpdate {
            update_id: 203,
            message: Some(text_msg(42, Some("/unlink now"))),
        };
        assert_eq!(parse_update(&update), ParsedMessage::Unlink { chat_id: 42 });
    }

    /// `/unlinkxxx` (no space between command and suffix) is NOT our command —
    /// falls through to `Text` and is forwarded to the agent as free text.
    /// Guards against accidental release from partial typos.
    #[test]
    fn test_parse_unlink_no_space_is_text() {
        let update = TelegramUpdate {
            update_id: 204,
            message: Some(text_msg(42, Some("/unlinkxxx"))),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Text {
                chat_id: 42,
                text: "/unlinkxxx".to_string(),
                update_id: 204,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_telegram_client_debug_redacts_token() {
        let client = TelegramClient::new(Client::new(), SecretString::from("123456:ABC-DEF"));
        let debug = format!("{client:?}");
        assert!(!debug.contains("ABC-DEF"));
        assert!(debug.contains("[REDACTED]"));
    }

    // -- Photo parsing tests --

    #[test]
    fn test_parse_photo_message_picks_largest() {
        let photos = vec![
            make_photo_size("small", 90, 90),
            make_photo_size("medium", 320, 320),
            make_photo_size("large", 800, 800),
        ];
        let update = TelegramUpdate {
            update_id: 200,
            message: Some(photo_msg(42, photos, None)),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Photo {
                chat_id: 42,
                file_id: "large".to_string(),
                caption: None,
                update_id: 200,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_parse_photo_message_with_caption() {
        let photos = vec![make_photo_size("pic1", 640, 480)];
        let update = TelegramUpdate {
            update_id: 201,
            message: Some(photo_msg(42, photos, Some("Look at this!"))),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Photo {
                chat_id: 42,
                file_id: "pic1".to_string(),
                caption: Some("Look at this!".to_string()),
                update_id: 201,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_parse_photo_message_single_size() {
        let photos = vec![make_photo_size("only", 1024, 768)];
        let update = TelegramUpdate {
            update_id: 202,
            message: Some(photo_msg(42, photos, None)),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Photo {
                chat_id: 42,
                file_id: "only".to_string(),
                caption: None,
                update_id: 202,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    // -- Document parsing tests --

    #[test]
    fn test_parse_image_document() {
        let update = TelegramUpdate {
            update_id: 300,
            message: Some(document_msg(42, "doc_file_1", Some("image/jpeg"), None)),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Document {
                chat_id: 42,
                file_id: "doc_file_1".to_string(),
                mime_type: "image/jpeg".to_string(),
                caption: None,
                update_id: 300,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_parse_image_document_with_caption() {
        let update = TelegramUpdate {
            update_id: 301,
            message: Some(document_msg(
                42,
                "doc_file_2",
                Some("image/png"),
                Some("A diagram"),
            )),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Document {
                chat_id: 42,
                file_id: "doc_file_2".to_string(),
                mime_type: "image/png".to_string(),
                caption: Some("A diagram".to_string()),
                update_id: 301,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_parse_non_image_document_is_unsupported() {
        let update = TelegramUpdate {
            update_id: 302,
            message: Some(document_msg(42, "pdf_file", Some("application/pdf"), None)),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Unsupported { chat_id: 42 }
        );
    }

    #[test]
    fn test_parse_document_no_mime_is_unsupported() {
        let update = TelegramUpdate {
            update_id: 303,
            message: Some(document_msg(42, "unknown_file", None, None)),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Unsupported { chat_id: 42 }
        );
    }

    #[test]
    fn test_parse_webp_document() {
        let update = TelegramUpdate {
            update_id: 304,
            message: Some(document_msg(42, "webp_file", Some("image/webp"), None)),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Document {
                chat_id: 42,
                file_id: "webp_file".to_string(),
                mime_type: "image/webp".to_string(),
                caption: None,
                update_id: 304,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_parse_gif_document() {
        let update = TelegramUpdate {
            update_id: 305,
            message: Some(document_msg(42, "gif_file", Some("image/gif"), None)),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Document {
                chat_id: 42,
                file_id: "gif_file".to_string(),
                mime_type: "image/gif".to_string(),
                caption: None,
                update_id: 305,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    // -- Magic byte detection tests --

    #[test]
    fn test_detect_jpeg() {
        let bytes = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(detect_media_type(&bytes), Some("image/jpeg"));
    }

    #[test]
    fn test_detect_png() {
        let bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
        assert_eq!(detect_media_type(&bytes), Some("image/png"));
    }

    #[test]
    fn test_detect_gif() {
        let bytes = [0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0, 0, 0, 0, 0, 0];
        assert_eq!(detect_media_type(&bytes), Some("image/gif"));
    }

    #[test]
    fn test_detect_webp() {
        let bytes = [0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50];
        assert_eq!(detect_media_type(&bytes), Some("image/webp"));
    }

    #[test]
    fn test_detect_unknown_format() {
        let bytes = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
        ];
        assert_eq!(detect_media_type(&bytes), None);
    }

    #[test]
    fn test_detect_too_short() {
        let bytes = [0xFF, 0xD8];
        assert_eq!(detect_media_type(&bytes), None);
    }

    // -- JSON deserialization tests --

    #[test]
    fn test_deserialize_photo_update() {
        let json = r#"{
            "update_id": 500,
            "message": {
                "chat": {"id": 42},
                "photo": [
                    {"file_id": "sm", "file_unique_id": "u1", "width": 90, "height": 90},
                    {"file_id": "lg", "file_unique_id": "u2", "width": 800, "height": 600}
                ],
                "caption": "Check this out"
            }
        }"#;
        let update: TelegramUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Photo {
                chat_id: 42,
                file_id: "lg".to_string(),
                caption: Some("Check this out".to_string()),
                update_id: 500,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_deserialize_document_update() {
        let json = r#"{
            "update_id": 501,
            "message": {
                "chat": {"id": 42},
                "document": {
                    "file_id": "doc1",
                    "file_unique_id": "u3",
                    "file_name": "photo.jpg",
                    "mime_type": "image/jpeg",
                    "file_size": 123456
                }
            }
        }"#;
        let update: TelegramUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Document {
                chat_id: 42,
                file_id: "doc1".to_string(),
                mime_type: "image/jpeg".to_string(),
                caption: None,
                update_id: 501,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_deserialize_text_update_ignores_new_fields() {
        // Verify backward compat: a text-only update still parses correctly
        let json = r#"{
            "update_id": 502,
            "message": {
                "chat": {"id": 42},
                "text": "Hello"
            }
        }"#;
        let update: TelegramUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Text {
                chat_id: 42,
                text: "Hello".to_string(),
                update_id: 502,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    // -- Reply-to-message tests --

    #[test]
    fn test_parse_reply_message_extracts_id() {
        let mut msg = text_msg(42, Some("replying"));
        msg.reply_to_message = Some(ReplyToMessage {
            message_id: 999,
            text: None,
        });
        let update = TelegramUpdate {
            update_id: 600,
            message: Some(msg),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Text {
                chat_id: 42,
                text: "replying".to_string(),
                update_id: 600,
                reply_to_message_id: Some(999),
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_parse_no_reply_returns_none() {
        let update = TelegramUpdate {
            update_id: 601,
            message: Some(text_msg(42, Some("no reply"))),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Text {
                chat_id: 42,
                text: "no reply".to_string(),
                update_id: 601,
                reply_to_message_id: None,
                reply_to_text: None,
            }
        );
    }

    #[test]
    fn test_deserialize_reply_to_message() {
        let json = r#"{
            "update_id": 602,
            "message": {
                "chat": {"id": 42},
                "text": "reply text",
                "reply_to_message": {"message_id": 555, "chat": {"id": 42}, "text": "original"}
            }
        }"#;
        let update: TelegramUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Text {
                chat_id: 42,
                text: "reply text".to_string(),
                update_id: 602,
                reply_to_message_id: Some(555),
                reply_to_text: Some("original".to_string()),
            }
        );
    }

    // -- parse_agent_prefix tests --

    #[test]
    fn test_parse_agent_prefix_valid() {
        assert_eq!(
            parse_agent_prefix("[mika-test] hello"),
            Some("mika-test".to_string())
        );
    }

    #[test]
    fn test_parse_agent_prefix_default_agent() {
        assert_eq!(
            parse_agent_prefix("[mika] Hello, Vincent!"),
            Some("mika".to_string())
        );
    }

    #[test]
    fn test_parse_agent_prefix_no_prefix() {
        assert_eq!(parse_agent_prefix("Hello world"), None);
    }

    #[test]
    fn test_parse_agent_prefix_empty_name() {
        assert_eq!(parse_agent_prefix("[] hello"), None);
    }

    #[test]
    fn test_parse_agent_prefix_invalid_uppercase() {
        assert_eq!(parse_agent_prefix("[MIKA] hello"), None);
    }

    #[test]
    fn test_parse_agent_prefix_too_long() {
        let long_name = "a".repeat(33);
        assert_eq!(parse_agent_prefix(&format!("[{long_name}] hello")), None);
    }

    #[test]
    fn test_parse_agent_prefix_no_space_after_bracket() {
        assert_eq!(parse_agent_prefix("[mika]hello"), None);
    }

    #[test]
    fn test_parse_agent_prefix_with_digits() {
        assert_eq!(
            parse_agent_prefix("[agent-2] task result"),
            Some("agent-2".to_string())
        );
    }

    #[test]
    fn test_deserialize_reply_with_text() {
        let json = r#"{"message_id": 100, "text": "[mika-test] hello"}"#;
        let reply: ReplyToMessage = serde_json::from_str(json).unwrap();
        assert_eq!(reply.message_id, 100);
        assert_eq!(reply.text, Some("[mika-test] hello".to_string()));
    }

    #[test]
    fn test_deserialize_reply_without_text() {
        let json = r#"{"message_id": 200}"#;
        let reply: ReplyToMessage = serde_json::from_str(json).unwrap();
        assert_eq!(reply.message_id, 200);
        assert_eq!(reply.text, None);
    }

    // -- file_path validation tests --

    #[test]
    fn test_validate_file_path_accepts_normal_path() {
        assert!(validate_file_path("photos/file_1.jpg").is_ok());
    }

    #[test]
    fn test_validate_file_path_accepts_nested_path() {
        assert!(validate_file_path("documents/user/photo.png").is_ok());
    }

    #[test]
    fn test_validate_file_path_rejects_empty() {
        let err = validate_file_path("").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_validate_file_path_rejects_leading_slash() {
        let err = validate_file_path("/etc/passwd").unwrap_err();
        assert!(err.to_string().contains("starts with /"));
    }

    #[test]
    fn test_validate_file_path_rejects_traversal() {
        let err = validate_file_path("photos/../../etc/passwd").unwrap_err();
        assert!(err.to_string().contains("contains .."));
    }

    #[test]
    fn test_validate_file_path_rejects_bare_traversal() {
        let err = validate_file_path("..").unwrap_err();
        assert!(err.to_string().contains("contains .."));
    }

    #[test]
    fn test_validate_file_path_rejects_at_sign() {
        let err = validate_file_path("photos/@evil.com/file").unwrap_err();
        assert!(err.to_string().contains("contains '@'"));
    }

    #[test]
    fn test_validate_file_path_rejects_question_mark() {
        let err = validate_file_path("photos/file?token=leak").unwrap_err();
        assert!(err.to_string().contains("contains '?'"));
    }

    #[test]
    fn test_validate_file_path_rejects_hash() {
        let err = validate_file_path("photos/file#fragment").unwrap_err();
        assert!(err.to_string().contains("contains '#'"));
    }

    // -- api_url tests --

    #[test]
    fn test_api_url_constructs_correct_url() {
        let url = api_url("123456:ABC-DEF", "sendMessage");
        assert_eq!(
            url,
            "https://api.telegram.org/bot123456:ABC-DEF/sendMessage"
        );
    }

    #[test]
    fn test_api_url_get_file() {
        let url = api_url("tok", "getFile");
        assert_eq!(url, "https://api.telegram.org/bottok/getFile");
    }

    // -- CustomerTelegramClient tests --

    #[test]
    fn test_customer_telegram_client_debug_redacts_token() {
        let client =
            CustomerTelegramClient::new(Client::new(), SecretString::from("123456:ABC-DEF"));
        let debug = format!("{client:?}");
        assert!(!debug.contains("ABC-DEF"));
        assert!(debug.contains("[REDACTED]"));
    }

    // -- strip_markdown_around_urls tests (mika#2126) --
    //
    // AC2 is a test of EFFECT, not of form: every positive case asserts on the URL
    // *as Telegram's autolinker would hand it to the user* — the whitespace-delimited
    // token carrying the scheme — and requires it to parse as a well-formed URL.
    // Asserting on the whole message would pass even if the URL stayed broken.

    /// The URL a Telegram user would actually click: the first whitespace-delimited
    /// token carrying an http(s) scheme. Telegram's autolinker stops at whitespace,
    /// which is exactly why glued decoration ends up inside the link.
    fn clicked_url(text: &str) -> String {
        text.split_whitespace()
            .find(|t| t.contains("http://") || t.contains("https://"))
            .unwrap_or_default()
            .to_string()
    }

    /// Assert the clicked URL of the cleaned message is exactly `expected` AND is a
    /// well-formed absolute URL (R2/AC2).
    fn assert_clicked_url(input: &str, expected: &str) {
        let cleaned = strip_markdown_around_urls(input);
        let clicked = clicked_url(&cleaned);
        assert_eq!(
            clicked, expected,
            "clicked URL mismatch for input {input:?} (cleaned: {cleaned:?})"
        );
        url::Url::parse(&clicked)
            .unwrap_or_else(|e| panic!("clicked URL {clicked:?} is not well-formed: {e}"));
    }

    /// AC4 — frozen fixture of the reported case. Vincent via Al, 2026-09-01 12:03:
    /// the agent sent `**https://github.com/senara-solutions/mika**`, Telegram absorbed
    /// the trailing asterisks into the link, and `/mika**` returned 404 (`/mika` → 200).
    /// If this test stops failing when the cleaning is removed, it is testing nothing.
    #[test]
    fn test_strip_markdown_mika_2126_reported_case_bold_repo_url() {
        assert_clicked_url(
            "**https://github.com/senara-solutions/mika**",
            "https://github.com/senara-solutions/mika",
        );
    }

    // -- Positives: decoration glued to the URL is removed --

    #[test]
    fn test_strip_markdown_bold_url() {
        assert_clicked_url("**https://example.com/a**", "https://example.com/a");
    }

    #[test]
    fn test_strip_markdown_italic_url() {
        assert_clicked_url("_https://example.com/a_", "https://example.com/a");
    }

    #[test]
    fn test_strip_markdown_backticked_url() {
        assert_clicked_url("`https://example.com/a`", "https://example.com/a");
    }

    #[test]
    fn test_strip_markdown_strikethrough_url() {
        assert_clicked_url("~~https://example.com/a~~", "https://example.com/a");
    }

    #[test]
    fn test_strip_markdown_bold_url_inside_sentence() {
        assert_clicked_url(
            "Le dépôt est **https://example.com/a** si tu veux voir.",
            "https://example.com/a",
        );
    }

    #[test]
    fn test_strip_markdown_link_becomes_label_then_bare_url() {
        let cleaned = strip_markdown_around_urls("[le dépôt](https://example.com/a)");
        assert_eq!(cleaned, "le dépôt : https://example.com/a");
        assert_clicked_url("[le dépôt](https://example.com/a)", "https://example.com/a");
    }

    #[test]
    fn test_strip_markdown_link_with_label_equal_to_url_keeps_url_only() {
        let cleaned = strip_markdown_around_urls("[https://example.com/a](https://example.com/a)");
        assert_eq!(cleaned, "https://example.com/a");
    }

    #[test]
    fn test_strip_markdown_link_with_empty_label_keeps_url_only() {
        let cleaned = strip_markdown_around_urls("[](https://example.com/a)");
        assert_eq!(cleaned, "https://example.com/a");
    }

    #[test]
    fn test_strip_markdown_bold_markdown_link_is_fully_unwrapped() {
        assert_clicked_url(
            "**[le dépôt](https://example.com/a)**",
            "https://example.com/a",
        );
    }

    #[test]
    fn test_strip_markdown_http_scheme_is_covered_too() {
        assert_clicked_url("**http://example.com/a**", "http://example.com/a");
    }

    #[test]
    fn test_strip_markdown_two_decorated_urls_in_one_message() {
        let cleaned =
            strip_markdown_around_urls("Voir **https://example.com/a** et `https://example.com/b`");
        assert_eq!(
            cleaned,
            "Voir https://example.com/a et https://example.com/b"
        );
    }

    // -- Negatives (AC3): a healthy message passes byte-for-byte unchanged --
    //
    // A fix that rewrites correct URLs has repaired nothing: it has added a second
    // way to break them. Each of these asserts `out == input`, not merely "looks ok".

    #[test]
    fn test_strip_markdown_bare_url_unchanged() {
        let input = "Va voir https://example.com/a";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    #[test]
    fn test_strip_markdown_url_ending_in_underscore_unchanged() {
        // `_` is a legal unreserved URL character. An unpaired trailing one is part
        // of the URL, not decoration (KTD3).
        let input = "https://example.com/path_with_underscore_";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    #[test]
    fn test_strip_markdown_url_containing_tilde_unchanged() {
        let input = "https://example.com/~vincent";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    #[test]
    fn test_strip_markdown_url_followed_by_sentence_period_unchanged() {
        // A period is sentence punctuation, not a markdown marker.
        let input = "Voir https://example.com/a.";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    #[test]
    fn test_strip_markdown_hostname_without_scheme_unchanged() {
        // Real case: OFFLINE_ERROR_MSG (routes.rs) carries `console.getmika.ai` with
        // no scheme, so it sits outside the rule's anchor entirely.
        let input = "Réessaie plus tard, ou passe par console.getmika.ai.";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    #[test]
    fn test_strip_markdown_message_without_url_unchanged() {
        let input = "Bonjour Sonia 🌸";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    /// **R2 (mika#2291) — what this test pins has changed scope, and saying so is
    /// the point.** It still asserts that *this function* leaves `**important**`
    /// alone, which remains correct: its perimeter is URLs. It no longer describes
    /// what the **pipeline** does — since mika#2291 the recognizer upstream renders
    /// that bold, and `mika2291_r3_bold_text_now_changes_at_pipeline_level` asserts
    /// exactly that. R2 and R3 side by side are the readable trace of the perimeter
    /// moving. A frozen test left green without a word about its narrowed scope is a
    /// test that lies later.
    #[test]
    fn test_strip_markdown_bold_text_without_url_unchanged() {
        // Out of scope on purpose: the perimeter is URLs, not markdown rendering (AC3).
        let input = "C'est **important** de le savoir.";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    #[test]
    fn test_strip_markdown_preserves_exact_whitespace_and_newlines() {
        let input = "Ligne un\n\n  Va voir  https://example.com/a\tfin";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    #[test]
    fn test_strip_markdown_empty_string_unchanged() {
        assert_eq!(strip_markdown_around_urls(""), "");
    }

    #[test]
    fn test_strip_markdown_non_url_markdown_link_unchanged() {
        // Not anchored on a scheme → left intact (conservatism = AC3 safety).
        let input = "[le dépôt](/relatif/a)";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    #[test]
    fn test_strip_markdown_link_with_space_in_url_unchanged() {
        let input = "[x](https://example.com/a b)";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    #[test]
    fn test_strip_markdown_multibyte_text_around_url_unchanged() {
        // KTD6: no raw byte indexing — this must not panic on a UTF-8 boundary.
        let input = "Éh 🌸 https://example.com/été — ça va ?";
        assert_eq!(strip_markdown_around_urls(input), input);
    }

    #[test]
    fn test_strip_markdown_multibyte_text_around_decorated_url() {
        assert_clicked_url(
            "Éh 🌸 **https://example.com/a** — ça va ?",
            "https://example.com/a",
        );
    }

    // -- mika#2291: the rendering pipeline, on top of the mika#2126 auxiliary --

    use crate::telegram_markdown::{AC3_CORPUS, render_html, render_plain, tokenize};

    /// `plain_body` carries a `chat_id` for its metrics line only; the value is
    /// irrelevant to every assertion below, so one sentinel serves them all.
    const TEST_CHAT_ID: i64 = 42;

    /// R1 — the mika#2126 test population, recounted **by the build** rather than
    /// remembered.
    ///
    /// A number you cannot recount is an AC you cannot verify, and this one had
    /// already drifted once in the plan's own history (30, corrected to 25). Making
    /// it a test rather than a comment naming a shell command is what keeps it
    /// honest: the first draft here *was* that comment, and because it quoted the
    /// pattern it counted itself and reported 26.
    ///
    /// Two scoping rules, both learned the same way: comment lines are excluded (a
    /// doc-comment must be free to name what it counts), and the needle is assembled
    /// from two pieces so that **this line does not match itself**.
    #[test]
    fn mika2291_r1_mika2126_test_population_is_unchanged() {
        let needle = concat!("fn test_strip_", "markdown");
        let count = include_str!("telegram.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .filter(|l| l.contains(needle))
            .count();
        assert_eq!(
            count, 25,
            "the mika#2126 test population changed; mika#2291 must leave all 25 of \
             them intact (add mika#2291 coverage under its own names instead)"
        );
    }

    /// What a Telegram client **displays** for one of our HTML bodies: the entities
    /// are consumed as formatting, the three escapes are restored.
    ///
    /// A model of Telegram's entity parsing, not Telegram itself — stated plainly
    /// because the distinction is what the test is worth. It is sound for the only
    /// shapes [`render_html`] emits (the six tags it writes, `<a href="…">`
    /// included), which is exactly the surface under test; it would be wrong for
    /// arbitrary HTML, and nothing here feeds it any.
    fn html_rendered_text(html: &str) -> String {
        let mut out = String::new();
        let mut chars = html.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '<' {
                // Drop the tag. `>` cannot appear inside one: render_html escapes it
                // in every text and in every href.
                for inner in chars.by_ref() {
                    if inner == '>' {
                        break;
                    }
                }
                continue;
            }
            out.push(c);
        }
        unescape_html_entities(&out)
    }

    fn unescape_html_entities(text: &str) -> String {
        text.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
    }

    /// The URL a Telegram user would actually reach from one of our HTML bodies.
    ///
    /// **Two sources, and conflating them is what a first draft of this helper got
    /// wrong.** In HTML mode a link can arrive two ways, and only one of them puts
    /// the URL in the text the user sees:
    ///
    /// 1. an explicit `<a href="…">label</a>` entity — the target is the `href`, and
    ///    the URL may appear nowhere in the displayed text (`[le dépôt](url)`
    ///    displays `le dépôt`);
    /// 2. a bare URL in the text, which Telegram's autolinker picks up exactly as it
    ///    does today on the plain path — and that is the path where glued decoration
    ///    used to break the link (mika#2126).
    ///
    /// The `href` wins when present, because that is the anchor Telegram honours.
    fn clicked_url_in_html(html: &str) -> String {
        const HREF: &str = "<a href=\"";
        if let Some(start) = html.find(HREF) {
            let after = &html[start + HREF.len()..]; // safe-byte-slice: `start` is a char boundary from str::find and HREF is ASCII, so the sum lands on a boundary
            if let Some(end) = after.find('"') {
                return unescape_html_entities(&after[..end]); // safe-byte-slice: `end` comes from str::find on `after`
            }
        }
        clicked_url(&html_rendered_text(html))
    }

    /// N12 — AC3 through the composition that is **actually emitted**.
    ///
    /// The module's own negatives assert `render_plain(tokenize(x)) == x`, but the
    /// plain path emits `strip_markdown_around_urls(render_plain(tokenize(x)))`. As
    /// long as the control stopped in the middle, AC3 was true of a value the user
    /// never receives. This replays the same corpus through [`plain_body`] and
    /// asserts the same byte-for-byte equality, so the one composition capable of
    /// rewriting a healthy message — two transformers that each preserve, placed end
    /// to end — stops being a blind spot.
    ///
    /// Its failure would be a precise signal: not "one of the two is wrong" but
    /// "they overlap".
    #[test]
    fn mika2291_n12_ac3_holds_through_the_emitted_composition() {
        for (id, input) in AC3_CORPUS {
            assert_eq!(
                plain_body(TEST_CHAT_ID, input),
                *input,
                "{id}: AC3 violated through the emitted composition, on {input:?}"
            );
        }
    }

    /// S1 — the floor's identity: disarmed mode and the fallback path produce the
    /// same byte.
    ///
    /// It holds **by construction** — both branches of `send_message_impl` call
    /// [`plain_body`], the single construction site — so the naive
    /// `assert_eq!(a, a)` would be vacuous. What is asserted instead is what could
    /// actually drift: that `plain_body` *is* the composition
    /// `strip_markdown_around_urls ∘ render_plain ∘ tokenize`, over a corpus that
    /// mixes healthy and decorated inputs. If someone inlines a different
    /// composition into one branch, this goes red.
    #[test]
    fn mika2291_s1_the_floor_is_one_single_composition() {
        let corpus: Vec<&str> = AC3_CORPUS
            .iter()
            .map(|(_, input)| *input)
            .chain([
                "C'est **important** de le savoir.",
                "un *mot* ital",
                "code `foo()` ici",
                "~~annulé~~",
                "[le dépôt](https://example.com/a)",
                "# Titre\ncorps",
                "* premier\n* second",
                "```rs\nlet x = 1;\n```",
                "**https://example.com/a**",
                "_https://example.com/a_",
                "**[le dépôt](https://example.com/a)**",
                "[mika-dev] C'est **important** de le savoir.",
                "a < b & c > d",
            ])
            .collect();
        assert!(corpus.len() >= 20, "corpus too small: {}", corpus.len());
        for input in corpus {
            let expected = strip_markdown_around_urls(&render_plain(&tokenize(input)));
            assert_eq!(
                plain_body(TEST_CHAT_ID, input),
                expected,
                "the floor is no longer one composition, on {input:?}"
            );
        }
    }

    /// S1's structural half: the plain body is built at **exactly one** site.
    ///
    /// The identity above is worth nothing if a second branch grows its own
    /// composition. A behavioural test cannot see that — both sites could agree
    /// today and drift tomorrow — so the guard reads the source, the shape the repo
    /// has had to impose before (mika#2131).
    #[test]
    fn mika2291_s1_plain_body_is_the_sole_floor_construction_site() {
        // The production half is read by `mika_common::source_guard`
        // (mika#2398). Cutting at the first `\n#[cfg(test)]` was blind to a
        // module-level `#[cfg(test)]` helper and to a single-line item, so a
        // second composition site placed below either one would have been
        // invisible to this guard while every assertion stayed green.
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let production = scanner.production_of(&scanner.src_root().join("telegram.rs"));
        let calls = production.matches("strip_markdown_around_urls(").count();
        // One definition + one call, inside `plain_body`.
        assert_eq!(
            calls, 2,
            "expected `strip_markdown_around_urls` to be defined once and called once \
             (inside plain_body); found {calls} occurrences in the production half"
        );
        assert_eq!(
            production.matches("render_plain(").count(),
            1,
            "the plain rendering must be composed at exactly one site (plain_body)"
        );
    }

    /// R3 — the sibling of R2, at pipeline level: `**important**` **changes** now.
    #[test]
    fn mika2291_r3_bold_text_now_changes_at_pipeline_level() {
        let input = "C'est **important** de le savoir.";
        assert_ne!(
            plain_body(TEST_CHAT_ID, input),
            input,
            "the pipeline must no longer leave raw bold markers"
        );
        assert_eq!(
            plain_body(TEST_CHAT_ID, input),
            "C'est important de le savoir."
        );
    }

    /// R4 — the founding defect of mika#2126 is closed **in both modes**.
    ///
    /// In HTML mode the bold becomes `<b>`, so the link's boundary is the tag and no
    /// `*` is glued to the URL. In plain mode the markers are gone before the
    /// auxiliary even runs.
    #[test]
    fn mika2291_r4_founding_defect_closed_in_both_modes() {
        let input = "**https://github.com/senara-solutions/mika**";
        let expected = "https://github.com/senara-solutions/mika";
        assert_eq!(
            clicked_url(&plain_body(TEST_CHAT_ID, input)),
            expected,
            "plain mode"
        );
        assert_eq!(
            clicked_url_in_html(&render_html(&tokenize(input))),
            expected,
            "html mode"
        );
    }

    /// R5 — **the armed mode holds mika#2126 without the net.**
    ///
    /// This is the control R1 structurally cannot give. R1 verifies the URL
    /// auxiliary is intact; the **default** path never calls it (`render_html` is
    /// posted directly). Without R5, mika#2126's non-regression would be measured
    /// only on the fallback path — the one that, in nominal operation, never runs.
    ///
    /// mika#2126's URL corpus, replayed against `render_html`: the founding case,
    /// paired borders, unpaired decoration, the trailing `_` that is legal in a URL,
    /// and the link with a space in its URL that the grammar refuses.
    #[test]
    fn mika2291_r5_armed_mode_holds_mika2126_without_the_net() {
        let cases: &[(&str, &str, &str)] = &[
            (
                "founding case",
                "**https://github.com/senara-solutions/mika**",
                "https://github.com/senara-solutions/mika",
            ),
            ("bold", "**https://example.com/a**", "https://example.com/a"),
            ("italic", "_https://example.com/a_", "https://example.com/a"),
            ("code", "`https://example.com/a`", "https://example.com/a"),
            (
                "strike",
                "~~https://example.com/a~~",
                "https://example.com/a",
            ),
            (
                "in a sentence",
                "Le dépôt est **https://example.com/a** si tu veux voir.",
                "https://example.com/a",
            ),
            (
                "markdown link",
                "[le dépôt](https://example.com/a)",
                "https://example.com/a",
            ),
            (
                "bold markdown link",
                "**[le dépôt](https://example.com/a)**",
                "https://example.com/a",
            ),
            (
                "http scheme",
                "**http://example.com/a**",
                "http://example.com/a",
            ),
            (
                "bare url untouched",
                "Va voir https://example.com/a",
                "https://example.com/a",
            ),
            (
                "unpaired trailing underscore is part of the URL",
                "https://example.com/path_with_underscore_",
                "https://example.com/path_with_underscore_",
            ),
            (
                "tilde is a legal path character",
                "https://example.com/~vincent",
                "https://example.com/~vincent",
            ),
            (
                "multibyte around a decorated url",
                "Éh 🌸 **https://example.com/a** — ça va ?",
                "https://example.com/a",
            ),
        ];
        for (label, input, expected) in cases {
            let html = render_html(&tokenize(input));
            let clicked = clicked_url_in_html(&html);
            assert_eq!(
                clicked,
                *expected,
                "{label}: clicked URL mismatch in HTML mode for {input:?} (html: {html:?}, \
                 displayed: {:?})",
                html_rendered_text(&html)
            );
            url::Url::parse(&clicked).unwrap_or_else(|e| {
                panic!("{label}: clicked URL {clicked:?} is not well-formed: {e}")
            });
        }
    }

    /// A shape mika#2126 never cleaned is not cleaned better in HTML mode — and
    /// **parity is the honest control here, not cleanliness**.
    ///
    /// `[x](https://example.com/a` (no closing paren) is refused by
    /// [`parse_markdown_link`], so the whole token `[x](https://…` is what Telegram's
    /// autolinker takes — today, on the plain path, exactly as much as in HTML mode.
    /// A first draft of R5 listed this among the cases that must yield a clean URL;
    /// that was asserting an ideal mika#2126 does not hold. What mika#2291 owes is
    /// that the armed mode be **no worse** than the floor, so that is what is
    /// asserted — and the residual limit is recorded rather than quietly widened.
    #[test]
    fn mika2291_refused_link_grammar_is_no_worse_in_html_mode() {
        let input = "[x](https://example.com/a";
        let floor = clicked_url(&plain_body(TEST_CHAT_ID, input));
        let armed = clicked_url_in_html(&render_html(&tokenize(input)));
        assert_eq!(
            armed, floor,
            "the armed mode must be no worse than the floor on a refused link shape"
        );
        assert_eq!(
            floor, "[x](https://example.com/a",
            "this records mika#2126's pre-existing residual limit; if the floor ever \
             cleans this shape, update both halves deliberately"
        );
    }

    // -- The fallback decision (AC2) --

    /// The fallback fires on 400 and on nothing else.
    ///
    /// 401 / 403 / 429 / 5xx are returned as-is: replaying them without `parse_mode`
    /// would double a doomed call and, on 429, worsen the rate limit.
    #[test]
    fn mika2291_ac2_fallback_fires_only_on_400() {
        assert!(should_fall_back_to_plain(&TelegramApiError::BadRequest {
            message: "Bad Request: can't parse entities: …".to_string(),
        }));
        for err in [
            TelegramApiError::Unauthorized,
            TelegramApiError::BotBlocked,
            TelegramApiError::RateLimited {
                retry_after: Some(30),
            },
            TelegramApiError::RateLimited { retry_after: None },
            TelegramApiError::Other {
                status: 500,
                body: "internal".to_string(),
            },
            TelegramApiError::Other {
                status: 502,
                body: String::new(),
            },
        ] {
            assert!(
                !should_fall_back_to_plain(&err),
                "the fallback must not fire on {err:?}"
            );
        }
    }

    /// The payload omits `parse_mode` entirely when it is `None`, and names `"HTML"`
    /// when set. Serialization matters here: a `"parse_mode": null` would be rejected
    /// by Telegram, so the floor would break on the very shape that exists to save it.
    #[test]
    fn mika2291_parse_mode_is_omitted_when_absent() {
        let plain = serde_json::to_string(&SendMessagePayload {
            chat_id: 42,
            text: "bonjour".to_string(),
            parse_mode: None,
        })
        .expect("payload serializes");
        assert!(
            !plain.contains("parse_mode"),
            "parse_mode must be omitted, not null: {plain}"
        );

        let html = serde_json::to_string(&SendMessagePayload {
            chat_id: 42,
            text: "<b>bonjour</b>".to_string(),
            parse_mode: Some("HTML"),
        })
        .expect("payload serializes");
        assert!(html.contains("\"parse_mode\":\"HTML\""), "{html}");
    }
}
