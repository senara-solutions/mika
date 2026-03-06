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
use tracing::{info, warn};

use mika_common::agent;
use mika_common::claude::ClaudeClient;
use mika_common::config::Settings;
use mika_common::embedding::EmbeddingClient;
use mika_common::home;

use crate::async_db::AsyncDatabase;
use crate::db::Database;
use crate::messaging::{GatewayMessageSender, MessageSender};
use crate::skills::SkillRegistry;
use crate::startup;
use crate::task_engine::{self, TaskDispatcher, TaskEngine};
use crate::tools;

use state::{AgentState, AppState};

/// Cron schedule for the heartbeat recurring task: every hour on the hour.
/// Pre-filters (active hours, rate limits, recent activity) are applied at dispatch time.
const HEARTBEAT_CRON: &str = "0 0 * * * *";

/// Cron schedule for the nightly reflection task: 2am UTC every day.
const REFLECTION_CRON: &str = "0 0 2 * * *";

/// Build the Axum router with all routes and middleware.
///
/// Shared between production `run_server` and test `test_app`.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/message",
            post(handlers::handle_message).layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)),
        )
        .route("/tasks/{id}/complete", post(handlers::handle_task_complete))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_internal_token,
        ))
        // Health endpoint is OUTSIDE auth layer (for health probes)
        .route("/health", get(handlers::handle_health))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Initialize a single agent and return its AgentState with a running TaskEngine.
///
/// Uses the shared container database at `{global_home}/data/mika.db`.
#[allow(clippy::too_many_arguments)]
async fn init_agent(
    agent_name: &str,
    agent_home: &std::path::Path,
    global_home: &std::path::Path,
    claude: &ClaudeClient,
    tool_registry: &Arc<crate::tools::ToolRegistry>,
    gateway_url: &str,
    internal_token: &secrecy::SecretString,
    http_client: &reqwest::Client,
    embedding_client: Option<EmbeddingClient>,
    brave_api_key: Option<String>,
    disable_bundled_skills: bool,
) -> Result<AgentState> {
    let db_path = home::container_db_path(global_home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let db = Database::open(&db_path)?;
    db.register_agent(agent_name, agent_name, agent_home.to_str().unwrap_or(""))?;
    startup::seed_core_memory_if_empty(&db, agent_home, agent_name)?;
    startup::seed_bundled_skills_if_needed(agent_home, disable_bundled_skills);
    let async_db = AsyncDatabase::new_with_agent(db, agent_name);

    let skill_registry = Arc::new(SkillRegistry::from_dir(&agent_home.join("skills")));

    let engine_sender = GatewayMessageSender::new(
        gateway_url.to_string(),
        internal_token.clone(),
        async_db.clone(),
        http_client.clone(),
        None,
    );
    let engine_sender: Arc<dyn MessageSender> = Arc::new(engine_sender);

    let skills_dirty = Arc::new(AtomicBool::new(false));
    let agent_lock = Arc::new(tokio::sync::Mutex::new(()));

    let dispatcher = Arc::new(TaskDispatcher {
        db: async_db.clone(),
        claude: claude.clone(),
        tools: tool_registry.clone(),
        skills: skill_registry.clone(),
        home_dir: agent_home.to_path_buf(),
        message_sender: Some(engine_sender),
        embedding_client: embedding_client.clone(),
        brave_api_key,
        skills_dirty: skills_dirty.clone(),
        agent_lock: Some(agent_lock.clone()),
    });

    let task_engine = Arc::new(tokio::sync::Mutex::new(TaskEngine::new(
        async_db.clone(),
        dispatcher.clone(),
    )));

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
        task_engine,
        dispatcher,
        agent_lock,
        home_dir: agent_home.to_path_buf(),
        embedding_client,
        mcp_manager,
    };

    info!(agent = agent_name, home = %agent_home.display(), "initialized agent");

    Ok(agent_state)
}

/// Run background startup cleanup tasks (pruning, vacuum, backfill).
///
/// These are slow operations moved out of the hot path. Runs concurrently with
/// the main server after `ready` is set.
async fn startup_cleanup(db: AsyncDatabase) {
    // Prune old heartbeat sends (> 7 days)
    if let Err(e) = db.prune_old_heartbeat_sends(7).await {
        warn!(error = %e, "failed to prune old heartbeat sends");
    }

    // Prune old reflection runs (> 90 days)
    if let Err(e) = db.prune_old_reflection_runs(90).await {
        warn!(error = %e, "failed to prune old reflection runs");
    }

    // Compact old memory events into monthly summaries
    match db.compact_old_memory_events(90).await {
        Ok(deleted) if deleted > 0 => {
            info!(deleted, "compacted old memory events");
            if let Err(e) = db.vacuum().await {
                warn!(error = %e, "failed to vacuum database");
            }
        }
        Ok(_) => {}
        Err(e) => warn!(error = %e, "failed to compact old memory events"),
    }

    // Warn if DB is growing large
    match db.db_size_bytes().await {
        Ok(size) if size > 500_000_000 => {
            warn!(size_bytes = size, "database size exceeds 500MB");
        }
        Err(e) => warn!(error = %e, "failed to check database size"),
        _ => {}
    }

    // Backfill search index if empty (one-time migration for existing data)
    if let Ok(0) = db.count_search_content().await
        && let Ok(facts) = db.get_all_facts_for_indexing().await
        && !facts.is_empty()
    {
        info!(count = facts.len(), "backfilling search index");
        let mut content_ids: Vec<(i64, usize)> = Vec::new();
        for (i, (source_type, source_id, content)) in facts.iter().enumerate() {
            if let Ok(cid) = db
                .index_content(source_type, Some(*source_id), content)
                .await
            {
                content_ids.push((cid, i));
            }
        }
        info!("search index backfill complete");
        let _ = content_ids; // embeddings require embedding_client; skipped here
    }
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

    let agent_names = agent::list_agents(global_home);

    if agent_names.is_empty() {
        // No agents found — initialize default agent
        if !home::container_db_path(global_home).exists() {
            let default_home = home::resolve_agent_home(global_home, agent::DEFAULT_AGENT);
            let agent_state = init_agent(
                agent::DEFAULT_AGENT,
                &default_home,
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
        }
    } else {
        for name in &agent_names {
            let agent_home = agent::agent_dir(global_home, name);
            match init_agent(
                name,
                &agent_home,
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
            .await
            {
                Ok(agent_state) => {
                    agents.insert(name.clone(), Arc::new(agent_state));
                }
                Err(e) => {
                    warn!(agent = name, error = %e, "failed to initialize agent, skipping");
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

    let app = build_router(state.clone());

    let port = settings.server_port;
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    info!(port, "mika-server listening");

    ready.store(true, Ordering::Release);
    info!("server ready");

    // Start task engines for all agents:
    // 1. Register recurring tasks (heartbeat, reflection) if missing
    // 2. Run startup recovery (expire/recover in_progress, load heap)
    // 3. Spawn 1-second tick loop — collect JoinHandle for shutdown abort
    // 4. Spawn background cleanup (pruning, vacuum, backfill) as a fire-and-forget task
    let mut tick_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    for (name, agent_state) in state.agents.iter() {
        let engine = agent_state.task_engine.clone();
        let db = agent_state.db.clone();
        let agent_name = name.clone();

        // Register recurring built-in tasks
        task_engine::ensure_recurring_task(
            &db,
            "heartbeat",
            HEARTBEAT_CRON,
            r#"{"trigger":"heartbeat"}"#,
        )
        .await;
        task_engine::ensure_recurring_task(
            &db,
            "reflection",
            REFLECTION_CRON,
            r#"{"trigger":"reflection"}"#,
        )
        .await;

        // Startup recovery (expire timed-out tasks, mark orphaned in_progress as failed, load heap)
        {
            let mut eng = engine.lock().await;
            if let Err(e) = eng.startup_recovery().await {
                warn!(agent = %agent_name, error = %e, "task engine startup recovery failed");
            }
        }

        // Prune completed tasks older than 30 days to prevent unbounded DB growth
        task_engine::prune_old_tasks(&db).await;

        // Spawn tick loop and retain the handle so we can abort it on shutdown
        let handle = TaskEngine::spawn_tick_loop(engine);
        tick_handles.push(handle);

        // Background cleanup runs concurrently — fire and forget
        tokio::spawn(startup_cleanup(db));
    }

    // Serve with graceful shutdown; abort all tick loops once the signal fires
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            for handle in &tick_handles {
                handle.abort();
            }
        })
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
    use crate::task_engine::{TaskDispatcher, TaskEngine};
    use crate::test_utils::test_helpers::test_async_db;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use secrecy::SecretString;
    use tower::ServiceExt;

    fn test_task_engine(
        db: AsyncDatabase,
    ) -> (Arc<tokio::sync::Mutex<TaskEngine>>, Arc<TaskDispatcher>) {
        let claude = ClaudeClient::new(
            Some("test-key".to_string()),
            "claude-sonnet-4-6".to_string(),
            4096,
        )
        .unwrap();
        let dispatcher = Arc::new(TaskDispatcher {
            db: db.clone(),
            claude,
            tools: Arc::new(tools::default_tools()),
            skills: Arc::new(SkillRegistry::empty()),
            message_sender: None,
            home_dir: std::path::PathBuf::from("/tmp/mika-test"),
            embedding_client: None,
            brave_api_key: None,
            skills_dirty: Arc::new(AtomicBool::new(false)),
            agent_lock: None,
        });
        let engine = Arc::new(tokio::sync::Mutex::new(TaskEngine::new(
            db,
            dispatcher.clone(),
        )));
        (engine, dispatcher)
    }

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
        let (task_engine, dispatcher) = test_task_engine(db.clone());

        let agent_state = AgentState {
            db,
            skills: std::sync::Mutex::new(skills_reg),
            skills_dirty,
            task_engine,
            dispatcher,
            agent_lock,
            home_dir: std::path::PathBuf::from("/tmp/mika-test"),
            embedding_client: None,
            mcp_manager: None,
        };

        let mut agents = HashMap::new();
        agents.insert("mika".to_string(), Arc::new(agent_state));

        AppState {
            agents: Arc::new(agents),
            default_agent: "mika".to_string(),
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
                telemetry_enabled: false,
                otlp_endpoint: None,
                otlp_auth_header: None,
            },
        }
    }

    fn test_app(state: AppState) -> Router {
        build_router(state)
    }

    #[tokio::test]
    async fn test_health_returns_503_before_ready() {
        let state = test_state();
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

        let agent_state = state.agents.get("mika").unwrap();
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
        let db = state.agents.get("mika").unwrap().db.clone();
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

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let chat_id = db.get_customer_config("chat_id").await.unwrap();
        assert_eq!(chat_id, Some("789".to_string()));
    }

    #[tokio::test]
    async fn test_heartbeat_endpoint_removed() {
        // /heartbeat is no longer an HTTP endpoint — the task engine handles it internally
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/heartbeat")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(r#"{"request_id":"hb1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Route no longer exists — expect 404 or 405
        assert!(
            resp.status() == StatusCode::NOT_FOUND
                || resp.status() == StatusCode::METHOD_NOT_ALLOWED,
            "expected 404/405, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn test_message_with_agent_field() {
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
                        r#"{"text":"hi","chat_id":123,"channel":"telegram","request_id":"r-agent","agent":"mika"}"#,
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
        assert_eq!(images[1].media_type, "image/png");
    }

    #[tokio::test]
    async fn test_message_request_deserializes_without_images() {
        let json = r#"{"text":"hello","chat_id":42,"channel":"telegram","request_id":"r2"}"#;
        let req: super::types::MessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.text, "hello");
        assert!(req.images.is_none());
    }

    #[tokio::test]
    async fn test_image_payload_converts_to_agent_params_image_source() {
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
        ];
        let user_images: Vec<ImageSource> = payloads
            .into_iter()
            .map(|img| ImageSource {
                source_type: "base64".to_string(),
                media_type: img.media_type,
                data: img.data,
            })
            .collect();

        assert_eq!(user_images.len(), 2);
        assert_eq!(user_images[0].source_type, "base64");
        assert_eq!(user_images[0].media_type, "image/jpeg");
    }

    #[tokio::test]
    async fn test_message_with_multiple_image_types_accepted() {
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
    }

    #[tokio::test]
    async fn test_message_empty_text_with_images_accepted() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let body = serde_json::json!({
            "text": "",
            "chat_id": 456,
            "channel": "telegram",
            "request_id": "img-only-001",
            "images": [{"media_type": "image/jpeg", "data": "dGVzdA=="}]
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
    }

    #[tokio::test]
    async fn test_message_rejects_unsupported_image_media_type() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let body = serde_json::json!({
            "text": "look at this",
            "chat_id": 456,
            "channel": "telegram",
            "request_id": "bad-img-001",
            "images": [{"media_type": "image/bmp", "data": "dGVzdA=="}]
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
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let error = json["error"].as_str().unwrap();
        assert!(error.contains("unsupported media_type"));
        assert!(error.contains("image/bmp"));
    }

    #[tokio::test]
    async fn test_message_empty_images_array_with_empty_text_rejected() {
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

    // -- POST /tasks/{id}/complete tests --

    async fn create_callback_task(db: &AsyncDatabase) -> String {
        db.create_task(crate::db::NewTask {
            agent_id: db.agent_id.clone(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "analyze-codebase".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_task_complete_not_found() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tasks/00000000-0000-0000-0000-000000000000/complete")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(r#"{"result":"analysis done"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_task_complete_success() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let db = state.agents.get("mika").unwrap().db.clone();
        let task_id = create_callback_task(&db).await;
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/tasks/{task_id}/complete"))
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(r#"{"result":"3 vulnerabilities found"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "completed");
        assert_eq!(json["task_id"], task_id);

        // Verify the task was marked completed in DB
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(task.status, "completed");
        assert_eq!(task.result.as_deref(), Some("3 vulnerabilities found"));
    }

    #[tokio::test]
    async fn test_task_complete_empty_result_rejected() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let db = state.agents.get("mika").unwrap().db.clone();
        let task_id = create_callback_task(&db).await;
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/tasks/{task_id}/complete"))
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(r#"{"result":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_task_complete_requires_auth() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tasks/some-task-id/complete")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"result":"done"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_task_complete_already_completed_returns_conflict() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let db = state.agents.get("mika").unwrap().db.clone();
        let task_id = create_callback_task(&db).await;

        // Mark the task completed before the HTTP request
        let transitioned = db
            .update_task_completed(&task_id, Some("first result"))
            .await
            .unwrap();
        assert!(transitioned, "task should have transitioned to completed");

        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/tasks/{task_id}/complete"))
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(r#"{"result":"second attempt"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CONFLICT);

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["task_id"], task_id);
        assert!(
            json["error"]
                .as_str()
                .unwrap()
                .contains("cannot be completed")
        );
    }

    #[tokio::test]
    async fn test_task_complete_non_callback_task_rejected() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let db = state.agents.get("mika").unwrap().db.clone();

        // Create a time-triggered task (not callback)
        let task_id = db
            .create_task(crate::db::NewTask {
                agent_id: db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "reminder".to_string(),
                trigger_type: "time".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: Some(9_999_999_999),
                timeout_at: None,
                action_type: "send_message".to_string(),
                action_config: r#"{"text":"hi"}"#.to_string(),
                input_context: None,
                created_by_session: None,
            })
            .await
            .unwrap();

        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/tasks/{task_id}/complete"))
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(r#"{"result":"done"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("callback"));
    }
}
