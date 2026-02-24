use config::{Config, Environment};
use secrecy::SecretString;
use serde::Deserialize;

/// Gateway-specific settings, loaded from MIKA_* environment variables.
#[derive(Deserialize, Clone)]
pub struct GatewaySettings {
    /// Postgres connection string
    pub database_url: SecretString,

    /// Telegram Bot API token
    pub telegram_bot_token: SecretString,

    /// Secret token for validating inbound Telegram webhooks
    pub telegram_webhook_secret: String,

    /// Public URL Telegram calls for webhook delivery
    pub telegram_webhook_url: String,

    /// Shared bearer token for gateway ↔ container auth
    pub internal_token: SecretString,

    /// Listen port (default: 8080)
    #[serde(default = "default_port")]
    pub gateway_port: u16,

    /// Log level (default: "info")
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_port() -> u16 {
    8080
}

fn default_log_level() -> String {
    "info".to_string()
}

impl GatewaySettings {
    /// Load settings from MIKA_* environment variables.
    pub fn load() -> anyhow::Result<Self> {
        let settings: Self = Config::builder()
            .add_source(
                Environment::with_prefix("MIKA")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()?
            .try_deserialize()?;

        // Validate webhook URL is well-formed
        reqwest::Url::parse(&settings.telegram_webhook_url)
            .map_err(|e| anyhow::anyhow!("MIKA_TELEGRAM_WEBHOOK_URL is not a valid URL: {e}"))?;

        Ok(settings)
    }
}

impl std::fmt::Debug for GatewaySettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewaySettings")
            .field("database_url", &"[REDACTED]")
            .field("telegram_bot_token", &"[REDACTED]")
            .field("telegram_webhook_secret", &"[REDACTED]")
            .field("telegram_webhook_url", &self.telegram_webhook_url)
            .field("internal_token", &"[REDACTED]")
            .field("gateway_port", &self.gateway_port)
            .field("log_level", &self.log_level)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_redacts_secrets() {
        let debug = format!(
            "{:?}",
            GatewaySettings {
                database_url: SecretString::from("postgres://user:pass@localhost/db"),
                telegram_bot_token: SecretString::from("123:ABC"),
                telegram_webhook_secret: "secret".to_string(),
                telegram_webhook_url: "https://example.com/webhook".to_string(),
                internal_token: SecretString::from("token-123"),
                gateway_port: 8080,
                log_level: "info".to_string(),
            }
        );
        assert!(!debug.contains("pass"));
        assert!(!debug.contains("ABC"));
        assert!(!debug.contains("token-123"));
        assert!(debug.contains("[REDACTED]"));
    }
}
