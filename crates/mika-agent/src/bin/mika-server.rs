use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // Register sqlite-vec extension before any DB connections are opened.
    mika_agent::db::init_sqlite_vec();

    let home_dir = mika_common::home::resolve_home_dir()?;

    if !mika_common::home::is_initialized(&home_dir) {
        // Bootstrap as multi-agent layout from the start
        std::fs::create_dir_all(home_dir.join("agents"))
            .with_context(|| format!("failed to create {}/agents/", home_dir.display()))?;
        mika_common::home::bootstrap_agent(&home_dir, mika_common::agent::DEFAULT_AGENT)
            .with_context(|| format!("failed to initialize default agent"))?;
        mika_common::home::write_active_agent(&home_dir, mika_common::agent::DEFAULT_AGENT)?;
        mika_common::home::write_default_if_missing_pub(
            &home_dir,
            "config.toml",
            mika_common::home::DEFAULT_GLOBAL_CONFIG,
        )?;
    }

    let settings = mika_common::config::Settings::load(&home_dir)
        .context("Failed to load config. Set MIKA_ANTHROPIC_API_KEY and MIKA_INTERNAL_TOKEN.")?;

    // Server mode uses structured JSON logging
    mika_common::logging::init(&settings.log_level);

    mika_agent::server::run_server(&settings).await
}
