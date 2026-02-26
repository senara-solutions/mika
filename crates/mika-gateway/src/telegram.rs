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

// -- Telegram Update types (minimal, only what we need) --

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TelegramMessage {
    pub chat: TelegramChat,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TelegramChat {
    pub id: i64,
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
    },
    BareStart {
        chat_id: i64,
    },
    Unsupported {
        chat_id: i64,
    },
    NoMessage,
}

/// Parse a Telegram update into a structured message type.
pub fn parse_update(update: &TelegramUpdate) -> ParsedMessage {
    let message = match &update.message {
        Some(m) => m,
        None => return ParsedMessage::NoMessage,
    };

    let chat_id = message.chat.id;

    match &message.text {
        Some(text) => {
            if text == "/start" {
                return ParsedMessage::BareStart { chat_id };
            }
            if let Some(payload) = text.strip_prefix("/start ") {
                let token = payload.trim();
                if token.is_empty() {
                    return ParsedMessage::Unsupported { chat_id };
                }
                ParsedMessage::Start {
                    chat_id,
                    pairing_token: token.to_string(),
                }
            } else {
                ParsedMessage::Text {
                    chat_id,
                    text: text.clone(),
                    update_id: update.update_id,
                }
            }
        }
        None => ParsedMessage::Unsupported { chat_id },
    }
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

    /// Send a text message to a chat. Returns Ok(()) on success.
    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), TelegramApiError> {
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
            return Ok(());
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

    #[test]
    fn test_parse_text_message() {
        let update = TelegramUpdate {
            update_id: 100,
            message: Some(TelegramMessage {
                chat: TelegramChat { id: 42 },
                text: Some("Hello Mika!".to_string()),
            }),
        };
        assert_eq!(
            parse_update(&update),
            ParsedMessage::Text {
                chat_id: 42,
                text: "Hello Mika!".to_string(),
                update_id: 100,
            }
        );
    }

    #[test]
    fn test_parse_start_command() {
        let token = "a1b2c3d4e5f6";
        let update = TelegramUpdate {
            update_id: 101,
            message: Some(TelegramMessage {
                chat: TelegramChat { id: 42 },
                text: Some(format!("/start {token}")),
            }),
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
            message: Some(TelegramMessage {
                chat: TelegramChat { id: 42 },
                text: Some("/start  abc123  ".to_string()),
            }),
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
            message: Some(TelegramMessage {
                chat: TelegramChat { id: 42 },
                text: Some("/start ".to_string()),
            }),
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
            message: Some(TelegramMessage {
                chat: TelegramChat { id: 42 },
                text: None,
            }),
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
            message: Some(TelegramMessage {
                chat: TelegramChat { id: 42 },
                text: Some("/start".to_string()),
            }),
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
}
