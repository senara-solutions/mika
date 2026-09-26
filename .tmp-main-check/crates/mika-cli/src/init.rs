use anyhow::{Context, Result};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::messaging::{GatewayMessageSender, MessageSender};
use mika_agent::startup;
use mika_common::config::Settings;
use mika_common::github_app::GitHubApp;
use mika_common::home;
use mika_common::llm::LlmProvider;
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
    ///
    /// Since mika#2304 the resolution itself lives in
    /// [`mika_common::llm::model_override`], reachable by `mika-agent` too —
    /// `mika ask` no longer executes the turn (mika#1727), so the server must be
    /// able to resolve the same string the same way. This method is now the
    /// **in-process** consumer only: `mika chat`, whose turn really does run
    /// against `self.llm`.
    pub fn override_model(&mut self, model: &str) -> Result<()> {
        let (provider, model_id) =
            mika_common::llm::model_override::resolve_model_override(&self.db_ctx.settings, model)?;
        self.db_ctx
            .settings
            .set_provider_model(provider, Some(model_id));
        self.llm = self.db_ctx.settings.make_llm_provider()?;
        Ok(())
    }
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
///
/// Reads from the operator-shell scoped MCP config path (mika#1737 AC3).
/// Runs the AC5 one-shot migration from `{agent_home}/mcp.json` if the
/// operator-shell path does not yet exist. `agent_home` is retained only
/// as the migration source; runtime MCP connections themselves are
/// operator-shell scoped.
pub async fn connect_mcp(agent_home: &Path) -> Option<mika_agent::mcp::McpManager> {
    if let Err(e) =
        mika_agent::mcp::config::McpConfig::migrate_from_agent_home_if_needed(agent_home)
    {
        tracing::warn!(error = %e, "mika#1737 AC5 MCP migration failed on connect_mcp");
    }
    let config = match mika_agent::mcp::config::McpConfig::load_operator_shell() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load operator-shell MCP config, skipping");
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
        settings.customer_id.clone(),
    );
    Some(Arc::new(sender))
}

#[cfg(test)]
mod tests {
    // The behavioural tests for alias resolution, prefix stripping and the
    // API-key check moved with their functions to
    // `mika_common::llm::model_override` (mika#2304). They run there unchanged,
    // which is what attests the move is a move and not a rewrite — `mika chat`
    // still gets the same answers (T8).
    //
    // What stays here is the guard a behavioural test cannot provide.

    /// **T7 — `mika-cli` keeps no second resolver (mika#2304, D5).**
    ///
    /// A behavioural test cannot see this class of regression. A re-added local
    /// copy of the alias table or of the prefix-strip rule would make no tested
    /// decision wrong: it would simply let `mika chat` and the A2A turn answer
    /// the same question differently, with only one of the two paths covered.
    /// That is the shape mika#2158 had to engrave after a grooming regex was
    /// copied and then missed two widenings for months — nothing broke, the two
    /// readers just stopped agreeing.
    ///
    /// The allowlist is empty on purpose: there is no legitimate reason for this
    /// crate to define either. Re-exporting from `mika_common` (`pub use`) is
    /// not a definition and is how the four existing call sites still read
    /// `crate::cli::MODEL_ALIASES`.
    #[test]
    fn mika2304_the_cli_keeps_no_second_resolver() {
        // (needle, what a hit would mean). Each needle is assembled with
        // `concat!` so this file does not contain the strings it forbids —
        // otherwise the guard would flag itself and could only be silenced by
        // an allowlist, which is exactly what it must not have.
        const FORBIDDEN: &[(&str, &str)] = &[
            (
                concat!("MODEL_ALIASES", ": &["),
                "a second alias table — `mika_common::llm::model_override::MODEL_ALIASES` is the one",
            ),
            (
                concat!("fn ", "resolve_model_alias"),
                "a second alias resolver — re-export `mika_common::llm::model_override::resolve_model_alias`",
            ),
            (
                concat!("fn ", "parse_model_override"),
                "a second prefix-strip rule — call `mika_common::llm::model_override::parse_model_override`",
            ),
            (
                concat!("fn ", "check_provider_key"),
                "a second API-key check — call `mika_common::llm::model_override::check_provider_key`",
            ),
            (
                concat!("fn ", "provider_requires_api_key"),
                "a second key-requirement table — call `mika_common::llm::model_override::provider_requires_api_key`",
            ),
        ];

        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offences = Vec::new();
        visit_rust_files(&src, &mut |path, body| {
            for (needle, why) in FORBIDDEN {
                if body.contains(needle) {
                    offences.push(format!("{}: {needle} — {why}", path.display()));
                }
            }
        });

        assert!(
            offences.is_empty(),
            "mika#2304 D5: the resolution of a `--model` override lives at exactly one \
             site, in `mika-common`, because `mika-agent` must reach the same one. \
             Found:\n  {}",
            offences.join("\n  ")
        );
    }

    /// Good-faith control for the scan above: it must actually read files, or
    /// the guard would pass on an empty walk and attest nothing.
    #[test]
    fn mika2304_the_resolver_scan_reads_the_crate() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut seen = 0usize;
        let mut found_this_file = false;
        visit_rust_files(&src, &mut |path, body| {
            seen += 1;
            if path.ends_with("init.rs")
                && body.contains("mika2304_the_cli_keeps_no_second_resolver")
            {
                found_this_file = true;
            }
        });
        assert!(seen > 10, "the scan only read {seen} files");
        assert!(found_this_file, "the scan did not read init.rs itself");
    }

    fn visit_rust_files(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path, &str)) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit_rust_files(&path, f);
            } else if path.extension().is_some_and(|e| e == "rs")
                && let Ok(body) = std::fs::read_to_string(&path)
            {
                f(&path, &body);
            }
        }
    }
}
