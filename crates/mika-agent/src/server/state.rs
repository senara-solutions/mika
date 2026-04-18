use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use dashmap::DashMap;
use mika_a2a::streaming::StreamEvent;
use mika_common::agent;
use mika_common::config::Settings;
use mika_common::embedding::EmbeddingClient;
use mika_common::github_app::GitHubApp;
use mika_common::llm::LlmProvider;
use secrecy::SecretString;
use tokio::sync::{OnceCell, broadcast};

use crate::async_db::AsyncDatabase;
use crate::mcp::McpManager;
use crate::server::webhook_queue::DeferredWebhook;
use crate::skills::SkillRegistry;
use crate::task_engine::{TaskDispatcher, TaskEngine};
use crate::tools::ToolRegistry;

/// Per-agent state bundle. Each agent gets its own DB, skills, task engine, and lock.
/// Always used behind `Arc<AgentState>` — does not need Clone.
pub struct AgentState {
    pub db: AsyncDatabase,
    pub skills: std::sync::Mutex<Arc<SkillRegistry>>,
    pub skills_dirty: Arc<AtomicBool>,
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
    pub agents: Arc<HashMap<String, Arc<AgentState>>>,
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
}

impl AppState {
    /// Resolve the AgentState for a given agent name.
    /// Falls back to the default agent if the name is empty.
    /// Returns an Arc clone (cheap atomic increment) instead of a reference,
    /// so callers don't need to clone the inner AgentState.
    pub fn resolve_agent(&self, name: &str) -> Option<Arc<AgentState>> {
        let effective = if name.is_empty() {
            &self.default_agent
        } else {
            name
        };
        let normalized = agent::normalize_agent_name(effective);
        self.agents.get(&normalized).cloned()
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
            .field("agents", &self.agents.keys().collect::<Vec<_>>())
            .field("dashboard_db", &"AsyncDatabase(unscoped)")
            .finish_non_exhaustive()
    }
}
