use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let home_dir = mika_common::home::resolve_home_dir()?;

    if !mika_common::home::is_initialized(&home_dir) {
        mika_common::home::bootstrap_fresh_install(&home_dir)?;
    }

    let settings = mika_common::config::Settings::load(&home_dir)
        .context("Failed to load config. Set MIKA_ANTHROPIC_API_KEY (API key or OAuth token) and MIKA_INTERNAL_TOKEN.")?;

    // Server mode uses structured JSON logging (+ optional file output)
    let _log_guard =
        mika_common::logging::init(&settings.log_level, settings.server_log_file.as_deref());

    mika_agent::server::run_server(&settings).await
}
