use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mika_common::agent;
use mika_common::claude::ClaudeClient;
use mika_common::config::Settings;
use mika_common::embedding::EmbeddingClient;
use secrecy::SecretString;

use crate::async_db::AsyncDatabase;
use crate::mcp::McpManager;
use crate::scheduler::ReminderScheduler;
use crate::skills::SkillRegistry;
use crate::tools::ToolRegistry;

/// Per-agent state bundle. Each agent gets its own DB, skills, scheduler, and lock.
/// Always used behind `Arc<AgentState>` — does not need Clone.
pub struct AgentState {
    pub db: AsyncDatabase,
    pub skills: std::sync::Mutex<Arc<SkillRegistry>>,
    pub skills_dirty: Arc<AtomicBool>,
    pub scheduler: Arc<ReminderScheduler>,
    pub agent_lock: Arc<tokio::sync::Mutex<()>>,
    pub home_dir: PathBuf,
    pub embedding_client: Option<EmbeddingClient>,
    pub mcp_manager: Option<McpManager>,
}

/// Shared application state for the Axum HTTP server.
///
/// All fields are Clone-able (owned or Arc-wrapped) so Axum can share
/// state across handler tasks.
#[derive(Clone)]
pub struct AppState {
    /// Per-agent state, keyed by agent name (e.g. "main", "work").
    /// Each AgentState is Arc-wrapped so handler clones are a cheap atomic increment
    /// instead of 3 heap allocations (PathBuf + EmbeddingClient strings).
    pub agents: Arc<HashMap<String, Arc<AgentState>>>,
    /// Default agent name (resolved from active_agent file).
    pub default_agent: String,
    pub claude: ClaudeClient,
    pub tools: Arc<ToolRegistry>,
    pub ready: Arc<AtomicBool>,
    pub internal_token: SecretString,
    pub gateway_url: String,
    pub startup_time: std::time::Instant,
    pub http_client: reqwest::Client,
    pub brave_api_key: Option<String>,
    pub global_home_dir: PathBuf,
    pub settings: Settings,
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
            .field("gateway_url", &self.gateway_url)
            .field("default_agent", &self.default_agent)
            .field("agents", &self.agents.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}
