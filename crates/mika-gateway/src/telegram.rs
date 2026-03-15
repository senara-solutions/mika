use bytes::Bytes;
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

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

/// Minimal representation of a replied-to message (only message_id needed for routing).
#[derive(Debug, Clone, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct ReplyToMessage {
    pub message_id: i64,
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
    },
    Photo {
        chat_id: i64,
        file_id: String,
        caption: Option<String>,
        update_id: i64,
        reply_to_message_id: Option<i64>,
    },
    Document {
        chat_id: i64,
        file_id: String,
        mime_type: String,
        caption: Option<String>,
        update_id: i64,
        reply_to_message_id: Option<i64>,
    },
    BareStart {
        chat_id: i64,
    },
    Unsupported {
        chat_id: i64,
    },
    NoMessage,
}

/// Image MIME types supported for forwarding to the agent.
const SUPPORTED_IMAGE_MIMES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

/// Parse a Telegram update into a structured message type.
pub fn parse_update(update: &TelegramUpdate) -> ParsedMessage {
    let message = match &update.message {
        Some(m) => m,
        None => return ParsedMessage::NoMessage,
    };

    let chat_id = message.chat.id;
    let reply_to_message_id = message.reply_to_message.as_ref().map(|r| r.message_id);

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
        return ParsedMessage::Text {
            chat_id,
            text: text.clone(),
            update_id: update.update_id,
            reply_to_message_id,
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

/// Telegram API client wrapper.
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

    fn api_url(&self, method: &str) -> String {
        format!(
            "https://api.telegram.org/bot{}/{}",
            self.bot_token.expose_secret(),
            method
        )
    }

    /// Send a text message to a chat. Returns the Telegram message_id on success.
    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<i64, TelegramApiError> {
        let payload = SendMessagePayload {
            chat_id,
            text: text.to_string(),
        };

        let resp = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&payload)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 200 {
            // Parse the message_id from the successful response
            let send_resp: TelegramSendResponse =
                resp.json().await.map_err(|e| TelegramApiError::Other {
                    status: 200,
                    body: format!("failed to parse sendMessage response: {e}"),
                })?;
            return Ok(send_resp.result.map(|r| r.message_id).unwrap_or(0));
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
    async fn get_file(&self, file_id: &str) -> Result<String, TelegramApiError> {
        let url = self.api_url("getFile");
        let resp = self
            .client
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

        let file_resp: GetFileResponse =
            resp.json().await.map_err(|e| TelegramApiError::Other {
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

    /// Download file bytes from Telegram's file server.
    ///
    /// Validates `file_path` against traversal/URL-manipulation attacks, then
    /// checks the `Content-Length` header before reading the body to reject
    /// oversized files early (avoids buffering up to 20 MB only to discard).
    async fn download_file_bytes(&self, file_path: &str) -> Result<Bytes, TelegramApiError> {
        Self::validate_file_path(file_path)?;

        let url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            self.bot_token.expose_secret(),
            file_path
        );
        let resp = self.client.get(&url).send().await?;

        let status = resp.status().as_u16();
        if status != 200 {
            return Err(TelegramApiError::Other {
                status,
                body: format!("file download returned {status}"),
            });
        }

        // Early rejection: check Content-Length header before reading the body.
        // This avoids downloading files between 5-20 MB just to discard them.
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
    /// Returns the raw bytes and detected media type.
    pub async fn download_image(&self, file_id: &str) -> Result<DownloadedImage, TelegramApiError> {
        let file_path = self.get_file(file_id).await?;
        let bytes = self.download_file_bytes(&file_path).await?;

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

    /// Register the webhook URL with Telegram. Verifies `ok: true` response.
    pub async fn set_webhook(&self, webhook_url: &str, webhook_secret: &str) -> anyhow::Result<()> {
        let payload = SetWebhookPayload {
            url: webhook_url.to_string(),
            secret_token: webhook_secret.to_string(),
            allowed_updates: vec!["message".to_string()],
            max_connections: 30,
        };

        let resp = self
            .client
            .post(self.api_url("setWebhook"))
            .json(&payload)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("setWebhook request failed: {e}"))?;

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
}

impl std::fmt::Debug for TelegramClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramClient")
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
            }
        );
    }

    // -- Reply-to-message tests --

    #[test]
    fn test_parse_reply_message_extracts_id() {
        let mut msg = text_msg(42, Some("replying"));
        msg.reply_to_message = Some(ReplyToMessage { message_id: 999 });
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
            }
        );
    }

    // -- file_path validation tests --

    #[test]
    fn test_validate_file_path_accepts_normal_path() {
        assert!(TelegramClient::validate_file_path("photos/file_1.jpg").is_ok());
    }

    #[test]
    fn test_validate_file_path_accepts_nested_path() {
        assert!(TelegramClient::validate_file_path("documents/user/photo.png").is_ok());
    }

    #[test]
    fn test_validate_file_path_rejects_empty() {
        let err = TelegramClient::validate_file_path("").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_validate_file_path_rejects_leading_slash() {
        let err = TelegramClient::validate_file_path("/etc/passwd").unwrap_err();
        assert!(err.to_string().contains("starts with /"));
    }

    #[test]
    fn test_validate_file_path_rejects_traversal() {
        let err = TelegramClient::validate_file_path("photos/../../etc/passwd").unwrap_err();
        assert!(err.to_string().contains("contains .."));
    }

    #[test]
    fn test_validate_file_path_rejects_bare_traversal() {
        let err = TelegramClient::validate_file_path("..").unwrap_err();
        assert!(err.to_string().contains("contains .."));
    }

    #[test]
    fn test_validate_file_path_rejects_at_sign() {
        let err = TelegramClient::validate_file_path("photos/@evil.com/file").unwrap_err();
        assert!(err.to_string().contains("contains '@'"));
    }

    #[test]
    fn test_validate_file_path_rejects_question_mark() {
        let err = TelegramClient::validate_file_path("photos/file?token=leak").unwrap_err();
        assert!(err.to_string().contains("contains '?'"));
    }

    #[test]
    fn test_validate_file_path_rejects_hash() {
        let err = TelegramClient::validate_file_path("photos/file#fragment").unwrap_err();
        assert!(err.to_string().contains("contains '#'"));
    }
}
