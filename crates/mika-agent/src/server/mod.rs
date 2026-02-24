mod auth;
mod handlers;
pub mod state;
pub mod types;

use anyhow::{Result, anyhow};
use axum::{Router, middleware, routing::{get, post}};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpListener;
use tracing::info;

use mika_common::claude::ClaudeClient;
use mika_common::config::Settings;

use crate::async_db::AsyncDatabase;
use crate::db::Database;
use crate::scheduler::ReminderScheduler;
use crate::tools;

use state::AppState;

/// Start the Mika HTTP server.
///
/// Initializes the database, Claude client, tool registry, and scheduler,
/// then binds to the configured port and serves until SIGTERM/Ctrl-C.
pub async fn run_server(settings: &Settings) -> Result<()> {
    let home_dir = &settings.home_dir;

    // Open and wrap database
    let db = Database::open(&settings.db_path)?;

    // Seed core memory if empty
    if db.get_all_core_memory()?.is_empty() {
        let user_md_path = home_dir.join("user.md");
        let user_md_content = std::fs::read_to_string(&user_md_path).ok();
        let user_md_ref = user_md_content
            .as_deref()
            .filter(|s| !s.starts_with("# Tell Mika about yourself"));
        db.seed_core_memory(user_md_ref)?;
        info!("seeded core memory for new database");
    }

    let async_db = AsyncDatabase::new(db);

    let claude = ClaudeClient::new(
        settings.anthropic_api_key.clone(),
        settings.claude_model.clone(),
        settings.claude_max_tokens,
    );
    let tool_registry = Arc::new(tools::default_tools());
    let ready = Arc::new(AtomicBool::new(false));

    // Validate required settings for server mode
    let gateway_url = settings
        .routing_url
        .clone()
        .ok_or_else(|| anyhow!("MIKA_ROUTING_URL is required in server mode"))?;
    let internal_token = settings
        .internal_token
        .clone()
        .ok_or_else(|| anyhow!("MIKA_INTERNAL_TOKEN is required in server mode"))?;

    let scheduler = Arc::new(ReminderScheduler {
        db: async_db.clone(),
        claude: claude.clone(),
        tools: tool_registry.clone(),
        home_dir: home_dir.to_path_buf(),
        message_sender: None, // Wired in PR 3 with GatewayMessageSender
    });

    let state = AppState {
        db: async_db,
        claude,
        tools: tool_registry,
        scheduler: scheduler.clone(),
        agent_lock: Arc::new(tokio::sync::Mutex::new(())),
        ready: ready.clone(),
        internal_token,
        gateway_url,
        home_dir: home_dir.to_path_buf(),
        startup_time: std::time::Instant::now(),
    };

    // Routes with auth (message, heartbeat) — route_layer applies to routes above it
    let app = Router::new()
        .route("/message", post(handlers::handle_message))
        .route("/heartbeat", post(handlers::handle_heartbeat))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_internal_token,
        ))
        // Health endpoint is OUTSIDE auth layer (for K8s probes)
        .route("/health", get(handlers::handle_health))
        .with_state(state);

    let port = settings.server_port;
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    info!(port, "mika-server listening");

    // Schedule future reminder timers (fast), then mark ready
    // (Full reminder recovery runs after health check is up)
    ready.store(true, Ordering::Release);
    info!("server ready");

    // Fire past-due reminders in background (slow, runs agent loops)
    tokio::spawn(async move {
        if let Err(e) = scheduler.recover().await {
            tracing::warn!(error = %e, "reminder recovery failed");
        }
    });

    // Serve with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("server shut down cleanly");
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use crate::test_utils::test_helpers::test_async_db;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let db = test_async_db();
        let claude = ClaudeClient::new(
            "test-key".to_string(),
            "claude-sonnet-4-6".to_string(),
            4096,
        );
        let tools_reg = Arc::new(tools::default_tools());
        let scheduler = Arc::new(ReminderScheduler {
            db: db.clone(),
            claude: claude.clone(),
            tools: tools_reg.clone(),
            home_dir: std::path::PathBuf::from("/tmp/mika-test"),
            message_sender: None,
        });
        AppState {
            db,
            claude,
            tools: tools_reg,
            scheduler,
            agent_lock: Arc::new(tokio::sync::Mutex::new(())),
            ready: Arc::new(AtomicBool::new(false)),
            internal_token: "test-token-secret".to_string(),
            gateway_url: "http://localhost:9999".to_string(),
            home_dir: std::path::PathBuf::from("/tmp/mika-test"),
            startup_time: std::time::Instant::now(),
        }
    }

    fn test_app(state: AppState) -> Router {
        Router::new()
            .route("/message", post(handlers::handle_message))
            .route("/heartbeat", post(handlers::handle_heartbeat))
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                auth::require_internal_token,
            ))
            .route("/health", get(handlers::handle_health))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_health_returns_503_before_ready() {
        let state = test_state();
        // ready is false by default
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_health_returns_200_after_ready() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert!(json["uptime_secs"].is_number());
    }

    #[tokio::test]
    async fn test_message_returns_401_without_token() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"text":"hi","chat_id":123,"channel":"telegram","request_id":"r1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_message_returns_401_with_wrong_token() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::from(
                        r#"{"text":"hi","chat_id":123,"channel":"telegram","request_id":"r1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_message_returns_202_accepted() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(
                        r#"{"text":"hello mika","chat_id":456,"channel":"telegram","request_id":"req-001"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let body = axum::body::to_bytes(resp.into_body(), 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["request_id"], "req-001");
        assert_eq!(json["status"], "accepted");
    }

    #[tokio::test]
    async fn test_message_returns_400_for_empty_text() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(
                        r#"{"text":"","chat_id":123,"channel":"telegram","request_id":"r1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_message_returns_429_when_busy() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);

        // Pre-acquire the lock to simulate a busy agent
        let _guard = state.agent_lock.clone().lock_owned().await;

        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(
                        r#"{"text":"hi","chat_id":123,"channel":"telegram","request_id":"r2"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let body = axum::body::to_bytes(resp.into_body(), 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "agent busy");
    }

    #[tokio::test]
    async fn test_message_stores_chat_id() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let db = state.db.clone();
        let app = test_app(state);

        let _resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(
                        r#"{"text":"hi","chat_id":789,"channel":"telegram","request_id":"r3"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Give the spawned task a moment to run
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let chat_id = db.get_customer_config("chat_id").await.unwrap();
        assert_eq!(chat_id, Some("789".to_string()));
    }

    #[tokio::test]
    async fn test_heartbeat_returns_401_without_token() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/heartbeat")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"request_id":"hb1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_heartbeat_returns_204_when_mutex_held() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);

        // Set timezone so active-hours check passes
        let _ = state.db.set_customer_config("timezone", "UTC").await;

        // Pre-acquire the lock to simulate a busy agent
        let _guard = state.agent_lock.clone().lock_owned().await;

        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/heartbeat")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(r#"{"request_id":"hb2"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
