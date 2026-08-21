use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use dashmap::DashMap;
use mika_a2a::streaming::StreamEvent;
use mika_common::agent;
use mika_common::config::Settings;
use mika_common::embedding::EmbeddingClient;
use mika_common::github_app::GitHubApp;
use mika_common::home;
use mika_common::llm::LlmProvider;
use secrecy::SecretString;
use tokio::sync::{OnceCell, broadcast};

use crate::async_db::AsyncDatabase;
use crate::kg::config::KgAgentConfig;
use crate::mcp::McpManager;
use crate::server::webhook_queue::DeferredWebhook;
use crate::server::webhook_queue_v2::WebhookQueue;
use crate::skills::SkillRegistry;
use crate::task_engine::{TaskDispatcher, TaskEngine};
use crate::tools::ToolRegistry;

/// Per-agent state bundle. Each agent gets its own DB, skills, task engine, and lock.
/// Always used behind `Arc<AgentState>` — does not need Clone.
pub struct AgentState {
    pub db: AsyncDatabase,
    pub skills: std::sync::Mutex<Arc<SkillRegistry>>,
    pub skills_dirty: Arc<AtomicBool>,
    /// Cross-turn skill-authoring nudge state (mika#1583). Shared across all of
    /// an agent's turn types; threaded into the conversation loop by reference on
    /// `AgentParams` (same pattern as `skills_dirty`). Phase 1 only increments/
    /// injects in conversation mode.
    pub skill_nudge: Arc<crate::agent_loop::skill_nudge::SkillNudgeState>,
    pub task_engine: Arc<tokio::sync::Mutex<TaskEngine>>,
    pub dispatcher: Arc<TaskDispatcher>,
    pub agent_lock: Arc<tokio::sync::Mutex<()>>,
    pub home_dir: PathBuf,
    pub embedding_client: Option<EmbeddingClient>,
    pub mcp_manager: Option<McpManager>,
    /// Per-agent settings loaded via `Settings::load_for_agent(global_home, agent_home)`.
    /// Ensures callback/heartbeat/reflection turns use the agent's LLM provider config,
    /// not the global default. See #323.
    pub settings: Settings,
    /// Per-agent LLM provider built from `self.settings`.
    pub llm: Arc<dyn LlmProvider>,
    /// GitHub App authentication manager (optional). When present, installation
    /// tokens are preferred over `MIKA_GITHUB_TOKEN` PAT for agent operations.
    pub github_app: Option<Arc<GitHubApp>>,
    /// In-memory queue of deferred GitHub webhooks awaiting callback completion (#528).
    /// Webhooks targeting a task with an in-flight callback are held here until
    /// the callback completes or the 60s timeout expires.
    pub webhook_queue: Arc<tokio::sync::Mutex<Vec<DeferredWebhook>>>,
    /// Bounded webhook queue with backpressure + coalescing (mika#1870). The
    /// uniform ingestion queue for `POST /message`; a single per-agent drain
    /// worker is the sole consumer. Distinct from `webhook_queue` above (mika#528
    /// deferral, a different mechanism). Constructed in `init_agent` from
    /// `effective_webhook_queue_*()` config.
    pub webhook_queue_v2: Arc<WebhookQueue>,
    /// Per-agent KG configuration resolved at init time (#778). `Disabled` skips
    /// all KG subsystem construction; `Enabled` provides the validated docs_root
    /// and precomputed docs_root_hash for the three KG startup loops.
    pub kg_config: KgAgentConfig,
    /// Canonical session ID for singleton agents (mika#1401). `Some` when the
    /// agent's `identity.toml` sets `[session] singleton = true` — the `/send`
    /// handler then reuses this one session instead of minting a UUID per message.
    /// `None` for normal agents (default: fresh session per message). Resolved
    /// once at `init_agent` time from the agent's identity.
    pub canonical_session_id: Option<String>,
}

/// Shared application state for the Axum HTTP server.
///
/// All fields are Clone-able (owned or Arc-wrapped) so Axum can share
/// state across handler tasks.
#[derive(Clone)]
pub struct AppState {
    /// Per-agent state, keyed by agent name (e.g. "mika", "work").
    /// Each AgentState is Arc-wrapped so handler clones are a cheap atomic increment
    /// instead of 3 heap allocations (PathBuf + EmbeddingClient strings).
    /// DashMap allows lazy-insertion of agents created after server startup (#1399).
    pub agents: Arc<DashMap<String, Arc<AgentState>>>,
    /// Default agent name (resolved from active_agent file).
    pub default_agent: String,
    pub tools: Arc<ToolRegistry>,
    pub ready: Arc<AtomicBool>,
    pub internal_token: SecretString,
    /// Separate bearer token for read-only dashboard API routes.
    /// If `None`, dashboard routes accept only `internal_token` (backwards compat).
    pub dashboard_token: Option<SecretString>,
    pub gateway_url: String,
    pub startup_time: std::time::Instant,
    pub http_client: reqwest::Client,
    pub brave_api_key: Option<String>,
    pub github_token: Option<String>,
    /// GitHub App authentication manager (optional, shared across agents).
    pub github_app: Option<Arc<GitHubApp>>,
    pub global_home_dir: PathBuf,
    pub settings: Settings,
    /// Unscoped database handle for dashboard API endpoints (cross-agent queries).
    pub dashboard_db: AsyncDatabase,
    /// Serializes investigation agent runs (independent of per-agent locks).
    pub investigation_lock: Arc<tokio::sync::Mutex<()>>,
    /// Lazily initialized investigation tool registry.
    pub investigation_tools: Arc<OnceCell<Arc<ToolRegistry>>>,
    /// Runtime toggle for the embedded dashboard (initialized from `settings.dashboard_enabled`).
    pub dashboard_enabled: Arc<AtomicBool>,
    /// Active A2A task broadcasters for SSE streaming (keyed by task ID).
    pub a2a_broadcasters: Arc<DashMap<String, broadcast::Sender<StreamEvent>>>,
    /// Session-scoped PR review dedup map (#821). Outer key: session_id,
    /// inner set: PR dedup keys. Prevents duplicate `gh pr review` calls
    /// across turns within the same session. Evicted at `end_session()` callsites.
    pub pr_reviews_posted: Arc<DashMap<String, std::collections::HashSet<String>>>,
    /// Throttle state for `rate_limit_trip` audit emission (mika#1710 AC3). Keyed by
    /// agent name → last emit instant. When the per-agent concurrency-1 lock rejects a
    /// message with 429 ("agent busy"), we emit an audit event so the trip is visible
    /// to the orchestrator — but throttled to at most one row per agent per
    /// `RATE_LIMIT_TRIP_AUDIT_INTERVAL` so a flood does not itself write tens of
    /// thousands of audit rows.
    pub rate_limit_audit_last: Arc<DashMap<String, std::time::Instant>>,
    /// Throttle state for the five `webhook_queue_*` audit events (mika#1870 AC5).
    /// Keyed `"{agent}:{action}"` → last emit instant. Same 1/sec/action/agent
    /// throttle shape as `rate_limit_audit_last` so a burst does not itself flood
    /// the audit table.
    pub webhook_queue_audit_last: Arc<DashMap<String, std::time::Instant>>,
    /// Parent cancellation token for the per-agent webhook drain workers
    /// (mika#1870). Stored here so lazy-resolved agents (`resolve_agent`, #1399)
    /// can spawn a worker with a child token. Cancelled at server shutdown
    /// alongside `kg_shutdown_token`.
    pub webhook_queue_shutdown: tokio_util::sync::CancellationToken,
    /// Permission-decision coordination surface (mika#1733 sub-C AC1). Shared
    /// broadcast channel + pending-decision map. See
    /// `crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md § AC1`.
    pub permissions_channel: Arc<super::permissions_stream::PermissionsChannel>,
    /// Task-event live stream broadcast surface (mika#1732). Per-process
    /// broadcast channel carrying `TaskEventFrame` lifecycle events. Wire
    /// only in v1 — the emission-from-transition-sites plumbing lands in a
    /// follow-up ticket. See
    /// `crates/mika-agent/docs/tasks-event-stream-frame-catalog-2026-07-10.md`.
    pub task_events_channel: Arc<super::tasks_stream::TaskEventsChannel>,
}

impl AppState {
    /// Resolve the AgentState for a given agent name.
    /// Falls back to the default agent if the name is empty.
    /// Returns an Arc clone (cheap atomic increment) instead of a reference,
    /// so callers don't need to clone the inner AgentState.
    ///
    /// On cache miss, checks if the agent exists on disk (identity.toml) AND
    /// has a DB row. If so, lazy-constructs the AgentState via `init_agent`
    /// and inserts it into the map (#1399). Subsequent calls hit the fast path.
    pub async fn resolve_agent(&self, name: &str) -> Option<Arc<AgentState>> {
        let effective = if name.is_empty() {
            &self.default_agent
        } else {
            name
        };
        let normalized = agent::normalize_agent_name(effective);

        // Fast path: already in the map.
        if let Some(r) = self.agents.get(&normalized) {
            return Some(r.value().clone());
        }

        // Slow path: lazy-construct from disk + DB (#1399).
        let agent_home = home::resolve_agent_home(&self.global_home_dir, &normalized);
        if !agent_home.join("identity.toml").exists() {
            return None;
        }
        self.dashboard_db
            .get_agent_with_stats(&normalized)
            .await
            .ok()
            .flatten()?;

        // Construct via the same factory as startup.
        let embedding_client = self.settings.make_embedding_client();
        match super::init_agent(
            &normalized,
            &agent_home,
            &self.global_home_dir,
            &self.tools,
            &self.gateway_url,
            &self.internal_token,
            &self.http_client,
            embedding_client,
            self.brave_api_key.clone(),
            self.settings.disable_bundled_skills,
            self.pr_reviews_posted.clone(),
        )
        .await
        {
            Ok(agent_state) => {
                // mika#1758: attach the per-process task-event broadcast
                // channel BEFORE inserting into the agents map so any
                // lifecycle transition that fires immediately after (e.g.,
                // during webhook-drain-worker spawn below) reaches the wire.
                agent_state
                    .db
                    .set_task_events_channel(self.task_events_channel.clone());
                let agent_state = Arc::new(agent_state);
                self.agents.insert(normalized.clone(), agent_state.clone());
                tracing::info!(
                    agent = normalized.as_str(),
                    event = "agent_resolved_lazily",
                    "lazy-constructed agent state for dashboard access"
                );
                // Spawn the per-agent webhook drain worker for the newly-resolved
                // agent (mika#1870 AC4). Child of the shared parent token, so it
                // is cancelled with the rest at shutdown.
                crate::server::handlers::spawn_webhook_drain_worker(
                    self.clone(),
                    agent_state.clone(),
                    self.webhook_queue_shutdown.child_token(),
                );
                tracing::info!(
                    agent = normalized.as_str(),
                    event = "webhook_queue_worker_spawned",
                    "spawned webhook drain worker for lazily-resolved agent"
                );
                Some(agent_state)
            }
            Err(e) => {
                tracing::warn!(
                    agent = normalized.as_str(),
                    error = %e,
                    "lazy agent construction failed"
                );
                None
            }
        }
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("internal_token", &"[REDACTED]")
            .field(
                "dashboard_token",
                &self.dashboard_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("gateway_url", &self.gateway_url)
            .field("default_agent", &self.default_agent)
            .field(
                "agents",
                &self
                    .agents
                    .iter()
                    .map(|r| r.key().clone())
                    .collect::<Vec<_>>(),
            )
            .field("dashboard_db", &"AsyncDatabase(unscoped)")
            .finish_non_exhaustive()
    }
}
