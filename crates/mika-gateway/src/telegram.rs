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

/// **No `parse_mode` — and that is a decision, not an omission (mika#2126).**
///
/// If you came here to "just turn on MarkdownV2", read this first; it was weighed and
/// rejected.
///
/// MarkdownV2 requires escaping `_ * [ ] ( ) ~ ` > # + - = | { } . !` throughout the
/// *entire* text, URLs included. A single unescaped character makes the Telegram API
/// reject the **whole message** with a 400. We would then have traded a broken link
/// for an **absent message** — a clear regression, because today the user at least
/// receives the text. The property that decides it: **plain text cannot be broken by
/// a renderer that never parses it.**
///
/// The cost of plain text is that markdown the agent writes arrives literally, and
/// Telegram's autolinker swallows any decoration glued to a URL (`**https://…/mika**`
/// → a link to `…/mika**` → 404). That is handled at the single emission point by
/// [`strip_markdown_around_urls`], which carries the rule and its residual risk.
///
/// Changing this field means owning the escaping of every outbound message, including
/// agent-authored text we do not control. Take it back through grooming, not here.
#[derive(Debug, Serialize)]
struct SendMessagePayload {
    chat_id: i64,
    text: String,
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
fn parse_markdown_link(chars: &[char], open: usize) -> Option<(String, String, usize)> {
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
                t.truncate(t.len() - n);
            }
        }

        if t.len() == before {
            break;
        }
    }
    t.iter().collect()
}

/// Send a text message to a chat via the Telegram Bot API.
/// Shared implementation for both client types. Returns the Telegram message_id on success.
async fn send_message_impl(
    client: &Client,
    bot_token: &str,
    chat_id: i64,
    text: &str,
) -> Result<i64, TelegramApiError> {
    // Single emission point (mika#2126): every present and future caller inherits the
    // cleaning here, so no individual call site has to remember it.
    let cleaned = strip_markdown_around_urls(text);
    if cleaned != text {
        // Metrics only — never the message body (user data). Without this counter,
        // "the rule stopped firing because the agent stopped decorating" and "the rule
        // stopped matching" look identical from the gateway. `url_tokens` counts the
        // scheme-bearing tokens the rule acted on in the incoming text; keeping
        // `strip_markdown_around_urls` at its pinned infallible `-> String` signature
        // is worth more than an exact touched-count.
        let url_tokens = text
            .split_whitespace()
            .filter(|t| t.contains("http://") || t.contains("https://"))
            .count();
        debug!(
            chat_id,
            len_before = text.len(),
            len_after = cleaned.len(),
            url_tokens,
            "stripped markdown decoration glued to outbound URL(s)"
        );
    }

    let payload = SendMessagePayload {
        chat_id,
        text: cleaned,
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
}
