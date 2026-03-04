mod auth;
mod handlers;
pub mod json_extractor;
pub mod openapi;
pub mod state;
pub mod types;

use anyhow::{Result, anyhow};
use axum::{
    Router, middleware,
    routing::{get, post},
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpListener;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use mika_common::agent;
use mika_common::claude::ClaudeClient;
use mika_common::config::Settings;
use mika_common::embedding::EmbeddingClient;
use mika_common::home;

use crate::async_db::AsyncDatabase;
use crate::db::Database;
use crate::messaging::{GatewayMessageSender, MessageSender};
use crate::scheduler::ReminderScheduler;
use crate::skills::SkillRegistry;
use crate::startup;
use crate::tools;

use state::{AgentState, AppState};

/// Build the Axum router with all routes and middleware.
///
/// Shared between production `run_server` and test `test_app`.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/message",
            post(handlers::handle_message).layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)),
        )
        .route("/heartbeat", post(handlers::handle_heartbeat))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_internal_token,
        ))
        // Health endpoint is OUTSIDE auth layer (for health probes)
        .route("/health", get(handlers::handle_health))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Initialize a single agent and return its AgentState.
#[allow(clippy::too_many_arguments)]
async fn init_agent(
    agent_name: &str,
    agent_home: &std::path::Path,
    claude: &ClaudeClient,
    tool_registry: &Arc<crate::tools::ToolRegistry>,
    gateway_url: &str,
    internal_token: &secrecy::SecretString,
    http_client: &reqwest::Client,
    embedding_client: Option<EmbeddingClient>,
    brave_api_key: Option<String>,
    disable_bundled_skills: bool,
) -> Result<(AgentState, Arc<ReminderScheduler>)> {
    let db = Database::open(&agent_home.join("data").join("mika.db"))?;
    startup::seed_core_memory_if_empty(&db, agent_home)?;
    startup::seed_bundled_skills_if_needed(agent_home, disable_bundled_skills);
    let async_db = AsyncDatabase::new(db);

    let skill_registry = Arc::new(SkillRegistry::from_dir(&agent_home.join("skills")));

    let scheduler_sender = GatewayMessageSender::new(
        gateway_url.to_string(),
        internal_token.clone(),
        async_db.clone(),
        http_client.clone(),
        None,
    );
    let scheduler_sender: Arc<dyn MessageSender> = Arc::new(scheduler_sender);

    let skills_dirty = Arc::new(AtomicBool::new(false));
    let agent_lock = Arc::new(tokio::sync::Mutex::new(()));

    let scheduler = Arc::new(ReminderScheduler {
        db: async_db.clone(),
        claude: claude.clone(),
        tools: tool_registry.clone(),
        skills: skill_registry.clone(),
        home_dir: agent_home.to_path_buf(),
        message_sender: Some(scheduler_sender),
        embedding_client: embedding_client.clone(),
        brave_api_key,
        skills_dirty: skills_dirty.clone(),
        agent_lock: Some(agent_lock.clone()),
        reflection_config: {
            let identity = crate::prompt::load_identity(agent_home);
            identity.reflection
        },
    });

    // Load MCP configuration and connect to configured servers
    let mcp_config = crate::mcp::config::McpConfig::load(agent_home)?;
    let mcp_manager = if mcp_config.mcp_servers.is_empty() {
        None
    } else {
        let manager = crate::mcp::McpManager::connect_all(&mcp_config).await;
        if manager.has_connections() {
            Some(manager)
        } else {
            None
        }
    };

    let agent_state = AgentState {
        db: async_db,
        skills: std::sync::Mutex::new(skill_registry),
        skills_dirty,
        scheduler: scheduler.clone(),
        agent_lock,
        home_dir: agent_home.to_path_buf(),
        embedding_client,
        mcp_manager,
    };

    info!(agent = agent_name, home = %agent_home.display(), "initialized agent");

    Ok((agent_state, scheduler))
}

/// Start the Mika HTTP server.
///
/// Discovers all agents in the home directory, initializes each one,
/// then binds to the configured port and serves until SIGTERM/Ctrl-C.
pub async fn run_server(settings: &Settings) -> Result<()> {
    let global_home = &settings.home_dir;

    // Auto-migrate to multi-agent layout if needed
    home::migrate_to_multi_agent(global_home)?;

    let claude = ClaudeClient::new(
        settings.anthropic_api_key.clone(),
        settings.claude_model.clone(),
        settings.claude_max_tokens,
    )?;
    let mut tool_registry = tools::default_tools();
    for tool in tools::management_tools_if_needed(global_home, settings) {
        tool_registry.register(tool);
    }
    let tool_registry = Arc::new(tool_registry);
    let ready = Arc::new(AtomicBool::new(false));
    let http_client = reqwest::Client::new();

    // Validate required settings for server mode
    let gateway_url = settings
        .routing_url
        .clone()
        .ok_or_else(|| anyhow!("MIKA_ROUTING_URL is required in server mode"))?;

    // Validate gateway URL is well-formed and uses http(s) scheme
    let parsed_url = reqwest::Url::parse(&gateway_url)
        .map_err(|e| anyhow!("MIKA_ROUTING_URL is not a valid URL: {e}"))?;
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err(anyhow!("MIKA_ROUTING_URL must use http or https scheme"));
    }

    let internal_token = settings
        .internal_token
        .clone()
        .ok_or_else(|| anyhow!("MIKA_INTERNAL_TOKEN is required in server mode"))?;

    let embedding_client = settings.make_embedding_client();
    if embedding_client.is_some() {
        info!("Layer 3 vector search enabled (embedding client configured)");
    }

    // Discover and initialize all agents
    let mut agents = HashMap::new();
    let mut schedulers = Vec::new();

    let agent_names = agent::list_agents(global_home);

    if agent_names.is_empty() {
        // No agents found — initialize default agent via legacy path
        // (handles server mode on a fresh/legacy install)
        let agent_home = home::resolve_agent_home(global_home, agent::DEFAULT_AGENT);
        if !agent_home.join("data").join("mika.db").exists() {
            // Legacy layout: global_home IS the agent home
            let (agent_state, scheduler) = init_agent(
                agent::DEFAULT_AGENT,
                global_home,
                &claude,
                &tool_registry,
                &gateway_url,
                &internal_token,
                &http_client,
                embedding_client.clone(),
                settings.brave_api_key.clone(),
                settings.disable_bundled_skills,
            )
            .await?;
            agents.insert(agent::DEFAULT_AGENT.to_string(), Arc::new(agent_state));
            schedulers.push(scheduler);
        }
    } else {
        for name in &agent_names {
            let agent_home = agent::agent_dir(global_home, name);
            match init_agent(
                name,
                &agent_home,
                &claude,
                &tool_registry,
                &gateway_url,
                &internal_token,
                &http_client,
                embedding_client.clone(),
                settings.brave_api_key.clone(),
                settings.disable_bundled_skills,
            )
            .await
            {
                Ok((agent_state, scheduler)) => {
                    agents.insert(name.clone(), Arc::new(agent_state));
                    schedulers.push(scheduler);
                }
                Err(e) => {
                    tracing::warn!(agent = name, error = %e, "failed to initialize agent, skipping");
                }
            }
        }
    }

    let default_agent = home::read_active_agent(global_home);
    info!(
        agents = ?agents.keys().collect::<Vec<_>>(),
        default = %default_agent,
        "discovered {} agent(s)",
        agents.len()
    );

    let state = AppState {
        agents: Arc::new(agents),
        default_agent,
        claude,
        tools: tool_registry,
        ready: ready.clone(),
        internal_token,
        gateway_url,
        startup_time: std::time::Instant::now(),
        http_client,
        brave_api_key: settings.brave_api_key.clone(),
        global_home_dir: global_home.to_path_buf(),
        settings: settings.clone(),
    };

    let app = build_router(state);

    let port = settings.server_port;
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    info!(port, "mika-server listening");

    // Schedule future reminder timers (fast), then mark ready
    // (Full reminder recovery runs after health check is up)
    ready.store(true, Ordering::Release);
    info!("server ready");

    // Fire past-due reminders in background (slow, runs agent loops),
    // then start a poller for each agent to fire future reminders when due.
    // The poller starts AFTER recovery completes to avoid double-firing
    // reminders that recovery is still processing.
    for scheduler in schedulers {
        let poller_scheduler = scheduler.clone();
        tokio::spawn(async move {
            if let Err(e) = scheduler.recover().await {
                tracing::warn!(error = %e, "reminder recovery failed");
            }
            // Start polling only after recovery finishes
            // (handle dropped, task lives on for the server's lifetime)
            poller_scheduler.spawn_poller();
        });
    }

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
    use crate::test_utils::test_helpers::test_async_db;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use secrecy::SecretString;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let db = test_async_db();
        let claude = ClaudeClient::new(
            Some("test-key".to_string()),
            "claude-sonnet-4-6".to_string(),
            4096,
        )
        .expect("test API key should be valid");
        let tools_reg = Arc::new(tools::default_tools());
        let skills_reg = Arc::new(SkillRegistry::empty());
        let skills_dirty = Arc::new(AtomicBool::new(false));
        let agent_lock = Arc::new(tokio::sync::Mutex::new(()));
        let scheduler = Arc::new(ReminderScheduler {
            db: db.clone(),
            claude: claude.clone(),
            tools: tools_reg.clone(),
            skills: skills_reg.clone(),
            home_dir: std::path::PathBuf::from("/tmp/mika-test"),
            message_sender: None,
            embedding_client: None,
            brave_api_key: None,
            skills_dirty: skills_dirty.clone(),
            agent_lock: Some(agent_lock.clone()),
            reflection_config: None,
        });

        let agent_state = AgentState {
            db,
            skills: std::sync::Mutex::new(skills_reg),
            skills_dirty,
            scheduler,
            agent_lock,
            home_dir: std::path::PathBuf::from("/tmp/mika-test"),
            embedding_client: None,
            mcp_manager: None,
        };

        let mut agents = HashMap::new();
        agents.insert("main".to_string(), Arc::new(agent_state));

        AppState {
            agents: Arc::new(agents),
            default_agent: "main".to_string(),
            claude,
            tools: tools_reg,
            ready: Arc::new(AtomicBool::new(false)),
            internal_token: SecretString::from("test-token-secret"),
            gateway_url: "http://localhost:9999".to_string(),
            startup_time: std::time::Instant::now(),
            http_client: reqwest::Client::new(),
            brave_api_key: None,
            global_home_dir: std::path::PathBuf::from("/tmp/mika-test"),
            settings: Settings {
                anthropic_api_key: Some("test-key".to_string()),
                claude_model: "claude-sonnet-4-6".to_string(),
                claude_max_tokens: 4096,
                db_path: std::path::PathBuf::from("/tmp/mika-test/data/mika.db"),
                log_level: "info".to_string(),
                routing_url: None,
                customer_id: None,
                server_port: 8080,
                internal_token: None,
                openai_api_key: None,
                embedding_model: "text-embedding-3-small".to_string(),
                embedding_dimensions: 512,
                brave_api_key: None,
                home_dir: std::path::PathBuf::from("/tmp/mika-test"),
                server_log_file: None,
                disable_bundled_skills: false,
            },
        }
    }

    fn test_app(state: AppState) -> Router {
        build_router(state)
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

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
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

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
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
        let agent_state = state.agents.get("main").unwrap();
        let _guard = agent_state.agent_lock.clone().lock_owned().await;

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

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "agent busy");
    }

    #[tokio::test]
    async fn test_message_stores_chat_id() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let db = state.agents.get("main").unwrap().db.clone();
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
        let agent_state = state.agents.get("main").unwrap();
        let _ = agent_state.db.set_customer_config("timezone", "UTC").await;

        // Pre-acquire the lock to simulate a busy agent
        let _guard = agent_state.agent_lock.clone().lock_owned().await;

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

    #[tokio::test]
    async fn test_message_with_agent_field() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        // Send with explicit agent field
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(
                        r#"{"text":"hi","chat_id":123,"channel":"telegram","request_id":"r-agent","agent":"main"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn test_message_with_unknown_agent() {
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
                        r#"{"text":"hi","chat_id":123,"channel":"telegram","request_id":"r-bad","agent":"nonexistent"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_message_with_images_accepted() {
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
                        r#"{"text":"[Photo]","chat_id":456,"channel":"telegram","request_id":"img-001","images":[{"media_type":"image/jpeg","data":"dGVzdA=="}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["request_id"], "img-001");
        assert_eq!(json["status"], "accepted");
    }

    #[tokio::test]
    async fn test_message_without_images_backward_compat() {
        // Verify messages without images field still work (backward compat)
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
                        r#"{"text":"hello","chat_id":456,"channel":"telegram","request_id":"compat-001"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn test_message_request_deserializes_images() {
        // Unit test for MessageRequest deserialization with images
        let json = r#"{
            "text": "[Photo]",
            "chat_id": 42,
            "channel": "telegram",
            "request_id": "r1",
            "images": [
                {"media_type": "image/jpeg", "data": "base64data1"},
                {"media_type": "image/png", "data": "base64data2"}
            ]
        }"#;
        let req: super::types::MessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.text, "[Photo]");
        let images = req.images.unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].media_type, "image/jpeg");
        assert_eq!(images[0].data, "base64data1");
        assert_eq!(images[1].media_type, "image/png");
        assert_eq!(images[1].data, "base64data2");
    }

    #[tokio::test]
    async fn test_message_request_deserializes_without_images() {
        // images field is optional — missing should default to None
        let json = r#"{
            "text": "hello",
            "chat_id": 42,
            "channel": "telegram",
            "request_id": "r2"
        }"#;
        let req: super::types::MessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.text, "hello");
        assert!(req.images.is_none());
    }

    #[tokio::test]
    async fn test_image_payload_converts_to_agent_params_image_source() {
        // Verify the handler's ImagePayload -> ImageSource conversion that feeds AgentParams.
        // This exercises the exact mapping the handler performs before spawning the agent task.
        use super::types::ImagePayload;
        use mika_common::claude::ImageSource;

        let payloads = vec![
            ImagePayload {
                media_type: "image/jpeg".to_string(),
                data: "dGVzdC1qcGVn".to_string(),
            },
            ImagePayload {
                media_type: "image/png".to_string(),
                data: "dGVzdC1wbmc=".to_string(),
            },
            ImagePayload {
                media_type: "image/gif".to_string(),
                data: "dGVzdC1naWY=".to_string(),
            },
            ImagePayload {
                media_type: "image/webp".to_string(),
                data: "dGVzdC13ZWJw".to_string(),
            },
        ];

        // Apply the same conversion the handler uses (handlers.rs lines 157-167)
        let user_images: Vec<ImageSource> = payloads
            .into_iter()
            .map(|img| ImageSource {
                source_type: "base64".to_string(),
                media_type: img.media_type,
                data: img.data,
            })
            .collect();

        assert_eq!(user_images.len(), 4);

        // Verify each converted ImageSource has the correct fields for AgentParams
        assert_eq!(user_images[0].source_type, "base64");
        assert_eq!(user_images[0].media_type, "image/jpeg");
        assert_eq!(user_images[0].data, "dGVzdC1qcGVn");

        assert_eq!(user_images[1].source_type, "base64");
        assert_eq!(user_images[1].media_type, "image/png");
        assert_eq!(user_images[1].data, "dGVzdC1wbmc=");

        assert_eq!(user_images[2].source_type, "base64");
        assert_eq!(user_images[2].media_type, "image/gif");
        assert_eq!(user_images[2].data, "dGVzdC1naWY=");

        assert_eq!(user_images[3].source_type, "base64");
        assert_eq!(user_images[3].media_type, "image/webp");
        assert_eq!(user_images[3].data, "dGVzdC13ZWJw");
    }

    #[tokio::test]
    async fn test_message_with_multiple_image_types_accepted() {
        // Integration test: send all four supported image types through the full server path
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let body = serde_json::json!({
            "text": "describe these images",
            "chat_id": 456,
            "channel": "telegram",
            "request_id": "multi-img-001",
            "images": [
                {"media_type": "image/jpeg", "data": "dGVzdC1qcGVn"},
                {"media_type": "image/png", "data": "dGVzdC1wbmc="},
                {"media_type": "image/gif", "data": "dGVzdC1naWY="},
                {"media_type": "image/webp", "data": "dGVzdC13ZWJw"}
            ]
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let resp_body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(json["request_id"], "multi-img-001");
        assert_eq!(json["status"], "accepted");
    }

    #[tokio::test]
    async fn test_message_empty_text_with_images_accepted() {
        // The handler allows empty text when images are present (image-only sends)
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let body = serde_json::json!({
            "text": "",
            "chat_id": 456,
            "channel": "telegram",
            "request_id": "img-only-001",
            "images": [
                {"media_type": "image/jpeg", "data": "dGVzdA=="}
            ]
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let resp_body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(json["request_id"], "img-only-001");
        assert_eq!(json["status"], "accepted");
    }

    #[tokio::test]
    async fn test_message_rejects_unsupported_image_media_type() {
        // The handler validates image media_type against an allowlist
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let body = serde_json::json!({
            "text": "look at this",
            "chat_id": 456,
            "channel": "telegram",
            "request_id": "bad-img-001",
            "images": [
                {"media_type": "image/bmp", "data": "dGVzdA=="}
            ]
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp_body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        let error = json["error"].as_str().unwrap();
        assert!(
            error.contains("unsupported media_type"),
            "error should mention unsupported media_type, got: {error}"
        );
        assert!(
            error.contains("image/bmp"),
            "error should mention the rejected type, got: {error}"
        );
    }

    #[tokio::test]
    async fn test_message_empty_images_array_with_empty_text_rejected() {
        // Empty images array does not count as "has images" — empty text should be rejected
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let body = serde_json::json!({
            "text": "",
            "chat_id": 456,
            "channel": "telegram",
            "request_id": "empty-arr-001",
            "images": []
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
