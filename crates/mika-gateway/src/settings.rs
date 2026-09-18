use config::{Config, Environment};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use subtle::ConstantTimeEq;

/// Gateway-specific settings, loaded from MIKA_* environment variables.
#[derive(Deserialize, Clone)]
pub struct GatewaySettings {
    /// Postgres connection string
    pub database_url: SecretString,

    /// Telegram Bot API token. When set, the global outbound client is built
    /// regardless of single-bot mode (mika#1590).
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

    /// Telegram HTML rendering kill-switch (mika#2291). **Default: armed.**
    ///
    /// `0` / `false` / `off` / `no` (case-insensitive, whitespace tolerated) disarm
    /// it; absent, empty, or anything unrecognized leaves it **armed**. See
    /// [`telegram_html_render_is_enabled`] — note its polarity is the inverse of its
    /// two neighbours here.
    ///
    /// **`Option<String>` and never `bool`, which is not a style detail.**
    /// [`GatewaySettings::load`] deserializes the environment through config-rs, so a
    /// `bool` field receiving `"plif"` makes `load()` fail — the gateway **refuses to
    /// start**, on a generic error pointing at `mika setup` without naming the
    /// offending variable. A typo in a p2 cosmetic flag would then take down all
    /// Telegram traffic. Refusing to boot on an invalid LLM budget protects against a
    /// mute agent; refusing to boot on a rendering flag protects against nothing.
    /// The crate has settled this class twice already: no `bool` lives in this
    /// struct.
    #[serde(default)]
    pub telegram_html_render: Option<String>,

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

    /// Public HTTPS base URL of the gateway (e.g., `https://gateway.mika.example.com`).
    /// Required for per-customer webhook registration — the endpoint constructs
    /// `{gateway_external_url}/webhook/telegram/{customer_id}` as the Telegram webhook URL.
    /// Maps to `MIKA_GATEWAY_EXTERNAL_URL`.
    #[serde(default)]
    pub gateway_external_url: Option<String>,

    /// Base URL of the control-monitor (cm-api) HTTP surface (e.g.,
    /// `http://127.0.0.1:8090`). When set, every validated inbound GitHub
    /// webhook is fire-and-forget forwarded to `{cm_api_url}/api/v1/webhooks/github`
    /// with the raw payload + `X-Hub-Signature-256` + `X-GitHub-Event` headers
    /// preserved so cm-api can re-verify HMAC against its own per-entity
    /// `webhook_secret`. See cm#88 Option B — the gateway is the deployed
    /// reachability path (`webhook.dupont.tech → Freebox → NAS nginx → gentux
    /// mika-gateway`); cm-api sits behind it as an additional subscriber.
    /// When absent, cm-forwarding is disabled and the gateway behaves as
    /// pre-cm#88. Maps to `MIKA_CM_API_URL`.
    #[serde(default)]
    pub cm_api_url: Option<String>,

    /// E1 egress-search substrate upstream selector (mika#1807). One of
    /// `"brave"`. `None` disables `POST /internal/search` (endpoint returns
    /// 404). E2 (#1808) wires the concrete upstream call — in v1 the
    /// endpoint returns 501 `not_implemented` when configured.
    ///
    /// See `crates/mika-gateway/src/egress_search.rs` module doc and
    /// `crates/mika-gateway/docs/egress-search.md` for the Q1/Q2/Q3/Q4
    /// tranchage.
    #[serde(default)]
    pub search_upstream: Option<String>,

    /// Brave Search API key (mika#1807). Required when `search_upstream = "brave"`.
    /// Maps to `MIKA_BRAVE_API_KEY`.
    #[serde(default)]
    pub brave_api_key: Option<SecretString>,

    /// Optional override for the Brave API endpoint (mika#1807). Used by
    /// E2 integration tests + self-hosted Brave mirrors. Defaults to
    /// `crate::egress_search::DEFAULT_BRAVE_ENDPOINT`.
    #[serde(default)]
    pub brave_endpoint: Option<String>,

    /// mika#2360 — admin READ-ONLY token. Opens
    /// `GET /admin/tenants/{customer_id}/recurring-tasks` and nothing else.
    /// Maps to `MIKA_GATEWAY_ADMIN_READ_TOKEN`.
    ///
    /// Optional: absent ⇒ the route answers 404 (fail-closed, like
    /// `github_webhook_secret`). Refused ⇒ route disarmed with a WARN when it
    /// equals `internal_token` — a copy-paste of the write secret would erase
    /// the read/write segregation this token exists to create. Deliberately
    /// NOT part of [`GatewaySettings::validate`]: a malformed read token must
    /// not be able to take the whole gateway down (see
    /// [`resolve_admin_read_token`]).
    #[serde(default)]
    pub gateway_admin_read_token: Option<SecretString>,
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

        settings.validate()?;
        Ok(settings)
    }

    /// Validate settings constraints. Extracted from `load()` for testability.
    fn validate(&self) -> anyhow::Result<()> {
        // Validate internal token
        validate_hex_token(&self.internal_token, "MIKA_INTERNAL_TOKEN")?;

        // Single-bot mode requires bot token, webhook secret, and webhook URL
        // for inbound webhook registration. When single-bot mode is off,
        // bot_token alone is sufficient for outbound delivery (mika#1590).
        if telegram_single_bot_mode_is_enabled(self.telegram_single_bot_mode.as_deref()) {
            if self.telegram_bot_token.is_none() {
                anyhow::bail!(
                    "MIKA_TELEGRAM_SINGLE_BOT_MODE is enabled but MIKA_TELEGRAM_BOT_TOKEN is not set"
                );
            }
            if self.telegram_webhook_secret.is_none() {
                anyhow::bail!(
                    "MIKA_TELEGRAM_SINGLE_BOT_MODE is enabled but MIKA_TELEGRAM_WEBHOOK_SECRET is not set"
                );
            }
            if self.telegram_webhook_url.is_none() {
                anyhow::bail!(
                    "MIKA_TELEGRAM_SINGLE_BOT_MODE is enabled but MIKA_TELEGRAM_WEBHOOK_URL is not set"
                );
            }

            // Validate webhook URL is well-formed
            reqwest::Url::parse(self.telegram_webhook_url.as_ref().unwrap()).map_err(|e| {
                anyhow::anyhow!("MIKA_TELEGRAM_WEBHOOK_URL is not a valid URL: {e}")
            })?;

            // Validate webhook secret format
            validate_hex_token(
                self.telegram_webhook_secret.as_ref().unwrap(),
                "MIKA_TELEGRAM_WEBHOOK_SECRET",
            )?;
        }

        // Validate agent_base_url scheme when set (dev-only override)
        if let Some(ref url_str) = self.agent_base_url {
            validate_agent_base_url(url_str)?;
        }

        // Validate egress-search upstream selector (mika#1807).
        // Only `"brave"` is recognized in v1. Unknown values hard-fail so a
        // typo doesn't silently degrade the endpoint to 404.
        if let Some(ref kind) = self.search_upstream {
            match kind.trim().to_ascii_lowercase().as_str() {
                "brave" => {
                    if self.brave_api_key.is_none() {
                        anyhow::bail!(
                            "MIKA_SEARCH_UPSTREAM='brave' but MIKA_BRAVE_API_KEY is not set"
                        );
                    }
                }
                "" => {
                    // Empty value == absent; ignore.
                }
                other => {
                    anyhow::bail!(
                        "MIKA_SEARCH_UPSTREAM='{other}' is unrecognized; v1 supports only 'brave'"
                    );
                }
            }
        }

        Ok(())
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
            .field("telegram_html_render", &self.telegram_html_render)
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
            .field("gateway_external_url", &self.gateway_external_url)
            .field("search_upstream", &self.search_upstream)
            .field(
                "brave_api_key",
                &self.brave_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("brave_endpoint", &self.brave_endpoint)
            .field(
                "gateway_admin_read_token",
                &self.gateway_admin_read_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// mika#2360 — resolve the admin read token once, at startup.
///
/// Returns the token that arms `GET /admin/tenants/{id}/recurring-tasks`, or
/// `None` when the route must stay 404. Two disarming cases, both logged so a
/// 404 on a route that exists is never read as "not deployed":
///
/// - unset / empty ⇒ INFO, route disabled;
/// - equal to `internal_token` ⇒ WARN, route disabled. Sharing the write
///   secret would silently void the segregation (R8) without any AC2 test
///   being able to see it.
///
/// Disarm rather than `bail!`: taking Telegram routing, GitHub webhooks and
/// `/send` down for every tenant because of a read-only inspection token
/// would be a ransom. The invalid state is resolved here, before `AppState`
/// exists, so no request path can forget to check it.
pub fn resolve_admin_read_token(
    raw: Option<&SecretString>,
    internal_token: &SecretString,
) -> Option<SecretString> {
    let token = raw?;
    let exposed = token.expose_secret();
    if exposed.trim().is_empty() {
        tracing::info!(
            "admin read route disabled (MIKA_GATEWAY_ADMIN_READ_TOKEN empty) — \
             GET /admin/tenants/{{id}}/recurring-tasks answers 404"
        );
        return None;
    }
    if bool::from(
        exposed
            .as_bytes()
            .ct_eq(internal_token.expose_secret().as_bytes()),
    ) {
        tracing::warn!(
            "mika#2360: MIKA_GATEWAY_ADMIN_READ_TOKEN equals MIKA_INTERNAL_TOKEN — \
             the read/write segregation would be void; admin read route DISARMED \
             (GET /admin/tenants/{{id}}/recurring-tasks answers 404). Generate a \
             distinct secret."
        );
        return None;
    }
    tracing::info!(
        "admin read route enabled (MIKA_GATEWAY_ADMIN_READ_TOKEN set) — \
         GET /admin/tenants/{{id}}/recurring-tasks"
    );
    Some(token.clone())
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

/// Parse `MIKA_TELEGRAM_HTML_RENDER` (mika#2291).
///
/// **The polarity is the inverse of both its neighbours in this file, and this line
/// is here so a copy-paste cannot get it silently wrong.** Returns `true` (armed) on
/// `None`, on empty, **and on any unrecognized value**;  returns `false` only for an
/// explicit `0` / `false` / `off` / `no` (case-insensitive, surrounding whitespace
/// tolerated). [`telegram_single_bot_mode_is_enabled`] and
/// [`orchestrator_inbox_is_enabled`] both do the opposite — they return `false` on
/// `None` and absorb every unknown value into `false`, without a WARN — so a body
/// copied from either of them would yield a kill-switch **disarmed by default**, the
/// exact inverse of the decision.
///
/// An unrecognized value leans toward the armed default rather than toward
/// disarming: a typo must not silently switch the rendering off. It is named in a
/// WARN **between quotes** — without the quotes a stray space is invisible
/// (mika#2220).
pub fn telegram_html_render_is_enabled(raw: Option<&str>) -> bool {
    let Some(value) = raw.map(str::trim) else {
        return true;
    };
    if value.is_empty() {
        return true;
    }
    match value.to_ascii_lowercase().as_str() {
        "0" | "false" | "off" | "no" => false,
        "1" | "true" | "on" | "yes" => true,
        _ => {
            // The **trimmed original**, not the lowercased match subject: quoting the
            // value exists to preserve diagnostic fidelity (mika#2220), and folding
            // its case throws away part of what the operator actually typed.
            tracing::warn!(
                event = "telegram_html_render_unrecognized_value",
                value = %format!("{value:?}"),
                "mika#2291: MIKA_TELEGRAM_HTML_RENDER carries an unrecognized value — \
                 HTML rendering stays ARMED (the default). Use 0/false/off/no to disarm."
            );
            true
        }
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
                telegram_html_render: None,
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
                gateway_external_url: Some("https://gateway.test.example.com".to_string()),
                cm_api_url: Some("http://127.0.0.1:8090".to_string()),
                search_upstream: Some("brave".to_string()),
                brave_api_key: Some(SecretString::from("brave-api-key-secret")),
                brave_endpoint: None,
                gateway_admin_read_token: Some(SecretString::from("admin-read-sentinel")),
            }
        );
        assert!(!debug.contains("pass"));
        assert!(!debug.contains("ABC"));
        assert!(!debug.contains("token-123"));
        assert!(!debug.contains("gh-webhook-secret"));
        assert!(!debug.contains("super-secret-pem"));
        assert!(!debug.contains("brave-api-key-secret"));
        assert!(debug.contains("[REDACTED]"));
        // mika#2360 — this Debug is exhaustive (`.finish()`): a field added to
        // the struct without its line here compiles silently. Both halves are
        // needed: the name must be present (no silent omission) and the value
        // must be absent (no leak).
        assert!(
            debug.contains("gateway_admin_read_token"),
            "gateway_admin_read_token omitted from GatewaySettings::Debug"
        );
        assert!(!debug.contains("admin-read-sentinel"));
    }

    // -- mika#2360 admin read token resolution --

    #[test]
    fn mika2360_admin_read_token_unset_disarms() {
        let internal = SecretString::from("a".repeat(64));
        assert!(resolve_admin_read_token(None, &internal).is_none());
        assert!(resolve_admin_read_token(Some(&SecretString::from("")), &internal).is_none());
        assert!(resolve_admin_read_token(Some(&SecretString::from("   ")), &internal).is_none());
    }

    /// R8 — a copy-paste of the write secret voids the segregation: disarm.
    #[test]
    fn mika2360_admin_read_token_equal_to_internal_disarms() {
        let internal = SecretString::from("a".repeat(64));
        let same = SecretString::from("a".repeat(64));
        assert!(resolve_admin_read_token(Some(&same), &internal).is_none());
    }

    #[test]
    fn mika2360_admin_read_token_distinct_arms() {
        let internal = SecretString::from("a".repeat(64));
        let read = SecretString::from("read-only-secret");
        let resolved = resolve_admin_read_token(Some(&read), &internal).expect("armed");
        assert_eq!(resolved.expose_secret(), "read-only-secret");
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

    // -- MIKA_TELEGRAM_HTML_RENDER (mika#2291, C1–C5) --

    /// C1 — absent / empty ⇒ **armed**.
    ///
    /// This freezes the polarity, which is the inverse of both neighbours in this
    /// file (F8). A body copied from `orchestrator_inbox_is_enabled` returns `false`
    /// here and makes this test red — which is the whole reason it exists as its own
    /// assertion rather than as a line in a shared table.
    #[test]
    fn mika2291_c1_html_render_default_is_armed() {
        assert!(telegram_html_render_is_enabled(None));
        assert!(telegram_html_render_is_enabled(Some("")));
        assert!(telegram_html_render_is_enabled(Some("   ")));
    }

    /// C2 — the explicit disarming vocabulary, whitespace tolerated.
    #[test]
    fn mika2291_c2_explicit_values_disarm() {
        for raw in [
            "0",
            "false",
            "FALSE",
            "False",
            "off",
            "OFF",
            "no",
            "NO",
            " 0 ",
            "\tfalse\n",
        ] {
            assert!(
                !telegram_html_render_is_enabled(Some(raw)),
                "{raw:?} must disarm"
            );
        }
        for raw in ["1", "true", "TRUE", "on", "yes", " 1 ", "\ttrue\n"] {
            assert!(
                telegram_html_render_is_enabled(Some(raw)),
                "{raw:?} must arm"
            );
        }
    }

    /// C3 — an unrecognized value leans toward the **armed** default.
    ///
    /// A typo must not silently switch the rendering off. (The WARN naming the value
    /// between quotes is a side effect this assertion cannot observe; the quoting
    /// itself matters because a stray space is otherwise invisible — mika#2220.)
    #[test]
    fn mika2291_c3_unrecognized_value_stays_armed() {
        for raw in ["plif", "2", "maybe", "0x0", "-1"] {
            assert!(
                telegram_html_render_is_enabled(Some(raw)),
                "{raw:?} must stay armed"
            );
        }
    }

    /// C5 — **`GatewaySettings::load` cannot fail on this field, whatever is set.**
    ///
    /// This is the property `Option<String>` buys and a `bool` would lose, and it is
    /// invisible to C1–C4: those test the *parse function*, not the
    /// *deserialization*. Without this test the regression `Option<String>` → `bool`
    /// would pass review — it would make no output wrong, it would make the gateway
    /// **unbootable** on a typo, and C1–C4 would all stay green.
    ///
    /// The second half is the anti-vacuity control: it proves config-rs really does
    /// hard-fail a `bool`, so the first half is testing something.
    #[test]
    fn mika2291_c5_load_cannot_fail_on_this_field() {
        let built = Config::builder()
            .set_override("database_url", "postgres://localhost/test")
            .and_then(|b| b.set_override("internal_token", "a".repeat(64)))
            .and_then(|b| b.set_override("telegram_html_render", "plif"))
            .and_then(|b| b.build());
        let settings: GatewaySettings = built
            .expect("config builds")
            .try_deserialize()
            .expect("an arbitrary MIKA_TELEGRAM_HTML_RENDER must not fail deserialization");
        assert_eq!(settings.telegram_html_render.as_deref(), Some("plif"));
        // …and it resolves to the armed default rather than taking the gateway down.
        assert!(telegram_html_render_is_enabled(
            settings.telegram_html_render.as_deref()
        ));

        #[derive(Deserialize)]
        struct BoolFlagProbe {
            #[allow(dead_code)]
            flag: bool,
        }
        let probe = Config::builder()
            .set_override("flag", "plif")
            .and_then(|b| b.build())
            .expect("config builds")
            .try_deserialize::<BoolFlagProbe>();
        assert!(
            probe.is_err(),
            "config-rs no longer hard-fails a bool field — the F8 rationale for \
             Option<String> must be re-examined before relying on it"
        );
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

    // -- validation contract tests (mika#1590) --

    /// Helper to build a GatewaySettings with sensible defaults for validation tests.
    fn test_settings() -> GatewaySettings {
        GatewaySettings {
            database_url: SecretString::from("postgres://localhost/test"),
            telegram_bot_token: None,
            telegram_webhook_secret: None,
            telegram_webhook_url: None,
            telegram_single_bot_mode: None,
            telegram_html_render: None,
            internal_token: SecretString::from("a".repeat(64)),
            gateway_port: 8080,
            log_level: "info".to_string(),
            log_format: "json".to_string(),
            agent_base_url: None,
            gateway_log_file: None,
            agents_namespace: "mika-agents".to_string(),
            github_webhook_secret: None,
            github_app_id: None,
            github_app_private_key: None,
            github_app_installation_id: None,
            orchestrator_inbox_enabled: None,
            gateway_external_url: None,
            cm_api_url: None,
            search_upstream: None,
            brave_api_key: None,
            brave_endpoint: None,
            gateway_admin_read_token: None,
        }
    }

    // -- egress-search validation contract tests (mika#1807) --

    #[test]
    fn test_validate_search_upstream_absent_succeeds() {
        // No egress-search config at all — valid (endpoint returns 404).
        let s = test_settings();
        assert!(s.validate().is_ok());
    }

    #[test]
    fn test_validate_search_upstream_brave_requires_api_key() {
        let mut s = test_settings();
        s.search_upstream = Some("brave".to_string());
        let err = s.validate().unwrap_err();
        assert!(
            err.to_string().contains("MIKA_BRAVE_API_KEY"),
            "expected error about missing brave api key, got: {err}"
        );
    }

    #[test]
    fn test_validate_search_upstream_brave_with_key_succeeds() {
        let mut s = test_settings();
        s.search_upstream = Some("brave".to_string());
        s.brave_api_key = Some(SecretString::from("k"));
        assert!(s.validate().is_ok());
    }

    #[test]
    fn test_validate_search_upstream_rejects_unknown() {
        let mut s = test_settings();
        s.search_upstream = Some("google".to_string());
        let err = s.validate().unwrap_err();
        assert!(
            err.to_string().contains("unrecognized"),
            "expected error about unknown upstream, got: {err}"
        );
    }

    #[test]
    fn test_validate_search_upstream_case_insensitive() {
        let mut s = test_settings();
        s.search_upstream = Some("BRAVE".to_string());
        s.brave_api_key = Some(SecretString::from("k"));
        assert!(s.validate().is_ok());
    }

    #[test]
    fn test_validate_token_only_mode_off_succeeds() {
        // AC7: bot token alone is sufficient when single-bot mode is off
        let mut s = test_settings();
        s.telegram_bot_token = Some(SecretString::from("123:ABC"));
        assert!(s.validate().is_ok());
    }

    #[test]
    fn test_validate_token_only_mode_on_fails() {
        // AC7: token-only with single-bot mode on → validation error
        let mut s = test_settings();
        s.telegram_bot_token = Some(SecretString::from("123:ABC"));
        s.telegram_single_bot_mode = Some("1".to_string());
        let err = s.validate().unwrap_err();
        assert!(
            err.to_string().contains("MIKA_TELEGRAM_WEBHOOK_SECRET"),
            "expected error about missing webhook secret, got: {err}"
        );
    }

    #[test]
    fn test_validate_all_three_mode_on_succeeds() {
        // AC7: all three vars + mode on → success
        let mut s = test_settings();
        s.telegram_bot_token = Some(SecretString::from("123:ABC"));
        s.telegram_webhook_secret = Some(SecretString::from("b".repeat(64)));
        s.telegram_webhook_url = Some("https://example.com/webhook".to_string());
        s.telegram_single_bot_mode = Some("1".to_string());
        assert!(s.validate().is_ok());
    }

    #[test]
    fn test_validate_no_token_no_mode_succeeds() {
        // No Telegram config at all — valid (Telegram features simply disabled)
        let s = test_settings();
        assert!(s.validate().is_ok());
    }
}
