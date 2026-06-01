use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let home_dir = mika_common::home::resolve_home_dir()?;
    mika_common::dotenv::load_dotenv(&home_dir);
    mika_common::dotenv::check_env_warnings(&home_dir);

    if !mika_common::home::is_initialized(&home_dir) {
        mika_common::home::bootstrap_fresh_install(&home_dir)?;
    }

    let settings = mika_common::config::Settings::load(&home_dir)
        .context("Failed to load config. Set MIKA_ANTHROPIC_API_KEY (or your provider's key) and MIKA_INTERNAL_TOKEN.")?;

    // Build optional OTel export layer (feature-gated, graceful degradation)
    let (otel_layer, _telemetry_guard) = mika_common::telemetry::try_init_otel(&settings);

    // Server mode uses structured logging (+ optional file output + optional OTel export)
    let log_format: mika_common::logging::LogFormat = settings
        .log_format
        .parse()
        .map_err(|e: String| anyhow::anyhow!(e))?;
    let _log_guard = mika_common::logging::init(
        &settings.log_level,
        settings.server_log_file.as_deref(),
        log_format,
        otel_layer,
        settings.log_llm_bodies,
    );

    // Install tracing-aware panic hook (defense-in-depth for spawned tasks
    // that panic without their JoinHandle being awaited — see mika#765)
    mika_agent::panic_hook::install_tracing_panic_hook();

    mika_agent::server::run_server(&settings).await
}
