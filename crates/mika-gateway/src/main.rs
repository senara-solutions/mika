pub mod openapi;
mod routes;
mod settings;
mod telegram;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

    // Initialize tracing (JSON structured logging for production, + optional file output)
    let _log_guard = mika_common::logging::init(
        &settings.log_level,
        settings.gateway_log_file.as_deref().map(Path::new),
        None::<mika_common::logging::NoopLayer>,
    );
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
    };

    let app = build_router(state);

    // Bind listener
    let port = settings.gateway_port;
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;

    // Mark ready — health endpoint starts returning 200
    ready.store(true, Ordering::Release);
    info!(port, "mika-gateway listening, ready");

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
