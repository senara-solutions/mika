mod a2a_auth;
mod a2a_routes;
pub(crate) mod dlq;
pub mod github;
pub mod openapi;
pub(crate) mod orchestrator_inbox;
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
    // Install rustls aws-lc-rs CryptoProvider before any TLS operation. Both
    // aws-lc-rs and ring are compiled in transitively (sqlx via tls-rustls-aws-lc-rs
    // and reqwest via rustls-tls-native-roots), so rustls cannot auto-select a
    // process-default crypto provider. Mirrors the mika-cloud console fix
    // (mika-cloud#97). Resolves "TLS upgrade required by connect options but
    // SQLx was built without TLS support enabled" when connecting to RDS with
    // force_ssl=1.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls aws-lc-rs CryptoProvider");

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

    // Create Telegram client (only in single-bot mode)
    let single_bot_mode =
        settings::telegram_single_bot_mode_is_enabled(settings.telegram_single_bot_mode.as_deref());
    let telegram = if single_bot_mode {
        let bot_token = settings
            .telegram_bot_token
            .clone()
            .expect("validated in GatewaySettings::load");
        let tg = TelegramClient::new(http_client.clone(), bot_token);

        // Register webhook with Telegram (idempotent)
        let webhook_url = settings
            .telegram_webhook_url
            .as_ref()
            .expect("validated in GatewaySettings::load");
        tg.set_webhook(
            webhook_url,
            settings
                .telegram_webhook_secret
                .as_ref()
                .expect("validated in GatewaySettings::load")
                .expose_secret(),
        )
        .await?;
        info!(url = %webhook_url, "telegram webhook registered (single-bot mode)");

        Some(tg)
    } else {
        info!("per-customer Telegram bot mode — global webhook registration skipped");
        None
    };

    // Log GitHub webhook configuration status
    if settings.github_webhook_secret.is_some() {
        info!("GitHub webhook endpoint enabled (MIKA_GITHUB_WEBHOOK_SECRET configured)");
    } else {
        info!("GitHub webhook endpoint disabled (MIKA_GITHUB_WEBHOOK_SECRET not set)");
    }

    // Construct GitHub App for outbound API calls (synchronize no-diff guard #886).
    // Returns None when credentials are incomplete — fail-open, all synchronize events pass through.
    let github_app = match (
        settings.github_app_id,
        settings.github_app_private_key.as_ref(),
        settings.github_app_installation_id,
    ) {
        (Some(app_id), Some(pk), Some(install_id)) => {
            mika_common::github_app::GitHubApp::from_credentials(
                app_id,
                pk.expose_secret(),
                install_id,
            )
        }
        _ => {
            info!("GitHub App not configured for gateway (synchronize no-diff guard disabled)");
            None
        }
    };

    let raw_orchestrator_inbox_flag = settings.orchestrator_inbox_enabled.as_deref();
    let orchestrator_inbox_enabled =
        settings::orchestrator_inbox_is_enabled(raw_orchestrator_inbox_flag);
    if orchestrator_inbox_enabled {
        info!("orchestrator inbox endpoints enabled (MIKA_ORCHESTRATOR_INBOX_ENABLED=1)");
    } else {
        info!(
            raw_value = ?raw_orchestrator_inbox_flag,
            "orchestrator inbox endpoints disabled (MIKA_ORCHESTRATOR_INBOX_ENABLED unset, empty, '0', '2', or unrecognized; only '1' or 'true' enables)"
        );
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
        github_delivery_cache: github::new_delivery_cache(),
        github_app,
        github_api_base_url: None,
        orchestrator_inbox_enabled,
        inbox_subscriber_semaphore: orchestrator_inbox::default_inbox_subscriber_semaphore(),
    };

    // Spawn DLQ background worker (retries pending deliveries every 30s)
    tokio::spawn(dlq::run_dlq_worker(state.clone()));

    // Spawn orchestrator inbox retention task (mika#1189) — purges rows older
    // than `ORCHESTRATOR_INBOX_RETENTION_DAYS` once an hour. Gated on the
    // feature flag so a gateway running without the migration applied
    // doesn't error-log a DELETE against a non-existent table every hour.
    if orchestrator_inbox_enabled {
        tokio::spawn(orchestrator_inbox::run_retention_task(state.pool.clone()));
    }

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
