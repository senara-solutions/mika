use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let home_dir = mika_common::home::resolve_home_dir()?;

    // mika#1968 AC1/AC2 — pre-logging::init boot line surfacing the resolved
    // home dir and the env vars that drove the resolution. Uses eprintln!
    // (not tracing) because the JSON subscriber isn't installed yet — stderr
    // is captured by OpenRC/systemd into the service log where an operator
    // can grep for `mika_spirit_home_resolved` without needing to run
    // subprocess-inspection diagnostics against a live `mika-arch` process.
    // See docs/solutions/best-practices/app-owned-env-vs-init-owned-env-2026-08-23.md.
    let mika_home_env = std::env::var("MIKA_HOME").unwrap_or_else(|_| "<unset>".to_string());
    let home_env = std::env::var("HOME").unwrap_or_else(|_| "<unset>".to_string());
    eprintln!(
        "mika_spirit_home_resolved home={} mika_home_env={} home_env={}",
        home_dir.display(),
        mika_home_env,
        home_env
    );

    mika_common::dotenv::load_dotenv(&home_dir);
    mika_common::dotenv::check_env_warnings(&home_dir);

    // mika#1968 AC2 — post-load-dotenv boot line summarizing what landed on
    // disk vs what the process env now sees for the load-bearing vars. Same
    // eprintln! reasoning as home_resolved above.
    let env_file_key_count = mika_common::dotenv::parse_dotenv(&home_dir).len();
    let manager_target_set = std::env::var("MIKA_MANAGER_TARGET_MILESTONE").is_ok();
    let github_token_set = std::env::var("MIKA_GITHUB_TOKEN").is_ok();
    eprintln!(
        "mika_spirit_env_check env_file_keys={} manager_target_set={} github_token_set={}",
        env_file_key_count, manager_target_set, github_token_set
    );

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
        settings.spirit_log_file.as_deref(),
        log_format,
        otel_layer,
        settings.log_llm_bodies,
    );

    // Install tracing-aware panic hook (defense-in-depth for spawned tasks
    // that panic without their JoinHandle being awaited — see mika#765)
    mika_agent::panic_hook::install_tracing_panic_hook();

    mika_agent::server::run_server(&settings).await
}
