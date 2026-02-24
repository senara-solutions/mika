use anyhow::{Context, Result};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::startup;
use mika_common::claude::ClaudeClient;
use mika_common::config::Settings;
use mika_common::home;
use std::path::{Path, PathBuf};

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
}

impl Drop for DbContext {
    fn drop(&mut self) {
        self.async_db.shutdown();
    }
}

/// Shared initialization: resolve home, ensure initialized, load settings, open DB.
fn init_base() -> Result<(Settings, AsyncDatabase, PathBuf)> {
    let home_dir = home::resolve_home_dir()?;
    ensure_initialized(&home_dir)?;

    let settings =
        Settings::load(&home_dir).context("Failed to load config (run `mika setup` first).")?;

    let db = open_db(&settings)?;
    startup::seed_core_memory_if_empty(&db, &home_dir)?;
    let async_db = AsyncDatabase::new(db);

    Ok((settings, async_db, home_dir))
}

/// Initialize full context (for chat).
pub fn init() -> Result<AppContext> {
    let db_ctx = init_db_only()?;

    let claude = ClaudeClient::new(
        db_ctx.settings.anthropic_api_key.clone(),
        db_ctx.settings.claude_model.clone(),
        db_ctx.settings.claude_max_tokens,
    )?;

    Ok(AppContext { db_ctx, claude })
}

/// Initialize database-only context (for memory, reminders, status, config).
pub fn init_db_only() -> Result<DbContext> {
    let (settings, async_db, home_dir) = init_base()?;
    Ok(DbContext {
        settings,
        async_db,
        home_dir,
    })
}

fn ensure_initialized(home_dir: &Path) -> Result<()> {
    if !home::is_initialized(home_dir) {
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
