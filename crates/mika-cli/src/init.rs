use anyhow::{Context, Result};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::startup;
use mika_common::claude::ClaudeClient;
use mika_common::config::Settings;
use mika_common::home;
use std::path::PathBuf;

/// Full application context for commands that need the Claude client.
pub struct AppContext {
    pub settings: Settings,
    pub async_db: AsyncDatabase,
    pub claude: ClaudeClient,
    pub home_dir: PathBuf,
}

/// Lightweight context for commands that only need the database.
pub struct DbContext {
    pub settings: Settings,
    pub async_db: AsyncDatabase,
    pub home_dir: PathBuf,
}

/// Initialize full context (for chat).
pub fn init() -> Result<AppContext> {
    let home_dir = home::resolve_home_dir()?;
    ensure_initialized(&home_dir)?;

    let settings = Settings::load(&home_dir)
        .context("Failed to load config. Set MIKA_ANTHROPIC_API_KEY env var.")?;

    let db = open_db(&settings)?;
    startup::seed_core_memory_if_empty(&db, &home_dir)?;
    let async_db = AsyncDatabase::new(db);

    let claude = ClaudeClient::new(
        settings.anthropic_api_key.clone(),
        settings.claude_model.clone(),
        settings.claude_max_tokens,
    );

    Ok(AppContext {
        settings,
        async_db,
        claude,
        home_dir,
    })
}

/// Initialize database-only context (for memory, reminders, status, config).
pub fn init_db_only() -> Result<DbContext> {
    let home_dir = home::resolve_home_dir()?;
    ensure_initialized(&home_dir)?;

    let settings = Settings::load(&home_dir)
        .context("Failed to load config. Set MIKA_ANTHROPIC_API_KEY env var.")?;

    let db = open_db(&settings)?;
    startup::seed_core_memory_if_empty(&db, &home_dir)?;
    let async_db = AsyncDatabase::new(db);

    Ok(DbContext {
        settings,
        async_db,
        home_dir,
    })
}

fn ensure_initialized(home_dir: &PathBuf) -> Result<()> {
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
