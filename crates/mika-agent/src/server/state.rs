use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mika_common::claude::ClaudeClient;
use secrecy::SecretString;

use crate::async_db::AsyncDatabase;
use crate::scheduler::ReminderScheduler;
use crate::tools::ToolRegistry;

/// Shared application state for the Axum HTTP server.
///
/// All fields are Clone-able (owned or Arc-wrapped) so Axum can share
/// state across handler tasks.
#[derive(Clone)]
pub struct AppState {
    pub db: AsyncDatabase,
    pub claude: ClaudeClient,
    pub tools: Arc<ToolRegistry>,
    pub scheduler: Arc<ReminderScheduler>,
    pub agent_lock: Arc<tokio::sync::Mutex<()>>,
    pub ready: Arc<AtomicBool>,
    pub internal_token: SecretString,
    pub gateway_url: String,
    pub home_dir: PathBuf,
    pub startup_time: std::time::Instant,
    pub http_client: reqwest::Client,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("internal_token", &"[REDACTED]")
            .field("gateway_url", &self.gateway_url)
            .field("home_dir", &self.home_dir)
            .finish_non_exhaustive()
    }
}
