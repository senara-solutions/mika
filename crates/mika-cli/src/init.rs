use anyhow::{Context, Result};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::messaging::{GatewayMessageSender, MessageSender};
use mika_agent::startup;
use mika_common::claude::ClaudeClient;
use mika_common::config::Settings;
use mika_common::home;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Full application context for commands that need the Claude client.
/// Dropping this shuts down the async database automatically.
pub struct AppContext {
    pub db_ctx: DbContext,
    pub claude: ClaudeClient,
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

    let db = open_db(&settings)?;
    let identity = mika_agent::prompt::load_identity(&agent_home);
    db.register_agent(
        agent_name,
        &identity.name,
        agent_home.to_str().unwrap_or(""),
    )?;
    startup::seed_core_memory_if_empty(&db, &agent_home, agent_name)?;
    startup::seed_bundled_skills_if_needed(&agent_home, settings.disable_bundled_skills);
    let async_db = AsyncDatabase::new_with_agent(db, agent_name);

    Ok((settings, async_db, agent_home, global_home))
}

/// Initialize full context for a named agent (for chat).
pub fn init_for_agent(agent_name: &str) -> Result<AppContext> {
    let db_ctx = init_db_only_for_agent(agent_name)?;

    let claude = ClaudeClient::new(
        db_ctx.settings.anthropic_api_key.clone(),
        db_ctx.settings.claude_model.clone(),
        db_ctx.settings.claude_max_tokens,
    )?;

    Ok(AppContext { db_ctx, claude })
}

/// Initialize database-only context for a named agent.
pub fn init_db_only_for_agent(agent_name: &str) -> Result<DbContext> {
    let (settings, async_db, home_dir, global_home) = init_base_for_agent(agent_name)?;
    Ok(DbContext {
        settings,
        async_db,
        home_dir,
        global_home,
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
            anyhow::bail!(
                "Agent '{agent_name}' not found. Create it with `mika agents create {agent_name}`."
            );
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
        None, // CLI doesn't send to Telegram gateway
    );
    Some(Arc::new(sender))
}
