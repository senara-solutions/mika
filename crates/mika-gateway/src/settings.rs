use config::{Config, Environment};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

/// Gateway-specific settings, loaded from MIKA_* environment variables.
#[derive(Deserialize, Clone)]
pub struct GatewaySettings {
    /// Postgres connection string
    pub database_url: SecretString,

    /// Telegram Bot API token (required only in single-bot mode)
    #[serde(default)]
    pub telegram_bot_token: Option<SecretString>,

    /// Secret token for validating inbound Telegram webhooks (required only in single-bot mode)
    #[serde(default)]
    pub telegram_webhook_secret: Option<SecretString>,

    /// Public URL Telegram calls for webhook delivery (required only in single-bot mode)
    #[serde(default)]
    pub telegram_webhook_url: Option<String>,

    /// Single-bot mode: when "1" or "true", the gateway uses the global
    /// MIKA_TELEGRAM_BOT_TOKEN for all customers (legacy behavior).
    /// Requires MIKA_TELEGRAM_BOT_TOKEN, MIKA_TELEGRAM_WEBHOOK_SECRET,
    /// and MIKA_TELEGRAM_WEBHOOK_URL to be set. Default: false.
    #[serde(default)]
    pub telegram_single_bot_mode: Option<String>,

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

    /// GitHub App ID (u64). Required for GitHub App authentication.
    #[serde(default)]
    pub github_app_id: Option<u64>,

    /// GitHub App private key (base64-encoded PEM).
    #[serde(default)]
    pub github_app_private_key: Option<SecretString>,

    /// GitHub App installation ID for the org (u64).
    #[serde(default)]
    pub github_app_installation_id: Option<u64>,

    /// Orchestrator inbox feature flag (mika#1189).
    /// Default: off (`/orchestrator/inbox/*` returns 404). Set `1` to enable
    /// dual-write with the filesystem inbox (`mika-platform#100`). `2`
    /// (gateway-only cutover) is reserved for a future ticket.
    #[serde(default)]
    pub orchestrator_inbox_enabled: Option<String>,
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

        // Validate internal token
        validate_hex_token(&settings.internal_token, "MIKA_INTERNAL_TOKEN")?;

        // Single-bot mode requires bot token, webhook secret, and webhook URL
        if telegram_single_bot_mode_is_enabled(settings.telegram_single_bot_mode.as_deref()) {
            if settings.telegram_bot_token.is_none() {
                anyhow::bail!(
                    "MIKA_TELEGRAM_SINGLE_BOT_MODE is enabled but MIKA_TELEGRAM_BOT_TOKEN is not set"
                );
            }
            if settings.telegram_webhook_secret.is_none() {
                anyhow::bail!(
                    "MIKA_TELEGRAM_SINGLE_BOT_MODE is enabled but MIKA_TELEGRAM_WEBHOOK_SECRET is not set"
                );
            }
            if settings.telegram_webhook_url.is_none() {
                anyhow::bail!(
                    "MIKA_TELEGRAM_SINGLE_BOT_MODE is enabled but MIKA_TELEGRAM_WEBHOOK_URL is not set"
                );
            }

            // Validate webhook URL is well-formed
            reqwest::Url::parse(settings.telegram_webhook_url.as_ref().unwrap()).map_err(|e| {
                anyhow::anyhow!("MIKA_TELEGRAM_WEBHOOK_URL is not a valid URL: {e}")
            })?;

            // Validate webhook secret format
            validate_hex_token(
                settings.telegram_webhook_secret.as_ref().unwrap(),
                "MIKA_TELEGRAM_WEBHOOK_SECRET",
            )?;
        }

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
            .field(
                "telegram_bot_token",
                &self.telegram_bot_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "telegram_webhook_secret",
                &self.telegram_webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("telegram_webhook_url", &self.telegram_webhook_url)
            .field("telegram_single_bot_mode", &self.telegram_single_bot_mode)
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
            .field("github_app_id", &self.github_app_id)
            .field(
                "github_app_private_key",
                &self.github_app_private_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "github_app_installation_id",
                &self.github_app_installation_id,
            )
            .field(
                "orchestrator_inbox_enabled",
                &self.orchestrator_inbox_enabled,
            )
            .finish()
    }
}

/// Parse `MIKA_TELEGRAM_SINGLE_BOT_MODE`. Treats `1` / `true` (case-insensitive)
/// as enabled; everything else as disabled.
pub fn telegram_single_bot_mode_is_enabled(raw: Option<&str>) -> bool {
    match raw.map(str::trim) {
        Some(v) => {
            let lower = v.to_ascii_lowercase();
            lower == "1" || lower == "true"
        }
        None => false,
    }
}

/// Parse `MIKA_ORCHESTRATOR_INBOX_ENABLED`. Treats `1` / `true` (case-insensitive)
/// as enabled; everything else (unset, empty, `0`, `false`, or any other value)
/// as disabled. The `2` (gateway-only) value is reserved for a future ticket
/// and currently treated as disabled to avoid silent partial cutover.
pub fn orchestrator_inbox_is_enabled(raw: Option<&str>) -> bool {
    match raw.map(str::trim) {
        Some(v) => {
            let lower = v.to_ascii_lowercase();
            lower == "1" || lower == "true"
        }
        None => false,
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
                telegram_bot_token: Some(SecretString::from("123:ABC")),
                telegram_webhook_secret: Some(SecretString::from("a".repeat(64))),
                telegram_webhook_url: Some("https://example.com/webhook".to_string()),
                telegram_single_bot_mode: Some("1".to_string()),
                internal_token: SecretString::from("b".repeat(64)),
                gateway_port: 8080,
                log_level: "info".to_string(),
                log_format: "json".to_string(),
                agent_base_url: None,
                gateway_log_file: None,
                agents_namespace: "mika-agents".to_string(),
                github_webhook_secret: Some(SecretString::from("gh-webhook-secret")),
                github_app_id: Some(12345),
                github_app_private_key: Some(SecretString::from("super-secret-pem")),
                github_app_installation_id: Some(67890),
                orchestrator_inbox_enabled: None,
            }
        );
        assert!(!debug.contains("pass"));
        assert!(!debug.contains("ABC"));
        assert!(!debug.contains("token-123"));
        assert!(!debug.contains("gh-webhook-secret"));
        assert!(!debug.contains("super-secret-pem"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn test_orchestrator_inbox_enabled_default_off() {
        assert!(!orchestrator_inbox_is_enabled(None));
        assert!(!orchestrator_inbox_is_enabled(Some("")));
    }

    #[test]
    fn test_orchestrator_inbox_enabled_accepts_one() {
        assert!(orchestrator_inbox_is_enabled(Some("1")));
    }

    #[test]
    fn test_orchestrator_inbox_enabled_accepts_true_case_insensitive() {
        assert!(orchestrator_inbox_is_enabled(Some("true")));
        assert!(orchestrator_inbox_is_enabled(Some("True")));
        assert!(orchestrator_inbox_is_enabled(Some("TRUE")));
    }

    #[test]
    fn test_orchestrator_inbox_enabled_rejects_zero() {
        // Plan: unset / `0` are equivalently disabled.
        assert!(!orchestrator_inbox_is_enabled(Some("0")));
        assert!(!orchestrator_inbox_is_enabled(Some("false")));
    }

    #[test]
    fn test_orchestrator_inbox_enabled_rejects_two() {
        // `2` (gateway-only) is reserved and currently treated as disabled.
        // Prevents partial-cutover silent enable.
        assert!(!orchestrator_inbox_is_enabled(Some("2")));
    }

    #[test]
    fn test_orchestrator_inbox_enabled_handles_whitespace() {
        assert!(orchestrator_inbox_is_enabled(Some(" 1 ")));
        assert!(orchestrator_inbox_is_enabled(Some("\ttrue\n")));
    }

    // -- telegram_single_bot_mode tests --

    #[test]
    fn test_single_bot_mode_default_off() {
        assert!(!telegram_single_bot_mode_is_enabled(None));
        assert!(!telegram_single_bot_mode_is_enabled(Some("")));
    }

    #[test]
    fn test_single_bot_mode_accepts_one() {
        assert!(telegram_single_bot_mode_is_enabled(Some("1")));
    }

    #[test]
    fn test_single_bot_mode_accepts_true_case_insensitive() {
        assert!(telegram_single_bot_mode_is_enabled(Some("true")));
        assert!(telegram_single_bot_mode_is_enabled(Some("True")));
        assert!(telegram_single_bot_mode_is_enabled(Some("TRUE")));
    }

    #[test]
    fn test_single_bot_mode_rejects_zero() {
        assert!(!telegram_single_bot_mode_is_enabled(Some("0")));
        assert!(!telegram_single_bot_mode_is_enabled(Some("false")));
    }

    #[test]
    fn test_single_bot_mode_handles_whitespace() {
        assert!(telegram_single_bot_mode_is_enabled(Some(" 1 ")));
        assert!(telegram_single_bot_mode_is_enabled(Some("\ttrue\n")));
    }
}
