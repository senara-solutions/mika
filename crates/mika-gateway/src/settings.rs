use config::{Config, Environment};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

/// Gateway-specific settings, loaded from MIKA_* environment variables.
#[derive(Deserialize, Clone)]
pub struct GatewaySettings {
    /// Postgres connection string
    pub database_url: SecretString,

    /// Telegram Bot API token
    pub telegram_bot_token: SecretString,

    /// Secret token for validating inbound Telegram webhooks
    pub telegram_webhook_secret: SecretString,

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

    /// Stdout log format: "json" (default) or "pretty"
    #[serde(default = "default_log_format")]
    pub log_format: String,

    /// Optional override for agent container base URL (for local E2E testing).
    /// When set, all messages route to this URL instead of internal DNS.
    /// Example: "http://localhost:8080"
    #[serde(default)]
    pub agent_base_url: Option<String>,

    /// Optional log file path for mika-gateway (maps to MIKA_GATEWAY_LOG_FILE)
    #[serde(default)]
    pub gateway_log_file: Option<String>,

    /// Namespace where agent pods run (for FQDN construction).
    /// Maps to MIKA_AGENTS_NAMESPACE env var. Default: "mika-agents".
    #[serde(default = "default_agents_namespace")]
    pub agents_namespace: String,

    /// Secret for validating inbound GitHub App webhooks (HMAC-SHA256).
    /// Optional — when absent, `POST /webhook/github` returns 404.
    /// GitHub webhook secrets are arbitrary strings (not hex-constrained).
    #[serde(default)]
    pub github_webhook_secret: Option<SecretString>,
}

fn default_port() -> u16 {
    8080
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> String {
    "json".to_string()
}

fn default_agents_namespace() -> String {
    "mika-agents".to_string()
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
            .try_deserialize()
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to load gateway settings: {e}. \
                 Run `mika setup --mode compose` to generate a .env file, \
                 or set the required MIKA_* env vars directly."
                )
            })?;

        // Validate webhook URL is well-formed
        reqwest::Url::parse(&settings.telegram_webhook_url)
            .map_err(|e| anyhow::anyhow!("MIKA_TELEGRAM_WEBHOOK_URL is not a valid URL: {e}"))?;

        // Validate tokens are fixed-length hex (eliminates constant_time_eq length timing leak)
        validate_hex_token(&settings.internal_token, "MIKA_INTERNAL_TOKEN")?;
        validate_hex_token(
            &settings.telegram_webhook_secret,
            "MIKA_TELEGRAM_WEBHOOK_SECRET",
        )?;

        // Validate agent_base_url scheme when set (dev-only override)
        if let Some(ref url_str) = settings.agent_base_url {
            validate_agent_base_url(url_str)?;
        }

        Ok(settings)
    }
}

/// Validate MIKA_AGENT_BASE_URL: must be a well-formed URL with an http/https scheme.
/// Emits a warning when the host is not localhost/127.x/::1 because this setting is
/// intended only for local E2E testing.
fn validate_agent_base_url(url_str: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(url_str)
        .map_err(|e| anyhow::anyhow!("MIKA_AGENT_BASE_URL is not a valid URL: {e}"))?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        anyhow::bail!(
            "MIKA_AGENT_BASE_URL has unsupported scheme '{scheme}': must be http or https"
        );
    }
    let is_local = url
        .host_str()
        .map(|h| h == "localhost" || h.starts_with("127.") || h == "::1")
        .unwrap_or(false);
    if !is_local {
        tracing::warn!(
            url = %url_str,
            "MIKA_AGENT_BASE_URL is set to a non-localhost host; \
             this setting is intended for local E2E testing only"
        );
    }
    Ok(())
}

fn validate_hex_token(token: &SecretString, name: &str) -> anyhow::Result<()> {
    let val = token.expose_secret();
    if val.len() != 64 || !val.bytes().all(|b| b.is_ascii_hexdigit()) {
        anyhow::bail!("{name} must be exactly 64 hex characters (32 bytes hex-encoded)");
    }
    Ok(())
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
            .field("log_format", &self.log_format)
            .field("agent_base_url", &self.agent_base_url)
            .field("gateway_log_file", &self.gateway_log_file)
            .field("agents_namespace", &self.agents_namespace)
            .field(
                "github_webhook_secret",
                &self.github_webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_agent_base_url_accepts_http_localhost() {
        assert!(validate_agent_base_url("http://localhost:8080").is_ok());
    }

    #[test]
    fn test_validate_agent_base_url_accepts_https_localhost() {
        assert!(validate_agent_base_url("https://localhost").is_ok());
    }

    #[test]
    fn test_validate_agent_base_url_accepts_127_x() {
        assert!(validate_agent_base_url("http://127.0.0.1:3000").is_ok());
    }

    #[test]
    fn test_validate_agent_base_url_rejects_invalid_url() {
        let err = validate_agent_base_url("not-a-url").unwrap_err();
        assert!(
            err.to_string()
                .contains("MIKA_AGENT_BASE_URL is not a valid URL"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_validate_agent_base_url_rejects_bad_scheme() {
        let err = validate_agent_base_url("ftp://localhost/path").unwrap_err();
        assert!(
            err.to_string().contains("unsupported scheme"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_validate_agent_base_url_accepts_non_local_with_warning() {
        // Non-localhost URLs are allowed but should trigger a tracing::warn.
        // We can only assert that the function succeeds (warn is a side-effect).
        assert!(validate_agent_base_url("https://my-agent.internal.example.com").is_ok());
    }

    #[test]
    fn test_debug_redacts_secrets() {
        let debug = format!(
            "{:?}",
            GatewaySettings {
                database_url: SecretString::from("postgres://user:pass@localhost/db"),
                telegram_bot_token: SecretString::from("123:ABC"),
                telegram_webhook_secret: SecretString::from("a".repeat(64)),
                telegram_webhook_url: "https://example.com/webhook".to_string(),
                internal_token: SecretString::from("b".repeat(64)),
                gateway_port: 8080,
                log_level: "info".to_string(),
                log_format: "json".to_string(),
                agent_base_url: None,
                gateway_log_file: None,
                agents_namespace: "mika-agents".to_string(),
                github_webhook_secret: Some(SecretString::from("gh-webhook-secret")),
            }
        );
        assert!(!debug.contains("pass"));
        assert!(!debug.contains("ABC"));
        assert!(!debug.contains("token-123"));
        assert!(!debug.contains("gh-webhook-secret"));
        assert!(debug.contains("[REDACTED]"));
    }
}
