mod a2a_auth;
mod a2a_routes;
pub mod github;
pub mod openapi;
mod routes;
mod settings;
mod telegram;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use std::path::Path;

use anyhow::Result;
use secrecy::ExposeSecret;
use tokio::net::TcpListener;
use tracing::info;

use routes::{AppState, build_router};
use settings::GatewaySettings;
use telegram::TelegramClient;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env from CWD (gateway has no ~/.mika/ home directory)
    let _ = dotenvy::dotenv();

    let settings = GatewaySettings::load()?;

    // Initialize tracing (structured logging, + optional file output)
    let log_format: mika_common::logging::LogFormat = settings
        .log_format
        .parse()
        .map_err(|e: String| anyhow::anyhow!(e))?;
    let is_pretty = log_format == mika_common::logging::LogFormat::Pretty;
    let _log_guard = mika_common::logging::init(
        &settings.log_level,
        settings.gateway_log_file.as_deref().map(Path::new),
        log_format,
        None::<mika_common::logging::NoopLayer>,
        false, // log_llm_bodies: gateway doesn't make LLM calls
    );

    if is_pretty {
        mika_common::logging::print_banner("mika-gateway", env!("CARGO_PKG_VERSION"));
    }

    info!(settings = ?settings, "starting mika-gateway");

    let ready = Arc::new(AtomicBool::new(false));

    // Connect to Postgres
    let pool = sqlx::postgres::PgPoolOptions::new()
        .min_connections(2)
        .max_connections(20)
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect(settings.database_url.expose_secret())
        .await?;

    info!("postgres connected");

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;
    info!("migrations applied");

    // Create shared HTTP client
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(2))
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .build()?;

    // Create Telegram client
    let telegram = TelegramClient::new(http_client.clone(), settings.telegram_bot_token.clone());

    // Register webhook with Telegram (idempotent)
    telegram
        .set_webhook(
            &settings.telegram_webhook_url,
            settings.telegram_webhook_secret.expose_secret(),
        )
        .await?;
    info!(url = %settings.telegram_webhook_url, "telegram webhook registered");

    // Log GitHub webhook configuration status
    if settings.github_webhook_secret.is_some() {
        info!("GitHub webhook endpoint enabled (MIKA_GITHUB_WEBHOOK_SECRET configured)");
        if settings.github_app_id.is_some() {
            info!("GitHub bot self-event filtering enabled (MIKA_GITHUB_APP_ID configured)");
        } else {
            info!("GitHub bot self-event filtering disabled (MIKA_GITHUB_APP_ID not set)");
        }
    } else {
        info!("GitHub webhook endpoint disabled (MIKA_GITHUB_WEBHOOK_SECRET not set)");
    }

    // Build app state
    let state = AppState {
        pool,
        telegram,
        http_client,
        internal_token: settings.internal_token.clone(),
        webhook_secret: settings.telegram_webhook_secret.clone(),
        ready: ready.clone(),
        // Capacity budget: 30 permits * 2 queries/task = 60 peak connection acquisitions.
        // Pool of 20 connections provides sufficient headroom.
        webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(30)),
        agent_base_url: settings.agent_base_url.clone(),
        agents_namespace: settings.agents_namespace.clone(),
        webhook_counter: Arc::new(AtomicU64::new(0)),
        github_webhook_secret: settings.github_webhook_secret.clone(),
        github_app_id: settings.github_app_id,
        github_delivery_cache: github::new_delivery_cache(),
    };

    let app = build_router(state);

    // Bind listener
    let port = settings.gateway_port;
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;

    // Mark ready — health endpoint starts returning 200
    ready.store(true, Ordering::Release);
    info!(port, "mika-gateway listening, ready");

    if is_pretty {
        mika_common::logging::print_ready();
    }

    // Serve with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("mika-gateway shut down cleanly");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to register SIGTERM handler");

    tokio::select! {
        _ = ctrl_c => info!("received Ctrl-C, shutting down..."),
        _ = sigterm.recv() => info!("received SIGTERM, shutting down..."),
    }
}
