use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mika_common::claude::ClaudeClient;
use mika_common::embedding::EmbeddingClient;
use secrecy::SecretString;

use crate::async_db::AsyncDatabase;
use crate::scheduler::ReminderScheduler;
use crate::skills::SkillRegistry;
use crate::tools::ToolRegistry;

/// Per-agent state bundle. Each agent gets its own DB, skills, scheduler, and lock.
#[derive(Clone)]
pub struct AgentState {
    pub db: AsyncDatabase,
    pub skills: Arc<SkillRegistry>,
    pub scheduler: Arc<ReminderScheduler>,
    pub agent_lock: Arc<tokio::sync::Mutex<()>>,
    pub home_dir: PathBuf,
    pub embedding_client: Option<EmbeddingClient>,
}

/// Shared application state for the Axum HTTP server.
///
/// All fields are Clone-able (owned or Arc-wrapped) so Axum can share
/// state across handler tasks.
#[derive(Clone)]
pub struct AppState {
    /// Per-agent state, keyed by agent name (e.g. "main", "work").
    pub agents: Arc<HashMap<String, AgentState>>,
    /// Default agent name (resolved from active_agent file).
    pub default_agent: String,
    pub claude: ClaudeClient,
    pub tools: Arc<ToolRegistry>,
    pub ready: Arc<AtomicBool>,
    pub internal_token: SecretString,
    pub gateway_url: String,
    pub startup_time: std::time::Instant,
    pub http_client: reqwest::Client,
}

impl AppState {
    /// Resolve the AgentState for a given agent name.
    /// Falls back to the default agent if the name is empty.
    pub fn resolve_agent(&self, name: &str) -> Option<&AgentState> {
        let effective = if name.is_empty() {
            &self.default_agent
        } else {
            name
        };
        self.agents.get(effective)
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
