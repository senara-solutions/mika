use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // Register sqlite-vec extension before any DB connections are opened.
    mika_agent::db::init_sqlite_vec();

    let home_dir = mika_common::home::resolve_home_dir()?;

    if !mika_common::home::is_initialized(&home_dir) {
        mika_common::home::bootstrap_fresh_install(&home_dir)?;
    }

    let settings = mika_common::config::Settings::load(&home_dir)
        .context("Failed to load config. Set MIKA_ANTHROPIC_API_KEY and MIKA_INTERNAL_TOKEN.")?;

    // Server mode uses structured JSON logging
    mika_common::logging::init(&settings.log_level);

    mika_agent::server::run_server(&settings).await
}
