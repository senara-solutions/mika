mod a2a;
mod auth;
pub mod checkpoint;
pub mod ci_failure_handler;
pub mod ci_success_handler;
pub mod dashboard;
pub mod dashboard_dev_runs;
pub mod embedded_dashboard;
mod handlers;
pub mod investigate;
pub mod json_extractor;
mod milestone_context_handler;
pub mod openapi;
pub mod ready_label_handler;
pub mod rewind;
pub mod state;
pub mod types;
pub mod variants;
pub(crate) mod verdict;
pub mod verdict_handler;
pub mod webhook_queue;

use crate::kg::config::KgAgentConfig;
use anyhow::{Context as _, Result, anyhow};
use axum::{
    Router,
    extract::Request,
    http, middleware,
    response::Response,
    routing::{get, post},
};
use secrecy::ExposeSecret;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::{debug, error, info, warn};

/// Carries HTTP method and path from request to response extensions,
/// making them available to TraceLayer's `on_response` callback as
/// top-level JSON fields (not nested inside the `spans` array).
#[derive(Clone, Debug)]
struct RequestMeta {
    method: String,
    path: String,
}

/// Middleware that captures method and path from the request and injects
/// them into response extensions for downstream logging.
async fn inject_request_meta(request: Request, next: middleware::Next) -> Response {
    let meta = RequestMeta {
        method: request.method().to_string(),
        path: request.uri().path().to_owned(),
    };
    let mut response = next.run(request).await;
    response.extensions_mut().insert(meta);
    response
}

use mika_common::agent;
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

/// Cron schedule for the auto-pull check: every 10 minutes (mika#1363).
/// Pre-filters (groomed-not-ready, priority ordering, queue-empty gate) applied at dispatch time.
const AUTO_PULL_CRON: &str = "0 */10 * * * *";

/// Default cron schedule for the curator review: daily at 03:00 UTC (mika#1584).
const CURATOR_REVIEW_CRON: &str = "0 0 3 * * *";

/// Build the Axum router with all routes and middleware.
///
/// Shared between production `run_server` and test `test_app`.
fn build_router(state: AppState) -> Router {
    // CORS — scoped to dashboard routes only (GET + POST + OPTIONS)
    let cors_origin =
        std::env::var("MIKA_CORS_ORIGIN").unwrap_or_else(|_| "http://localhost:5173".to_string());
    let cors = CorsLayer::new()
        .allow_origin(
            cors_origin
                .parse::<http::HeaderValue>()
                .expect("MIKA_CORS_ORIGIN must be a valid header value"),
        )
        .allow_methods([http::Method::GET, http::Method::OPTIONS, http::Method::POST])
        .allow_headers([http::header::AUTHORIZATION, http::header::CONTENT_TYPE]);

    // Dashboard API routes (read-only, accepts dashboard OR internal token)
    let dashboard_routes = Router::new()
        .route("/timeline", get(dashboard::handle_timeline))
        .route(
            "/timeline/trace/{trace_id}",
            get(dashboard::handle_trace_detail),
        )
        .route(
            "/timeline/trace/{trace_id}/messages",
            get(dashboard::handle_trace_messages),
        )
        .route("/agents", get(dashboard::handle_agents_list))
        .route("/agents/{id}", get(dashboard::handle_agent_detail))
        .route(
            "/agents/{id}/sessions",
            get(dashboard::handle_agent_sessions),
        )
        .route("/agents/{id}/audit", get(dashboard::handle_agent_audit))
        .route("/agents/{id}/facts", get(dashboard::handle_agent_facts))
        .route("/sessions", get(dashboard::handle_sessions_list))
        .route("/sessions/{id}", get(dashboard::handle_session_detail))
        .route(
            "/sessions/{id}/messages",
            get(dashboard::handle_session_messages),
        )
        .route("/tasks", get(dashboard::handle_tasks_list))
        .route("/tasks/{task_id}", get(dashboard::handle_task_detail))
        .route(
            "/tasks/{task_id}/children",
            get(dashboard::handle_task_children),
        )
        .route(
            "/tasks/{task_id}/descendants",
            get(dashboard::handle_task_descendants),
        )
        .route(
            "/tasks/{task_id}/sessions",
            get(dashboard::handle_task_sessions),
        )
        .route("/team-runs", get(dashboard::handle_team_runs_list))
        .route(
            "/team-runs/{run_id}",
            get(dashboard::handle_team_run_detail),
        )
        .route(
            "/team-runs/{run_id}/workspace",
            get(dashboard::handle_team_workspace),
        )
        .route(
            "/team-runs/{run_id}/summary",
            get(dashboard::handle_team_run_summary),
        )
        // Observability: LLM calls and tool calls
        .route("/llm-calls", get(dashboard::handle_llm_calls))
        .route("/llm-calls/cost-trend", get(dashboard::handle_cost_trend))
        .route("/llm-calls/{id}", get(dashboard::handle_llm_call_detail))
        .route(
            "/llm-calls/{id}/tool-calls",
            get(dashboard::handle_llm_call_tool_calls),
        )
        .route("/tool-calls", get(dashboard::handle_tool_calls))
        .route("/tool-calls/{id}", get(dashboard::handle_tool_call_detail))
        .route(
            "/traces/{trace_id}/llm-calls",
            get(dashboard::handle_trace_llm_calls),
        )
        .route(
            "/traces/{trace_id}/tool-calls",
            get(dashboard::handle_trace_tool_calls),
        )
        .route(
            "/sessions/{id}/llm-calls",
            get(dashboard::handle_session_llm_calls),
        )
        .route(
            "/sessions/{id}/tool-calls",
            get(dashboard::handle_session_tool_calls),
        )
        .route(
            "/sessions/{id}/skills",
            get(dashboard::handle_session_skills),
        )
        // Skills listing with optional property filters (mika#606)
        .route("/skills", get(dashboard::handle_skills_list))
        // Skill lifecycle management (mika#1582) — operator-tier
        .route(
            "/skills/{name}/promote",
            post(handlers::handle_skill_promote),
        )
        .route(
            "/skills/{name}/archive",
            post(handlers::handle_skill_archive),
        )
        .route("/dev-runs", get(dashboard_dev_runs::handle_dev_runs_list))
        .route(
            "/dev-runs/{task_id}",
            get(dashboard_dev_runs::handle_dev_run_detail),
        )
        // GitHub proxy endpoints for dashboard (issue/PR details)
        .route(
            "/github/issues/{owner}/{repo}/{number}",
            get(dashboard_dev_runs::handle_github_issue),
        )
        .route(
            "/github/pulls/{owner}/{repo}/{number}",
            get(dashboard_dev_runs::handle_github_pull),
        )
        .route(
            "/investigate",
            post(investigate::handle_investigate).layer(RequestBodyLimitLayer::new(
                investigate::INVESTIGATE_BODY_LIMIT,
            )),
        )
        // Dashboard toggle endpoints
        .route("/dashboard/enable", post(embedded_dashboard::handle_enable))
        .route(
            "/dashboard/disable",
            post(embedded_dashboard::handle_disable),
        )
        .route("/dashboard/status", get(embedded_dashboard::handle_status))
        // Variant management endpoints (read-only)
        .route("/skills/variants", get(variants::handle_variants_summary))
        .route(
            "/skills/{skill}/variants",
            get(variants::handle_skill_variants),
        )
        // Static route MUST come before {provider}/{model} wildcard
        .route(
            "/skills/{skill}/variants/reflect",
            get(variants::handle_variant_reflect),
        )
        .route(
            "/skills/{skill}/variants/{provider}/{model}",
            get(variants::handle_variant_detail),
        )
        .route(
            "/skills/{skill}/variants/{provider}/{model}/diff",
            get(variants::handle_variant_diff),
        )
        // Operational items (feature-gated behind MIKA_OPERATIONAL_PARTNER)
        .route(
            "/operational-items",
            get(dashboard::handle_operational_items),
        )
        // Variant mutation endpoints (still dashboard-auth for validate, promote needs internal)
        .route(
            "/skills/{skill}/variants/{provider}/{model}/validate",
            post(variants::handle_variant_validate),
        )
        .route(
            "/skills/{skill}/variants/{provider}/{model}/promote",
            post(variants::handle_variant_promote),
        )
        // Skill restore endpoint (mika#1584)
        .route(
            "/skills/{name}/restore",
            post(dashboard::handle_skill_restore),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_dashboard_or_internal_token,
        ));

    // Rewind routes — internal token only, no CORS (server-to-server)
    let rewind_routes = Router::new()
        .route("/rewind/preview", post(rewind::handle_rewind_preview))
        .route("/rewind/execute", post(rewind::handle_rewind_execute))
        .route("/rewind/resolve", post(rewind::handle_rewind_resolve))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_internal_token,
        ));

    // A2A routes (no CORS — gateway handles external auth).
    // Accepts only MIKA_INTERNAL_TOKEN.
    let a2a_routes = Router::new()
        .route(
            "/a2a/{agent_name}",
            post(a2a::handle_a2a_jsonrpc).layer(RequestBodyLimitLayer::new(2 * 1024 * 1024)),
        )
        .route("/a2a/{agent_name}/agent.json", get(a2a::handle_agent_card))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_internal_token,
        ));

    // Mutation routes (no CORS — gateway-to-agent only).
    // Accepts only MIKA_INTERNAL_TOKEN.
    let mutation_routes = Router::new()
        .route(
            "/message",
            post(handlers::handle_message).layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)),
        )
        .route("/tasks/{id}/complete", post(handlers::handle_task_complete))
        .route("/tasks/{id}/cancel", post(handlers::handle_task_cancel))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_internal_token,
        ));

    // Merge dashboard + rewind into a single /api/v1 nest — no prefix conflict.
    // CORS applied after merge so both sub-routers are reachable (Axum .layer()
    // wraps the router as a service; merging after .layer() can shadow routes).
    mutation_routes
        .merge(a2a_routes)
        .nest("/api/v1", dashboard_routes.merge(rewind_routes).layer(cors))
        // Embedded dashboard SPA (no auth — static assets; SPA authenticates its own API calls)
        .nest("/dashboard", embedded_dashboard::dashboard_routes())
        // Root: redirect to dashboard (if enabled) or return JSON info
        .route("/", get(embedded_dashboard::handle_root))
        // Health endpoint is OUTSIDE auth layer (for health probes)
        .route("/health", get(handlers::handle_health))
        // inject_request_meta must be inner to TraceLayer so that on the response
        // path it inserts RequestMeta into extensions BEFORE on_response reads them.
        .layer(middleware::from_fn(inject_request_meta))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &http::Request<_>| {
                    let path = request.uri().path();
                    let method = request.method();
                    if path == "/health" {
                        tracing::debug_span!("http_request", %method, path)
                    } else {
                        tracing::info_span!("http_request", %method, path)
                    }
                })
                .on_response(
                    |response: &http::Response<_>, latency: Duration, span: &tracing::Span| {
                        let status = response.status().as_u16();
                        let is_debug = span
                            .metadata()
                            .is_some_and(|m| *m.level() == tracing::Level::DEBUG);
                        let (method, path) = response
                            .extensions()
                            .get::<RequestMeta>()
                            .map(|m| (m.method.as_str(), m.path.as_str()))
                            .unwrap_or(("unknown", "unknown"));
                        if status >= 500 {
                            tracing::warn!(status, method, path, ?latency, "response");
                        } else if is_debug {
                            tracing::debug!(status, method, path, ?latency, "response");
                        } else {
                            tracing::info!(status, method, path, ?latency, "response");
                        }
                    },
                )
                .on_failure(
                    // NOTE: on_failure fires on connection-level failures where no
                    // response is produced. RequestMeta is carried via response
                    // extensions, so it is unavailable here. Method and path remain
                    // accessible via the parent span's fields (nested in JSON output
                    // under `spans`). Connection-level failures are rare.
                    |error: tower_http::classify::ServerErrorsFailureClass,
                     latency: Duration,
                     _span: &tracing::Span| {
                        tracing::error!(
                            classification = %error,
                            ?latency,
                            "response failed"
                        );
                    },
                ),
        )
        .with_state(state)
}

/// Initialize a single agent and return its AgentState with a running TaskEngine.
///
/// Uses the shared container database at `{global_home}/data/mika.db`.
/// Loads per-agent settings via `Settings::load_for_agent()` so each agent gets
/// its own LLM provider (respecting per-agent `config.toml` overrides). See #323.
/// Per-agent `.env` values are injected via `Settings::load_for_agent()` — no
/// process env mutation needed. See #430.
#[allow(clippy::too_many_arguments)]
async fn init_agent(
    agent_name: &str,
    agent_home: &std::path::Path,
    global_home: &std::path::Path,
    tool_registry: &Arc<crate::tools::ToolRegistry>,
    gateway_url: &str,
    internal_token: &secrecy::SecretString,
    http_client: &reqwest::Client,
    embedding_client: Option<EmbeddingClient>,
    brave_api_key: Option<String>,
    disable_bundled_skills: bool,
    pr_reviews_posted: Arc<dashmap::DashMap<String, std::collections::HashSet<String>>>,
) -> Result<AgentState> {
    // Load per-agent settings (global config.toml → agent config.toml → agent .env → env vars).
    // Settings::load_for_agent injects per-agent .env as a config source (#430),
    // so each agent gets its own secrets (e.g., MIKA_GITHUB_APP_*) without
    // polluting the process environment.
    let agent_settings = Settings::load_for_agent(global_home, agent_home)?;
    let github_token = agent_settings.agent_github_token().map(String::from);
    let agent_llm = agent_settings.make_llm_provider()?;
    let db_path = home::container_db_path(global_home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let mut db = Database::open(&db_path)?;
    let identity = crate::prompt::load_identity(agent_home);
    let kg_config = crate::kg::config::resolve_per_agent_docs_root(&identity, &agent_settings)
        .with_context(|| format!("failed to resolve [kg] config for agent {agent_name}"))?;
    db.register_agent(
        agent_name,
        &identity.name,
        agent_home.to_str().unwrap_or(""),
    )?;
    startup::seed_core_memory_if_empty(&db, agent_home, agent_name)?;
    startup::seed_bundled_skills_if_needed(agent_home, disable_bundled_skills);
    if agent_settings.dev_mode {
        // One-time migration: delete stale denylist rows for agents that
        // moved to identity-driven [skills].allowlist (#815). Idempotent
        // via schema_meta marker — no-op after first successful run.
        crate::well_known_agents::migrate_well_known_to_identity_allowlist(&mut db);
        crate::well_known_agents::seed_well_known_skill_overrides(&mut db, agent_name);
    }
    let async_db = AsyncDatabase::new_with_agent(db, agent_name);

    let skills_dir = agent_home.join("skills");
    // Migrate legacy .disabled marker files to DB (one-shot, idempotent).
    if let Err(e) =
        crate::skills::migrate_disabled_markers_async(&skills_dir, &async_db, agent_name).await
    {
        tracing::warn!(error = %e, "failed to migrate .disabled markers");
    }
    let mut skill_registry = SkillRegistry::from_dir(&skills_dir);
    // Phase -1: identity-driven skill allowlist (well-known agents like mika-arch)
    if let Some(ref allowlist) = identity.skills.allowlist {
        skill_registry.apply_identity_allowlist(allowlist);
    }
    // Phase 0+1: DB-backed overrides (disabled state, always_on, LLM provider/model)
    if let Ok(overrides) = async_db.get_skill_overrides(agent_name).await {
        skill_registry.apply_overrides(&overrides);
    }
    skill_registry.apply_load_safety_check();
    // Runtime allowlist↔required_tools coherence guard (mika#1576). Runs after
    // identity allowlist + DB overrides + safety check — the one point where both
    // the allowlist and each skill's required_tools are simultaneously visible.
    skill_registry.apply_required_tools_coherence_check(agent_name);
    skill_registry.log_summary();
    let skill_registry = Arc::new(skill_registry);

    let engine_sender = GatewayMessageSender::new(
        gateway_url.to_string(),
        internal_token.clone(),
        async_db.clone(),
        http_client.clone(),
        None,
        Some(agent_name.to_string()),
        None,
        agent_settings.customer_id.clone(),
    );
    let engine_sender: Arc<dyn MessageSender> = Arc::new(engine_sender);

    let skills_dirty = Arc::new(AtomicBool::new(false));
    let agent_lock = Arc::new(tokio::sync::Mutex::new(()));
    let github_app = mika_common::github_app::GitHubApp::from_settings(&agent_settings);

    let dispatcher = Arc::new(TaskDispatcher {
        db: async_db.clone(),
        llm: agent_llm.clone(),
        tools: tool_registry.clone(),
        skills: skill_registry.clone(),
        home_dir: agent_home.to_path_buf(),
        message_sender: Some(engine_sender),
        embedding_client: embedding_client.clone(),
        brave_api_key,
        github_token,
        github_app: github_app.clone(),
        skills_dirty: skills_dirty.clone(),
        agent_lock: Some(agent_lock.clone()),
        // Server mode: engine dispatches callbacks via dispatch_undelivered_callbacks().
        // The agent_lock serializes concurrent dispatch attempts.
        cli_mode: false,
        settings: agent_settings.clone(),
        pr_reviews_posted: Some(pr_reviews_posted),
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
        settings: agent_settings,
        llm: agent_llm,
        github_app,
        webhook_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        kg_config,
    };

    debug!(agent = agent_name, home = %agent_home.display(), "initialized agent");

    Ok(agent_state)
}

/// Run background startup cleanup tasks (pruning, vacuum, backfill).
///
/// These are slow operations moved out of the hot path. Runs concurrently with
/// the main server after `ready` is set.
async fn startup_cleanup(db: AsyncDatabase, embedding_client: Option<EmbeddingClient>) {
    /// Max items per `embed_batch()` API call — conservative to avoid rate limits.
    const EMBEDDING_BACKFILL_BATCH_SIZE: usize = 100;

    // Prune old heartbeat sends (> 7 days)
    if let Err(e) = db.prune_old_heartbeat_sends(7).await {
        warn!(error = %e, "failed to prune old heartbeat sends");
    }

    // Prune old reflection runs (> 90 days)
    if let Err(e) = db.prune_old_reflection_runs(90).await {
        warn!(error = %e, "failed to prune old reflection runs");
    }

    // Compact old memory events into monthly summaries
    match db.compact_old_audit_events(90).await {
        Ok(deleted) if deleted > 0 => {
            info!(deleted, "compacted old audit events");
            if let Err(e) = db.vacuum().await {
                warn!(error = %e, "failed to vacuum database");
            }
        }
        Ok(_) => {}
        Err(e) => warn!(error = %e, "failed to compact old audit events"),
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
        for (source_type, source_id, content) in &facts {
            let _ = db
                .index_content(source_type, Some(*source_id), content)
                .await;
        }
        info!("search index backfill complete");
    }

    // Backfill embeddings for rows missing them (idempotent, runs every startup)
    if let Some(ref client) = embedding_client {
        match db.get_unembedded_content().await {
            Ok(unembedded) if !unembedded.is_empty() => {
                info!(
                    count = unembedded.len(),
                    "backfilling embeddings for search content"
                );
                for chunk in unembedded.chunks(EMBEDDING_BACKFILL_BATCH_SIZE) {
                    let texts: Vec<&str> = chunk.iter().map(|(_, c)| c.as_str()).collect();
                    match client.embed_batch(&texts).await {
                        Ok(embeddings) => {
                            for ((id, _), emb) in chunk.iter().zip(embeddings) {
                                if let Err(e) = db.index_embedding(*id, emb).await {
                                    warn!(content_id = id, error = %e, "failed to store embedding");
                                }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "embedding batch failed, will retry on next startup");
                            break;
                        }
                    }
                }
                info!("embedding backfill complete");
            }
            Err(e) => {
                warn!(error = %e, "failed to query unembedded content");
            }
            _ => {}
        }
    }
}

/// Start the Mika HTTP server.
///
/// Discovers all agents in the home directory, initializes each one,
/// then binds to the configured port and serves until SIGTERM/Ctrl-C.
pub async fn run_server(settings: &Settings) -> Result<()> {
    let global_home = &settings.home_dir;
    let is_pretty = settings.log_format == "pretty";

    if is_pretty {
        mika_common::logging::print_banner("mika-spirit", env!("CARGO_PKG_VERSION"));
    }

    // Auto-migrate to multi-agent layout if needed
    home::migrate_to_multi_agent(global_home)?;

    // Auto-provision well-known dev agents if dev_mode is enabled
    if settings.dev_mode {
        crate::well_known_agents::provision_well_known_agents(
            global_home,
            settings,
            settings.disable_agent_provisioning,
        );
    }

    // Warn if embedded dashboard is enabled but no assets were compiled in
    if settings.dashboard_enabled && !embedded_dashboard::has_embedded_assets() {
        warn!(
            "MIKA_DASHBOARD_ENABLED=true but no dashboard assets are embedded in the binary. \
             Build the dashboard first: npm run build --prefix dashboard && cargo build"
        );
    }

    let http_client = reqwest::Client::new();
    let global_github_app = mika_common::github_app::GitHubApp::from_settings(settings);
    let mut tool_registry = tools::default_tools();
    for tool in tools::management_tools_if_needed(
        global_home,
        settings,
        http_client.clone(),
        global_github_app.clone(),
    ) {
        tool_registry.register(tool);
    }
    let tool_registry = Arc::new(tool_registry);
    let ready = Arc::new(AtomicBool::new(false));

    // Validate required settings for server mode
    let gateway_url = settings.routing_url.clone().ok_or_else(|| {
        anyhow!(
            "MIKA_ROUTING_URL is required in server mode. \
             Run `mika setup --mode server` to configure, or set the env var directly."
        )
    })?;

    // Validate gateway URL is well-formed and uses http(s) scheme
    let parsed_url = reqwest::Url::parse(&gateway_url)
        .map_err(|e| anyhow!("MIKA_ROUTING_URL is not a valid URL: {e}"))?;
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err(anyhow!("MIKA_ROUTING_URL must use http or https scheme"));
    }

    let internal_token = settings.internal_token.clone().ok_or_else(|| {
        anyhow!(
            "MIKA_INTERNAL_TOKEN is required in server mode. \
             Run `mika setup --mode server` to configure, or set the env var directly."
        )
    })?;

    let dashboard_token = settings.dashboard_token.clone();
    if dashboard_token.is_some() {
        info!("dashboard token configured — dashboard routes use separate auth");
    }

    let embedding_client = settings.make_embedding_client();
    if embedding_client.is_some() {
        info!("Layer 3 vector search enabled (embedding client configured)");
    }

    // Discover and initialize all agents
    let agents: dashmap::DashMap<String, Arc<AgentState>> = dashmap::DashMap::new();

    // Session-scoped PR review dedup map (#821). Created once, shared across
    // all agents via their TaskDispatchers and AppState.
    let pr_reviews_posted: Arc<dashmap::DashMap<String, std::collections::HashSet<String>>> =
        Arc::new(dashmap::DashMap::new());

    let agent_names = agent::list_agents(global_home);

    if agent_names.is_empty() {
        // No agents found — initialize default agent
        if !home::container_db_path(global_home).exists() {
            let default_home = home::resolve_agent_home(global_home, agent::DEFAULT_AGENT);
            let agent_state = init_agent(
                agent::DEFAULT_AGENT,
                &default_home,
                global_home,
                &tool_registry,
                &gateway_url,
                &internal_token,
                &http_client,
                embedding_client.clone(),
                settings
                    .brave_api_key
                    .as_ref()
                    .map(|s| s.expose_secret().to_string()),
                settings.disable_bundled_skills,
                pr_reviews_posted.clone(),
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
                &tool_registry,
                &gateway_url,
                &internal_token,
                &http_client,
                embedding_client.clone(),
                settings
                    .brave_api_key
                    .as_ref()
                    .map(|s| s.expose_secret().to_string()),
                settings.disable_bundled_skills,
                pr_reviews_posted.clone(),
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

    let mut default_agent = home::read_active_agent(global_home);
    {
        let agent_names: Vec<_> = agents.iter().map(|r| r.key().clone()).collect();
        info!(
            agents = ?agent_names,
            default = %default_agent,
            "discovered {} agent(s)",
            agent_names.len()
        );
    }

    // Validate the active agent exists; fall back to the first available agent
    // if the active_agent file points to a name that no longer exists.
    if agents.get(&default_agent).is_none() {
        if let Some(fallback) = agents.iter().next().map(|r| r.key().clone()) {
            warn!(
                requested = %default_agent,
                fallback = %fallback,
                "active agent not found, falling back"
            );
            default_agent = fallback;
            let _ = home::write_active_agent(global_home, &default_agent);
        } else {
            anyhow::bail!("no agents available");
        }
    }

    // Create an unscoped dashboard DB handle that shares the same DB thread
    // as the default agent. Dashboard queries don't filter by agent_id.
    let dashboard_db = agents
        .get(&default_agent)
        .expect("default agent must exist")
        .value()
        .db
        .clone();

    // Defense-in-depth for mika#636: periodic WAL checkpoint forces the
    // dashboard connection to advance past any held WAL snapshot, ensuring
    // writes from other processes (e.g. `mika ask`) become visible.
    checkpoint::spawn_dashboard_checkpoint_task(dashboard_db.clone());

    // Domain graph rebuild: populate kg_entities and kg_relationships from
    // authoritative sources (skill manifests, tool registry, MCP, agent configs).
    // Runs once per server boot, after all agents are initialized. Failures are
    // logged — the server continues to boot with stale or empty KG data.
    {
        let default_state = agents
            .get(&default_agent)
            .expect("default agent must exist")
            .value()
            .clone();
        // Clone the Arc<SkillRegistry> and drop the MutexGuard before the await
        // to avoid holding a std::sync::Mutex across an async boundary.
        let skill_reg = default_state.skills.lock().expect("skills lock").clone();
        let agent_infos: Vec<crate::kg::domain_builder::AgentInfo> = agents
            .iter()
            .map(|entry| crate::kg::domain_builder::AgentInfo {
                name: entry.key().clone(),
                role: None,
                model: Some(entry.value().llm.model_name().to_string()),
            })
            .collect();
        let builder = crate::kg::domain_builder::DomainGraphBuilder::new(
            &dashboard_db,
            &skill_reg,
            &tool_registry,
            default_state.mcp_manager.as_ref(),
            &agent_infos,
        );
        match builder.rebuild().await {
            Ok(stats) => info!(
                event = "domain_rebuild_complete",
                added = stats.entities_added,
                updated = stats.entities_updated,
                removed = stats.entities_removed,
                depends_on = stats.edges_depends_on,
                provides = stats.edges_provides,
                duration_ms = stats.duration_ms,
                "domain graph ready"
            ),
            Err(e) => warn!(
                error = %e,
                "domain graph rebuild failed; KG queries may return stale results until next restart"
            ),
        }
    }

    // Lexical graph ingestion: chunk docs per agent, index into
    // search_content for hybrid search. Runs after domain rebuild so each
    // KG layer's startup work composes cleanly for #690+.
    //
    // Per-agent docs_root resolved from identity.toml [kg] section (#778).
    // Agents with matching docs_root share a corpus via docs_root_hash (v27).
    // Agents with [kg] enabled=false are skipped entirely.
    //
    // Graceful shutdown token for KG background tasks (#802).
    // Cancelled on SIGTERM/Ctrl-C before tick handle abort, giving
    // in-flight batches a cooperative exit point.
    let kg_shutdown_token = CancellationToken::new();
    let mut kg_tick_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    {
        for entry in agents.iter() {
            let agent_name = entry.key();
            let agent_state = entry.value();
            let KgAgentConfig::Enabled { ref corpora } = agent_state.kg_config else {
                info!(
                    agent = agent_name.as_str(),
                    event = "lexical_ingest_disabled",
                    reason = ?agent_state.kg_config,
                    "KG lexical ingestion disabled — shared-corpus rows remain in DB; \
                     use `mika kg purge --agent <name>` to clean up (#779)"
                );
                continue;
            };

            for (corpus_idx, corpus) in corpora.iter().enumerate() {
                let docs_root = &corpus.docs_root;
                let docs_root_hash = &corpus.docs_root_hash;

                // CwdDefault paths may not exist on disk — warn-and-skip per #738 policy.
                if !docs_root.exists() {
                    info!(
                        agent = agent_name.as_str(),
                        docs_root = %docs_root.display(),
                        corpus_index = corpus_idx,
                        corpus_count = corpora.len(),
                        "docs_root not found — skipping lexical ingestion for this corpus"
                    );
                    continue;
                }

                // Register agent-corpus mapping (#798).
                if let Err(e) = agent_state
                    .db
                    .register_agent_corpus(
                        agent_name,
                        docs_root_hash,
                        &docs_root.display().to_string(),
                    )
                    .await
                {
                    warn!(
                        error = %e,
                        agent = agent_name.as_str(),
                        docs_root_hash = docs_root_hash.as_str(),
                        "failed to register agent corpus mapping"
                    );
                }

                // Drift WARN: first-run corpus with no prior ingestion (#778 R9).
                match agent_state
                    .db
                    .count_chunks_for_docs_root_hash(docs_root_hash)
                    .await
                {
                    Ok(0) => {
                        warn!(
                            agent = agent_name.as_str(),
                            docs_root = %docs_root.display(),
                            docs_root_hash = docs_root_hash.as_str(),
                            corpus_index = corpus_idx,
                            corpus_count = corpora.len(),
                            "agent docs_root_hash has no matching rows in shared corpus \
                             — first-run ingestion will populate"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!(
                            agent = agent_name.as_str(),
                            error = %e,
                            "failed to check corpus drift; continuing with ingestion"
                        );
                    }
                }

                let ingestor = crate::kg::lexical_ingestor::LexicalIngestor::new(
                    agent_state.db.clone(),
                    docs_root.clone(),
                    None,
                );
                match ingestor.ingest_all().await {
                    Ok(stats) => info!(
                        event = "lexical_ingest_complete",
                        agent_id = %agent_name,
                        docs_root = %docs_root.display(),
                        docs_root_hash = docs_root_hash.as_str(),
                        corpus_index = corpus_idx,
                        corpus_count = corpora.len(),
                        docs_scanned = stats.docs_scanned,
                        docs_ingested = stats.docs_ingested,
                        docs_skipped = stats.docs_skipped_unchanged,
                        docs_pruned = stats.docs_pruned,
                        chunks_added = stats.chunks_added,
                        duration_ms = stats.duration_ms,
                        "lexical ingestion ready"
                    ),
                    Err(e) => warn!(
                        error = %e,
                        agent_id = %agent_name,
                        "lexical ingestion failed; chunks may be stale until next restart"
                    ),
                }
            }
        }

        // Subject graph extraction: LLM-based NER + fact triples from
        // previously-ingested chunks (#690). Runs asynchronously in the
        // background after lexical ingestion — does not block server readiness.
        // Per D2: startup extraction spawns one background task per agent.
        //
        // The extraction LLM is also passed to the periodic tick (#1052) so
        // corpora that don't fully drain at startup get 48 more extraction
        // opportunities per day.
        let kg_batch_budget = settings.effective_kg_batch_budget();
        let extraction_llm_for_tick: Option<Arc<dyn mika_common::llm::LlmProvider>> =
            match settings.make_kg_extraction_provider() {
                Some(Ok(llm)) => Some(llm),
                Some(Err(e)) => {
                    warn!(
                        error = %e,
                        event = "extraction_llm_disabled",
                        "failed to create KG extraction provider — extraction disabled"
                    );
                    None
                }
                None => None,
            };
        match extraction_llm_for_tick.clone() {
            Some(extraction_llm) => {
                // #757 R4: advise operators when KG extraction resolves to
                // Anthropic (expensive for bulk NER). Fires once per startup.
                if extraction_llm
                    .provider_name()
                    .eq_ignore_ascii_case("anthropic")
                {
                    warn!(
                        event = "kg_anthropic_provider",
                        scope = "extraction",
                        provider = "anthropic",
                        model = %extraction_llm.model_name(),
                        "KG extraction is using Anthropic — typically ~10× more expensive than \
                         OpenRouter equivalents for bulk NER. Consider \
                         MIKA_KG_EXTRACTION_MODEL=openrouter/<model> for cost-sensitive deployments."
                    );
                }
                for entry in agents.iter() {
                    let agent_name = entry.key();
                    let agent_state = entry.value();
                    let KgAgentConfig::Enabled { ref corpora } = agent_state.kg_config else {
                        info!(
                            agent = agent_name.as_str(),
                            event = "extraction_disabled",
                            reason = ?agent_state.kg_config,
                            "KG subject extraction disabled"
                        );
                        continue;
                    };

                    // Per-corpus fair budget allocation (#962) — sibling of resolver fairness (#927).
                    let db = agent_state.db.clone();
                    let llm = extraction_llm.clone();
                    let agent_name_clone = agent_name.clone();
                    let budget = kg_batch_budget;
                    let corpora_clone: Vec<_> =
                        corpora.iter().map(|c| c.docs_root.clone()).collect();
                    let corpus_count = corpora_clone.len();

                    let extraction_agent = agent_name_clone.clone();
                    let handle = tokio::spawn(async move {
                        // Phase 1: Count pending docs per corpus.
                        let mut corpus_pending: Vec<u32> = Vec::new();
                        let mut extractors: Vec<crate::kg::subject_extractor::SubjectExtractor> =
                            Vec::new();
                        for docs_root in &corpora_clone {
                            let extractor = crate::kg::subject_extractor::SubjectExtractor::new(
                                db.clone(),
                                llm.clone(),
                                docs_root.clone(),
                                None,
                            );
                            let count = extractor.count_pending_docs().await.unwrap_or(0);
                            corpus_pending.push(count);
                            extractors.push(extractor);
                        }

                        // Phase 2: Fair budget allocation via shared function.
                        let allocated =
                            crate::kg::budget::allocate_fair_budget(&corpus_pending, budget);

                        // Phase 3: Execute extractors with per-corpus budgets.
                        let mut per_corpus_extracted: std::collections::BTreeMap<String, u32> =
                            std::collections::BTreeMap::new();
                        let mut total_extracted: usize = 0;
                        let mut total_entities: usize = 0;
                        let mut total_relationships: usize = 0;
                        let mut total_failed: usize = 0;
                        let mut total_duration_ms: u64 = 0;

                        for (idx, (extractor, per_budget)) in
                            extractors.into_iter().zip(allocated.iter()).enumerate()
                        {
                            if *per_budget == 0 {
                                continue;
                            }
                            match extractor.extract_pending(*per_budget).await {
                                Ok(stats) => {
                                    let corpus_key = corpora_clone[idx].display().to_string();
                                    per_corpus_extracted
                                        .insert(corpus_key, stats.docs_extracted as u32);
                                    total_extracted += stats.docs_extracted;
                                    total_entities += stats.total_entities;
                                    total_relationships += stats.total_relationships;
                                    total_failed += stats.docs_failed;
                                    total_duration_ms += stats.duration_ms;
                                }
                                Err(e) => warn!(
                                    error = %e,
                                    agent_id = %agent_name_clone,
                                    corpus_index = idx,
                                    "subject extraction failed for corpus; entities may be stale until next restart"
                                ),
                            }
                        }

                        if total_extracted > 0 || total_failed > 0 {
                            info!(
                                event = "subject_extraction_ready",
                                agent_id = %agent_name_clone,
                                corpus_count = corpus_count,
                                per_corpus_extracted = %serde_json::to_string(&per_corpus_extracted).unwrap_or_default(),
                                total_docs_extracted = total_extracted,
                                total_docs_failed = total_failed,
                                total_entities = total_entities,
                                total_relationships = total_relationships,
                                total_duration_ms = total_duration_ms,
                                "subject extraction complete"
                            );
                        }
                    });
                    tokio::spawn(async move {
                        match handle.await {
                            Ok(()) => {}
                            Err(e) if e.is_panic() => {
                                let payload = e.into_panic();
                                let msg = payload
                                    .downcast_ref::<&str>()
                                    .map(|s| (*s).to_string())
                                    .or_else(|| payload.downcast_ref::<String>().cloned())
                                    .unwrap_or_else(|| "<non-string panic>".to_string());
                                error!(
                                    panic_message = %msg,
                                    agent_id = %extraction_agent,
                                    event = "extraction_panicked",
                                    "subject extraction task panicked"
                                );
                            }
                            Err(e) if e.is_cancelled() => {
                                debug!(
                                    agent_id = %extraction_agent,
                                    event = "extraction_cancelled",
                                    "subject extraction task was cancelled"
                                );
                            }
                            Err(e) => warn!(
                                error = %e,
                                agent_id = %extraction_agent,
                                event = "extraction_join_error",
                                "subject extraction task join error"
                            ),
                        }
                    });
                }
            }
            None => {
                info!(
                    event = "extraction_disabled",
                    reason = "no extraction model configured",
                    "subject extraction disabled — set MIKA_KG_EXTRACTION_MODEL or MIKA_KG_INGESTION_MODEL to enable"
                );
            }
        }

        // Entity resolution: resolve pending subject entities to domain graph
        // nodes (#691). Runs after extraction to catch both newly-extracted
        // entities and previously-pending ones. Spawns one background task per
        // agent. Resolution LLM is optional — without it, exact-match-only mode.
        {
            let resolution_llm = match settings.make_kg_resolution_provider() {
                Some(Ok(llm)) => {
                    // #757 R4: advise when resolution resolves to Anthropic.
                    if llm.provider_name().eq_ignore_ascii_case("anthropic") {
                        warn!(
                            event = "kg_anthropic_provider",
                            scope = "resolution",
                            provider = "anthropic",
                            model = %llm.model_name(),
                            "KG resolution is using Anthropic — consider \
                             MIKA_KG_RESOLUTION_MODEL=openrouter/<model> for cost-sensitive deployments."
                        );
                    }
                    Some(llm)
                }
                Some(Err(e)) => {
                    warn!(
                        error = %e,
                        event = "resolution_llm_disabled",
                        "failed to create KG resolution provider — exact-match-only mode"
                    );
                    None
                }
                None => None,
            };

            for entry in agents.iter() {
                let agent_name = entry.key();
                let agent_state = entry.value();
                let KgAgentConfig::Enabled { ref corpora } = agent_state.kg_config else {
                    info!(
                        agent = agent_name.as_str(),
                        event = "resolution_disabled",
                        reason = ?agent_state.kg_config,
                        "KG entity resolution disabled"
                    );
                    continue;
                };

                // Gather all docs_root_hash values for this agent's corpora (#798).
                let hashes: Vec<String> =
                    corpora.iter().map(|c| c.docs_root_hash.clone()).collect();

                let db = agent_state.db.clone();
                let llm = resolution_llm.clone();
                let agent_name_clone = agent_name.clone();
                let budget = kg_batch_budget;

                let resolution_agent = agent_name_clone.clone();
                let handle = tokio::spawn(async move {
                    // Small delay to let extraction tasks commit first.
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                    let resolver = crate::kg::entity_resolver::SubjectEntityResolver::new(
                        db, llm, hashes, None,
                    );
                    match resolver.resolve_pending(budget).await {
                        Ok(stats) => {
                            if stats.matched_exact > 0 || stats.matched_llm > 0 || stats.errors > 0
                            {
                                info!(
                                    event = "entity_resolution_ready",
                                    agent_id = %agent_name_clone,
                                    matched_exact = stats.matched_exact,
                                    matched_llm = stats.matched_llm,
                                    no_match = stats.no_match,
                                    skipped_discovered = stats.skipped_discovered,
                                    skipped_no_llm = stats.skipped_no_llm,
                                    errors = stats.errors,
                                    duration_ms = stats.duration_ms,
                                    "entity resolution complete"
                                );
                            }
                        }
                        Err(e) => warn!(
                            error = %e,
                            agent_id = %agent_name_clone,
                            "entity resolution failed; resolutions may be stale until next restart"
                        ),
                    }
                });
                tokio::spawn(async move {
                    match handle.await {
                        Ok(()) => {}
                        Err(e) if e.is_panic() => {
                            let payload = e.into_panic();
                            let msg = payload
                                .downcast_ref::<&str>()
                                .map(|s| (*s).to_string())
                                .or_else(|| payload.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "<non-string panic>".to_string());
                            error!(
                                panic_message = %msg,
                                agent_id = %resolution_agent,
                                event = "resolution_panicked",
                                "entity resolution task panicked"
                            );
                        }
                        Err(e) if e.is_cancelled() => {
                            debug!(
                                agent_id = %resolution_agent,
                                event = "resolution_cancelled",
                                "entity resolution task was cancelled"
                            );
                        }
                        Err(e) => warn!(
                            error = %e,
                            agent_id = %resolution_agent,
                            event = "resolution_join_error",
                            "entity resolution task join error"
                        ),
                    }
                });

                // Periodic extraction + resolution tick (#906, #1052): drains
                // both extraction and Stage-2 resolution backlogs every 30 min
                // at the existing budget, decoupling drain rate from restart
                // cadence. The startup spawn above handles the immediate
                // post-restart drain; this tick handles steady-state.
                let kg_tick_handle = crate::kg::resolver_tick::spawn_resolver_tick_task(
                    agent_name.clone(),
                    agent_state.db.clone(),
                    extraction_llm_for_tick.clone(),
                    resolution_llm.clone(),
                    &agent_state.kg_config,
                    kg_batch_budget,
                    None, // default 30-min interval
                    kg_shutdown_token.child_token(),
                );
                kg_tick_handles.push(kg_tick_handle);
            }
        }
    }

    let state = AppState {
        agents: Arc::new(agents),
        default_agent,
        tools: tool_registry,
        ready: ready.clone(),
        internal_token,
        dashboard_token,
        gateway_url,
        startup_time: std::time::Instant::now(),
        http_client,
        brave_api_key: settings
            .brave_api_key
            .as_ref()
            .map(|s| s.expose_secret().to_string()),
        github_token: settings.agent_github_token().map(String::from),
        github_app: global_github_app,
        global_home_dir: global_home.to_path_buf(),
        settings: settings.clone(),
        dashboard_db,
        investigation_lock: Arc::new(tokio::sync::Mutex::new(())),
        investigation_tools: Arc::new(tokio::sync::OnceCell::new()),
        dashboard_enabled: Arc::new(AtomicBool::new(settings.dashboard_enabled)),
        a2a_broadcasters: Arc::new(dashmap::DashMap::new()),
        pr_reviews_posted,
        rate_limit_audit_last: Arc::new(dashmap::DashMap::new()),
    };

    let app = build_router(state.clone());

    let port = settings.spirit_port;
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    info!(port, "mika-spirit listening");

    ready.store(true, Ordering::Release);

    // Cancel orphan recurring tasks for agents no longer on disk (mika#1436).
    // Runs against the startup-time agent set before per-agent recurring-task setup.
    // #1399's lazy-insert path runs later and adds tasks via ensure_recurring_task
    // when the agent is first exercised — no conflict.
    {
        let known_agent_ids: Vec<String> = state.agents.iter().map(|e| e.key().clone()).collect();
        if !known_agent_ids.is_empty() {
            let db = state.dashboard_db.clone();
            match db
                .with_db(move |db| db.cancel_orphan_recurring_tasks(&known_agent_ids))
                .await
            {
                Ok(orphans) if !orphans.is_empty() => {
                    let agent_ids: Vec<&str> = orphans.iter().map(|(_, a)| a.as_str()).collect();
                    info!(
                        count = orphans.len(),
                        agent_ids = ?agent_ids,
                        "cancelled orphan recurring tasks for agents no longer on disk"
                    );
                }
                Ok(_) => {} // no orphans, no log noise
                Err(e) => warn!(error = %e, "orphan recurring task sweep failed"),
            }
        } else {
            warn!(
                "startup-time agent set is empty; skipping orphan recurring task sweep as safety measure"
            );
        }
    }

    // Start task engines for all agents:
    // 1. Register recurring tasks (heartbeat, reflection) if missing
    // 2. Run startup recovery (expire/recover in_progress, load heap)
    // 3. Spawn 1-second tick loop — collect JoinHandle for shutdown abort
    // 4. Spawn background cleanup (pruning, vacuum, backfill) as a fire-and-forget task
    let mut tick_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let mut total_tasks_loaded: usize = 0;

    for entry in state.agents.iter() {
        let name = entry.key();
        let agent_state = entry.value();
        let engine = agent_state.task_engine.clone();
        let db = agent_state.db.clone();
        let agent_name = name.clone();

        // Register recurring built-in tasks
        if task_engine::heartbeat_enabled_for_agent(&agent_state.home_dir).await {
            task_engine::ensure_recurring_task(
                &db,
                "heartbeat",
                HEARTBEAT_CRON,
                r#"{"trigger":"heartbeat"}"#,
            )
            .await;
        } else if let Err(e) = db.cancel_recurring_task_by_label("heartbeat").await {
            warn!(agent = %agent_name, error = %e, "failed to cancel stale heartbeat task");
        }
        if let Some(cron) = task_engine::reflection_cron_for_agent(&agent_state.home_dir, &db).await
        {
            task_engine::ensure_recurring_task(
                &db,
                "reflection",
                &cron,
                r#"{"trigger":"reflection"}"#,
            )
            .await;
        } else if let Err(e) = db.cancel_recurring_task_by_label("reflection").await {
            warn!(agent = %agent_name, error = %e, "failed to cancel stale reflection task");
        }

        // Register auto-pull recurring task for mika-dev only (mika#1363, AC4).
        if name == "mika-dev" {
            if std::env::var("MIKA_DEV_AUTO_PULL")
                .map(|v| v == "0")
                .unwrap_or(false)
            {
                info!(agent = %name, "auto_pull disabled via MIKA_DEV_AUTO_PULL=0");
                // Cancel any existing auto_pull task if the feature was just disabled
                if let Err(e) = db.cancel_recurring_task_by_label("auto_pull_groomed").await {
                    warn!(agent = %name, error = %e, "failed to cancel stale auto_pull_groomed task");
                }
            } else {
                task_engine::ensure_recurring_task(
                    &db,
                    "auto_pull_groomed",
                    AUTO_PULL_CRON,
                    r#"{"trigger":"auto_pull_groomed"}"#,
                )
                .await;
            }
        }

        // Register curator review recurring task (mika#1584).
        {
            let identity = crate::prompt::load_identity_async(&agent_state.home_dir).await;
            let curator_cron = identity
                .curator
                .as_ref()
                .and_then(|c| c.interval_hours)
                .map(|h| format!("0 0 */{h} * * *"))
                .unwrap_or_else(|| CURATOR_REVIEW_CRON.to_string());
            task_engine::ensure_recurring_task(
                &db,
                "curator_review",
                &curator_cron,
                r#"{"trigger":"curator_review"}"#,
            )
            .await;
        }

        // Startup recovery (expire timed-out tasks, mark orphaned in_progress as failed, load heap)
        {
            let mut eng = engine.lock().await;
            match eng.startup_recovery().await {
                Ok((loaded, _queue_len)) => {
                    total_tasks_loaded += loaded;
                }
                Err(e) => {
                    warn!(agent = %agent_name, error = %e, "task engine startup recovery failed");
                }
            }
        }

        // Prune completed tasks older than 30 days to prevent unbounded DB growth
        task_engine::prune_old_tasks(&db).await;

        // Spawn tick loop and retain the handle so we can abort it on shutdown
        let handle = TaskEngine::spawn_tick_loop(engine);
        tick_handles.push(handle);

        // Background cleanup runs concurrently — fire and forget
        tokio::spawn(startup_cleanup(db, agent_state.embedding_client.clone()));
    }

    let agent_count = state.agents.len();
    info!(
        total = total_tasks_loaded,
        agents = agent_count,
        "tasks loaded"
    );

    if is_pretty {
        mika_common::logging::print_ready();
    } else {
        info!("server ready");
    }

    // Serve with graceful shutdown; cancel KG tasks cooperatively first (#802),
    // then abort all remaining tick loops.
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            // Signal KG background tasks to stop cooperatively. They check
            // this between iterations and between per-entity LLM calls,
            // so worst-case latency is one LLM call (~1-2s).
            kg_shutdown_token.cancel();
            for handle in &tick_handles {
                handle.abort();
            }
            for handle in &kg_tick_handles {
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

    fn test_settings() -> Settings {
        let tmp = tempfile::tempdir().unwrap();
        Settings::load(tmp.path()).unwrap()
    }

    fn test_task_engine(
        db: AsyncDatabase,
    ) -> (Arc<tokio::sync::Mutex<TaskEngine>>, Arc<TaskDispatcher>) {
        let llm = mika_common::llm::dummy_provider();
        let dispatcher = Arc::new(TaskDispatcher {
            db: db.clone(),
            llm,
            tools: Arc::new(tools::default_tools()),
            skills: Arc::new(SkillRegistry::empty()),
            message_sender: None,
            home_dir: std::path::PathBuf::from("/tmp/mika-test"),
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            github_app: None,
            skills_dirty: Arc::new(AtomicBool::new(false)),
            agent_lock: None,
            cli_mode: false,
            settings: test_settings(),
            pr_reviews_posted: None,
        });
        let engine = Arc::new(tokio::sync::Mutex::new(TaskEngine::new(
            db,
            dispatcher.clone(),
        )));
        (engine, dispatcher)
    }

    fn test_state() -> AppState {
        let db = test_async_db();
        let dashboard_db = db.clone();
        let llm = mika_common::llm::dummy_provider();
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
            settings: test_settings(),
            llm: llm.clone(),
            github_app: None,
            webhook_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            kg_config: crate::kg::config::KgAgentConfig::Disabled {
                reason: crate::kg::config::DisabledReason::OperatorOptOut,
            },
        };

        let agents = dashmap::DashMap::new();
        agents.insert("mika".to_string(), Arc::new(agent_state));

        AppState {
            agents: Arc::new(agents),
            default_agent: "mika".to_string(),
            tools: tools_reg,
            ready: Arc::new(AtomicBool::new(false)),
            internal_token: SecretString::from("test-token-secret"),
            dashboard_token: None,
            gateway_url: "http://localhost:9999".to_string(),
            startup_time: std::time::Instant::now(),
            http_client: reqwest::Client::new(),
            brave_api_key: None,
            github_token: None,
            github_app: None,
            global_home_dir: std::path::PathBuf::from("/tmp/mika-test"),
            settings: test_settings(),
            dashboard_db,
            investigation_lock: Arc::new(tokio::sync::Mutex::new(())),
            investigation_tools: Arc::new(tokio::sync::OnceCell::new()),
            dashboard_enabled: Arc::new(AtomicBool::new(false)),
            a2a_broadcasters: Arc::new(dashmap::DashMap::new()),
            pr_reviews_posted: Arc::new(dashmap::DashMap::new()),
            rate_limit_audit_last: Arc::new(dashmap::DashMap::new()),
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

        let agent_state = state.agents.get("mika").unwrap().value().clone();
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
    async fn test_rate_limit_trip_emits_audit_event() {
        // mika#1710 AC3: the busy-lock 429 path emits a `rate_limit_trip` audit
        // event so the trip is visible to the orchestrator.
        let state = test_state();
        state.ready.store(true, Ordering::Release);

        let agent_state = state.agents.get("mika").unwrap().value().clone();
        let db = agent_state.db.clone();
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
                        r#"{"text":"hi","chat_id":123,"channel":"telegram","request_id":"trip-1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let events = db.get_audit_events("system").await.unwrap();
        let trips: Vec<_> = events
            .iter()
            .filter(|e| e.tool_name == "rate_limit_trip")
            .collect();
        assert_eq!(
            trips.len(),
            1,
            "exactly one rate_limit_trip audit row expected"
        );
        assert_eq!(trips[0].target_key, "agent:mika");
        assert_eq!(trips[0].after_value.as_deref(), Some("request_id=trip-1"));
    }

    #[tokio::test]
    async fn test_rate_limit_trip_audit_throttled() {
        // mika#1710 AC3 volume guard: N rapid 429s within the interval emit at most
        // one audit row per agent — the audit write must not itself become a flood.
        let state = test_state();
        state.ready.store(true, Ordering::Release);

        let agent_state = state.agents.get("mika").unwrap().value().clone();
        let db = agent_state.db.clone();
        let _guard = agent_state.agent_lock.clone().lock_owned().await;

        let app = test_app(state);
        for i in 0..3 {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/message")
                        .header("content-type", "application/json")
                        .header("authorization", "Bearer test-token-secret")
                        .body(Body::from(format!(
                            r#"{{"text":"hi","chat_id":123,"channel":"telegram","request_id":"trip-{i}"}}"#
                        )))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        }

        let events = db.get_audit_events("system").await.unwrap();
        let trips = events
            .iter()
            .filter(|e| e.tool_name == "rate_limit_trip")
            .count();
        assert_eq!(
            trips, 1,
            "rapid 429s within the throttle interval must emit at most one audit row"
        );
    }

    #[tokio::test]
    async fn test_message_stores_chat_id() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let db = state.agents.get("mika").unwrap().value().db.clone();
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
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
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
        let db = state.agents.get("mika").unwrap().value().db.clone();
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
        let db = state.agents.get("mika").unwrap().value().db.clone();
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
        let db = state.agents.get("mika").unwrap().value().db.clone();
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
        let db = state.agents.get("mika").unwrap().value().db.clone();

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
                next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
                timeout_at: None,
                action_type: "send_message".to_string(),
                action_config: r#"{"text":"hi"}"#.to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: None,
                dispatch_class: None,
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

    // ===== Dashboard endpoint tests =====

    fn test_state_with_dashboard_token() -> AppState {
        let mut state = test_state();
        state.dashboard_token = Some(SecretString::from("dashboard-token-secret"));
        state
    }

    #[tokio::test]
    async fn test_timeline_returns_paginated_empty() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/timeline")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 0);
        assert_eq!(json["page"], 1);
        assert_eq!(json["per_page"], 50);
        assert!(json["data"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_timeline_with_pagination_params() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/timeline?page=2&per_page=10")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["page"], 2);
        assert_eq!(json["per_page"], 10);
    }

    #[tokio::test]
    async fn test_timeline_with_filters() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/timeline?agent_id=mika&event_type=message")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_timeline_returns_data_after_message_insert() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let db = state.agents.get("mika").unwrap().value().db.clone();

        // Insert a message so the timeline view has data
        db.save_message("test-session", "user", "hello timeline", None)
            .await
            .unwrap();

        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/timeline")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["total"].as_u64().unwrap() >= 1);
        assert!(!json["data"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_agents_list_returns_agents() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/agents")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Should contain at least the default "mika" agent
        let agents = json.as_array().unwrap();
        assert!(!agents.is_empty());
        let mika = agents.iter().find(|a| a["id"] == "mika");
        assert!(mika.is_some(), "expected 'mika' agent in list");
    }

    #[tokio::test]
    async fn test_sessions_list_returns_paginated() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sessions")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["total"].is_number());
        assert!(json["data"].is_array());
        assert_eq!(json["page"], 1);
    }

    #[tokio::test]
    async fn test_session_detail_not_found() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sessions/nonexistent-session")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_session_detail_found() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        // The test_async_db() creates "test-session" during setup
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sessions/test-session")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], "test-session");
    }

    #[tokio::test]
    async fn test_session_messages_returns_paginated() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let db = state.agents.get("mika").unwrap().value().db.clone();

        db.save_message("test-session", "user", "hi", None)
            .await
            .unwrap();
        db.save_message("test-session", "assistant", "hello!", None)
            .await
            .unwrap();

        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sessions/test-session/messages")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 2);
        let msgs = json["data"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
    }

    #[tokio::test]
    async fn test_trace_detail_returns_empty_for_unknown() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/timeline/trace/00000000000000000000000000000000")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_trace_messages_returns_matching_messages() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let db = state.agents.get("mika").unwrap().value().db.clone();

        let trace_id = "aabb0011ccddeeff0011223344556677";
        db.save_message_with_metadata(
            "test-session",
            "user",
            "traced message",
            None,
            Some(trace_id),
            false,
        )
        .await
        .unwrap();

        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/timeline/trace/{trace_id}/messages"))
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.len(), 1);
        assert_eq!(json[0]["content"], "traced message");
        assert_eq!(json[0]["role"], "user");
    }

    // ===== Auth split tests =====

    #[tokio::test]
    async fn test_dashboard_token_accepted_on_api_routes() {
        let state = test_state_with_dashboard_token();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        // Dashboard token should work on /api/v1/* routes
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/timeline")
                    .header("authorization", "Bearer dashboard-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dashboard_token_accepted_on_agents_route() {
        let state = test_state_with_dashboard_token();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/agents")
                    .header("authorization", "Bearer dashboard-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dashboard_token_accepted_on_sessions_route() {
        let state = test_state_with_dashboard_token();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sessions")
                    .header("authorization", "Bearer dashboard-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dashboard_token_rejected_on_message_route() {
        let state = test_state_with_dashboard_token();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        // Dashboard token should NOT work on /message (internal-only)
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer dashboard-token-secret")
                    .body(Body::from(
                        r#"{"text":"hi","chat_id":123,"channel":"telegram","request_id":"r-dash"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_dashboard_token_rejected_on_task_complete_route() {
        let state = test_state_with_dashboard_token();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        // Dashboard token should NOT work on /tasks/{id}/complete (internal-only)
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tasks/some-id/complete")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer dashboard-token-secret")
                    .body(Body::from(r#"{"result":"done"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_internal_token_accepted_on_api_routes() {
        let state = test_state_with_dashboard_token();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        // Internal token should also work on /api/v1/* routes (superuser)
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/timeline")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_no_token_returns_401_on_api_routes() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/timeline")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_wrong_token_returns_401_on_api_routes() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/agents")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_agent_sessions_returns_paginated() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/agents/mika/sessions")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["total"].is_number());
        assert!(json["data"].is_array());
    }

    #[tokio::test]
    async fn test_agent_audit_returns_paginated() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/agents/mika/audit")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["total"].is_number());
        assert!(json["data"].is_array());
    }

    // ===== Dashboard toggle endpoint tests =====

    #[tokio::test]
    async fn test_dashboard_status_returns_200() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/dashboard/status")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Should have enabled, has_assets, has_token fields
        assert!(json.get("enabled").is_some());
        assert!(json.get("has_assets").is_some());
        assert!(json.get("has_token").is_some());
    }

    #[tokio::test]
    async fn test_dashboard_enable_returns_200() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/dashboard/enable")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dashboard_disable_returns_200() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/dashboard/disable")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dashboard_toggle_with_dashboard_token() {
        let state = test_state_with_dashboard_token();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        // Dashboard token should work on toggle endpoints
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/dashboard/status")
                    .header("authorization", "Bearer dashboard-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dashboard_toggle_without_token_returns_401() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/dashboard/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_dashboard_enable_toggles_state() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        // Start disabled
        assert!(!state.dashboard_enabled.load(Ordering::Relaxed));

        let app = test_app(state.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/dashboard/enable")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(state.dashboard_enabled.load(Ordering::Relaxed));
    }

    // ===== Rewind route tests (after restructuring into nested group) =====

    #[tokio::test]
    async fn test_rewind_preview_returns_401_without_token() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rewind/preview")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"session_id":"test","count":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Without token → 401 (not 404, proving the route exists)
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_rewind_routes_accept_internal_token() {
        let state = test_state();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        // Send a valid rewind resolve request — auth should pass (not 401).
        // The handler returns 404 "No exchanges found" because the session is empty,
        // which proves the route IS matched and auth IS accepted.
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rewind/resolve")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token-secret")
                    .body(Body::from(r#"{"session_id":"test-session","count":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Not 401 — auth passed
        assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
        // The handler returns 404 "No exchanges found" (not a routing 404)
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["error"].as_str().unwrap().contains("No exchanges"),
            "expected handler-level 404, got: {}",
            json
        );
    }

    #[tokio::test]
    async fn test_rewind_rejects_dashboard_token() {
        let state = test_state_with_dashboard_token();
        state.ready.store(true, Ordering::Release);
        let app = test_app(state);

        // Dashboard token should NOT work on rewind routes (internal-only)
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rewind/preview")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer dashboard-token-secret")
                    .body(Body::from(r#"{"session_id":"test","count":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
