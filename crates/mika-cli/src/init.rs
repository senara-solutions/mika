use anyhow::{Context, Result};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::messaging::{GatewayMessageSender, MessageSender};
use mika_agent::startup;
use mika_common::config::Settings;
use mika_common::github_app::GitHubApp;
use mika_common::home;
use mika_common::llm::{LlmProvider, ProviderKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Full application context for commands that need an LLM provider.
/// Dropping this shuts down the async database automatically.
pub struct AppContext {
    pub db_ctx: DbContext,
    pub llm: Arc<dyn LlmProvider>,
}

// Deref so callers can still use ctx.settings, ctx.async_db, ctx.home_dir.
impl std::ops::Deref for AppContext {
    type Target = DbContext;
    fn deref(&self) -> &DbContext {
        &self.db_ctx
    }
}

/// Lightweight context for commands that only need the database.
/// Dropping this shuts down the async database automatically.
pub struct DbContext {
    pub settings: Settings,
    pub async_db: AsyncDatabase,
    pub home_dir: PathBuf,
    /// The global Mika home directory (e.g. ~/.mika/).
    /// In multi-agent mode this differs from `home_dir` (which is the agent's dir).
    pub global_home: PathBuf,
    /// GitHub App authentication manager (optional).
    pub github_app: Option<Arc<GitHubApp>>,
}

impl Drop for DbContext {
    fn drop(&mut self) {
        self.async_db.shutdown();
    }
}

/// Shared initialization for an agent: migrate, resolve agent home, load config, open DB.
fn init_base_for_agent(agent_name: &str) -> Result<(Settings, AsyncDatabase, PathBuf, PathBuf)> {
    let global_home = home::resolve_home_dir()?;

    // Auto-migrate legacy layout to multi-agent on every startup
    home::migrate_to_multi_agent(&global_home)?;

    let agent_home = home::resolve_agent_home(&global_home, agent_name);
    ensure_initialized_for_agent(&global_home, &agent_home, agent_name)?;

    let settings = Settings::load_for_agent(&global_home, &agent_home)
        .context("Failed to load config (run `mika setup` first).")?;

    let mut db = open_db(&settings)?;
    let identity = mika_agent::prompt::load_identity(&agent_home);
    db.register_agent(
        agent_name,
        &identity.name,
        agent_home.to_str().unwrap_or(""),
    )?;
    startup::seed_core_memory_if_empty(&db, &agent_home, agent_name)?;
    startup::seed_bundled_skills_if_needed(&agent_home, settings.disable_bundled_skills);
    if settings.dev_mode {
        mika_agent::well_known_agents::seed_well_known_skill_overrides(&mut db, agent_name);
    }
    let async_db = AsyncDatabase::new_with_agent(db, agent_name);

    Ok((settings, async_db, agent_home, global_home))
}

impl AppContext {
    /// Apply a one-shot model override (not persisted to config).
    ///
    /// The `--model` flag is a **model-id-only** override: the provider is always
    /// inherited from the agent's configured `llm_provider`. The model id is never
    /// used to re-dispatch to a different (native) provider based on its name prefix
    /// (mika#1591) — that prefix-routing produced spurious HTTP 401 "no API key"
    /// failures when the inferred native provider had no key. A `prefix/` is stripped
    /// only when it names the configured provider itself (e.g. `qwen/qwen3.7-max`
    /// under `llm_provider = "qwen"`); under any other provider (e.g. OpenRouter,
    /// whose ids are vendor-prefixed) the full id is preserved.
    ///
    /// Aliases (e.g. "sonnet") are resolved before routing.
    pub fn override_model(&mut self, model: &str) -> Result<()> {
        let configured = self.db_ctx.settings.llm_provider;
        let (provider, model_id) = parse_model_override(model, configured);

        // AC2 (mika#1591): surface a named error instead of a bare downstream
        // 401 "no API key" when the inherited provider needs a key but none is
        // configured. Local providers (Ollama, MikaModel) are exempt.
        let (_, api_key, _) = self.db_ctx.settings.provider_fields(provider);
        check_provider_key(provider, api_key, &model_id)?;

        self.db_ctx
            .settings
            .set_provider_model(provider, Some(model_id));
        self.llm = self.db_ctx.settings.make_llm_provider()?;
        Ok(())
    }
}

/// Resolve a `--model` override into `(provider, model_id)` for the agent's
/// configured provider.
///
/// The returned provider is **always** `configured` — the model name's prefix
/// never re-dispatches to a different provider (mika#1591). Aliases are resolved
/// first. A `prefix/rest` model id has its prefix stripped only when `prefix`
/// parses to the configured provider itself; otherwise the full id is preserved
/// (OpenRouter and other vendor-prefixed providers need the full id).
fn parse_model_override(model: &str, configured: ProviderKind) -> (ProviderKind, String) {
    let resolved = crate::cli::resolve_model_alias(model);
    if let Some((prefix, rest)) = resolved.split_once('/')
        && let Ok(parsed) = prefix.parse::<ProviderKind>()
        && parsed == configured
    {
        return (configured, rest.to_string());
    }
    (configured, resolved)
}

/// Whether a provider needs an API key to authenticate. Local providers
/// (Ollama, MikaModel — localhost endpoints) do not.
fn provider_requires_api_key(provider: ProviderKind) -> bool {
    !matches!(provider, ProviderKind::Ollama | ProviderKind::MikaModel)
}

/// Validate that a key-requiring provider has an API key configured before a
/// `--model` override routes a request to it. Returns a named error (provider +
/// model id) instead of letting the request fail with a bare downstream 401
/// "no API key" (mika#1591 AC2).
fn check_provider_key(provider: ProviderKind, api_key: Option<&str>, model_id: &str) -> Result<()> {
    if provider_requires_api_key(provider) && api_key.is_none_or(|k| k.trim().is_empty()) {
        anyhow::bail!(
            "Provider '{provider}' has no API key configured. Cannot route model '{model_id}'."
        );
    }
    Ok(())
}

/// Initialize full context for a named agent (for chat).
pub fn init_for_agent(agent_name: &str) -> Result<AppContext> {
    let db_ctx = init_db_only_for_agent(agent_name)?;
    let llm = db_ctx.settings.make_llm_provider()?;
    Ok(AppContext { db_ctx, llm })
}

/// Initialize database-only context for a named agent.
pub fn init_db_only_for_agent(agent_name: &str) -> Result<DbContext> {
    let (settings, async_db, home_dir, global_home) = init_base_for_agent(agent_name)?;
    let github_app = GitHubApp::from_settings(&settings);
    Ok(DbContext {
        settings,
        async_db,
        home_dir,
        global_home,
        github_app,
    })
}

/// Resolve the active agent name from the home directory.
pub fn resolve_active_agent() -> Result<String> {
    let global_home = home::resolve_home_dir()?;
    Ok(home::read_active_agent(&global_home))
}

fn ensure_initialized_for_agent(
    global_home: &Path,
    agent_home: &Path,
    agent_name: &str,
) -> Result<()> {
    // In multi-agent layout, check the specific agent's home
    if home::is_multi_agent_layout(global_home) {
        if !agent_home.join("config.toml").exists() {
            // Auto-provision well-known agents if dev_mode is enabled
            if mika_agent::well_known_agents::find_well_known_agent(agent_name).is_some()
                && let Ok(global_settings) = Settings::load(global_home)
                && global_settings.dev_mode
            {
                mika_agent::well_known_agents::provision_well_known_agents(
                    global_home,
                    &global_settings,
                    global_settings.disable_agent_provisioning,
                );
            }
            // Re-check after provisioning attempt
            if !agent_home.join("config.toml").exists() {
                anyhow::bail!(
                    "Agent '{agent_name}' not found. Create it with `mika agents create {agent_name}`."
                );
            }
        }
    } else if !home::is_initialized(global_home) {
        anyhow::bail!(
            "Mika not initialized. Run `mika setup` first, or just run `mika` to auto-setup."
        );
    }
    Ok(())
}

fn open_db(settings: &Settings) -> Result<Database> {
    let db_path = &settings.db_path;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    Database::open(db_path).context("failed to open database")
}

/// Load MCP config and connect to all enabled servers.
/// Returns `None` if no servers are configured or all connections fail.
pub async fn connect_mcp(agent_home: &Path) -> Option<mika_agent::mcp::McpManager> {
    let config = match mika_agent::mcp::config::McpConfig::load(agent_home) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load MCP config, skipping");
            return None;
        }
    };
    if config.mcp_servers.is_empty() {
        return None;
    }
    let manager = mika_agent::mcp::McpManager::connect_all(&config).await;
    if manager.has_connections() {
        Some(manager)
    } else {
        None
    }
}

/// Build a `GatewayMessageSender` if both `routing_url` and `internal_token` are configured.
/// Returns `None` otherwise, preserving CLI-only behavior.
pub fn make_message_sender(
    settings: &Settings,
    db: &AsyncDatabase,
    http_client: &reqwest::Client,
    agent_name: &str,
) -> Option<Arc<dyn MessageSender>> {
    let url = settings.routing_url.as_deref()?;
    let token = settings.internal_token.clone()?;

    let parsed = match reqwest::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(e) => {
            tracing::warn!(error = %e, "invalid routing_url, skipping gateway message sender");
            return None;
        }
    };

    if !matches!(parsed.scheme(), "http" | "https") {
        tracing::warn!(
            scheme = parsed.scheme(),
            "routing_url must use http or https scheme"
        );
        return None;
    }

    let sender = GatewayMessageSender::new(
        url.to_string(),
        token,
        db.clone(),
        http_client.clone(),
        None,
        Some(agent_name.to_string()),
        None,
    );
    Some(Arc::new(sender))
}

#[cfg(test)]
mod tests {
    use super::*;

    // U1 / AC1: a vendor-prefixed id whose prefix does NOT name the configured
    // provider keeps the full id and inherits the configured provider (no
    // prefix re-dispatch). OpenRouter ids are vendor-prefixed.
    #[test]
    fn parse_keeps_full_id_for_openrouter() {
        let (provider, model) = parse_model_override("qwen/qwen3.7-max", ProviderKind::OpenRouter);
        assert_eq!(provider, ProviderKind::OpenRouter);
        assert_eq!(model, "qwen/qwen3.7-max");
    }

    // U1 / AC3: a vendor prefix that matches the configured provider is stripped.
    #[test]
    fn parse_strips_matching_prefix_for_native_qwen() {
        let (provider, model) = parse_model_override("qwen/qwen3.7-max", ProviderKind::Qwen);
        assert_eq!(provider, ProviderKind::Qwen);
        assert_eq!(model, "qwen3.7-max");
    }

    // U1 / AC4: a non-prefixed id is passed through unchanged under its provider.
    #[test]
    fn parse_passes_through_unprefixed_id() {
        let (provider, model) =
            parse_model_override("claude-sonnet-4-6-20250514", ProviderKind::Anthropic);
        assert_eq!(provider, ProviderKind::Anthropic);
        assert_eq!(model, "claude-sonnet-4-6-20250514");
    }

    // U1: aliases resolve before routing and still inherit the configured provider.
    // Under the alias's own native provider the resolved prefix is stripped; under
    // any other provider the full vendor-prefixed id is preserved.
    #[test]
    fn parse_resolves_alias_and_inherits_provider() {
        // "sonnet" resolves to "anthropic/claude-sonnet-4-6"; under Anthropic the
        // matching prefix is stripped to the native id.
        let (provider, model) = parse_model_override("sonnet", ProviderKind::Anthropic);
        assert_eq!(provider, ProviderKind::Anthropic);
        assert_eq!(model, "claude-sonnet-4-6");
        // Under a non-matching provider the resolved full id is kept (OpenRouter ids
        // are vendor-prefixed) and the provider is still inherited.
        let (provider, model) = parse_model_override("sonnet", ProviderKind::OpenRouter);
        assert_eq!(provider, ProviderKind::OpenRouter);
        assert_eq!(model, "anthropic/claude-sonnet-4-6");
    }

    // U1: a non-matching native prefix under a third provider never re-dispatches
    // — guards against regression of the old prefix-routing behavior.
    #[test]
    fn parse_never_redispatches_to_named_native_provider() {
        let (provider, model) = parse_model_override("qwen/qwen3.7-max", ProviderKind::DeepSeek);
        assert_eq!(provider, ProviderKind::DeepSeek);
        assert_eq!(model, "qwen/qwen3.7-max");
    }

    // U1: degenerate model strings never panic and inherit the configured provider.
    // A bare prefix or trailing/leading slash whose prefix is not a provider keeps
    // the full string; an empty string passes through unchanged.
    #[test]
    fn parse_handles_degenerate_strings() {
        assert_eq!(
            parse_model_override("", ProviderKind::Anthropic),
            (ProviderKind::Anthropic, String::new())
        );
        // "foo/" — prefix "foo" is not a provider → full string kept.
        assert_eq!(
            parse_model_override("foo/", ProviderKind::OpenRouter),
            (ProviderKind::OpenRouter, "foo/".to_string())
        );
        // "/bar" — empty prefix is not a provider → full string kept.
        assert_eq!(
            parse_model_override("/bar", ProviderKind::OpenRouter),
            (ProviderKind::OpenRouter, "/bar".to_string())
        );
    }

    // U2 / AC2: a key-requiring provider with no key yields a named error that
    // includes both the provider and the model id.
    #[test]
    fn check_key_errors_name_provider_and_model() {
        let err = check_provider_key(ProviderKind::OpenRouter, None, "qwen/qwen3.7-max")
            .unwrap_err()
            .to_string();
        assert!(err.contains("openrouter"), "error names provider: {err}");
        assert!(err.contains("qwen/qwen3.7-max"), "error names model: {err}");
    }

    // U2 / AC2: an empty/whitespace key is treated as absent.
    #[test]
    fn check_key_treats_blank_key_as_absent() {
        assert!(check_provider_key(ProviderKind::OpenRouter, Some("   "), "m").is_err());
    }

    // U2: a configured key passes the check.
    #[test]
    fn check_key_passes_with_configured_key() {
        assert!(check_provider_key(ProviderKind::OpenRouter, Some("sk-or-xxx"), "m").is_ok());
    }

    // U2: local providers (Ollama, MikaModel) are exempt from the key check.
    #[test]
    fn check_key_exempts_local_providers() {
        assert!(check_provider_key(ProviderKind::Ollama, None, "llama3").is_ok());
        assert!(check_provider_key(ProviderKind::MikaModel, None, "mika").is_ok());
    }
}
