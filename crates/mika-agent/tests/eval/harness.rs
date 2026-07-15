//! EvalHarness — builder for running `run_agent()` with a `MockLlmProvider`.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use tempfile::TempDir;

use dashmap::DashMap;
use mika_agent::agent::{AgentParams, run_agent, run_agent_with_deadline};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::mcp::McpManager;
use mika_agent::messaging::MessageSender;
use mika_agent::skills::SkillRegistry;
use mika_agent::tools::{ToolRegistry, default_tools};
use mika_common::config::Settings;
use mika_common::embedding::EmbeddingClient;
use mika_common::llm::LlmProvider;
use mika_common::llm::mock::{MockLlmProvider, MockResponse};
use tokio::time::Instant;

use super::trace::AgentTrace;

/// Integration test harness for the agent loop.
///
/// Wraps `run_agent()` with a `MockLlmProvider`, in-memory SQLite, and sensible defaults.
/// Use `EvalHarness::builder()` to configure, then `.run("message")` to execute.
pub struct EvalHarness {
    pub db: AsyncDatabase,
    /// The LLM provider used for the agent run (mock or real).
    pub llm: Arc<dyn LlmProvider>,
    /// The mock provider, if one was created. `None` when using a real provider.
    /// Used for `captured_requests()` inspection in mock-based tests.
    pub mock_provider: Option<Arc<MockLlmProvider>>,
    pub tools: ToolRegistry,
    pub skills: SkillRegistry,
    pub home_dir: TempDir,
    pub session_id: String,
    pub settings: Settings,
    pub trace_id: String,
    skills_dirty: AtomicBool,
    is_onboarding: bool,
    is_callback_turn: bool,
    skip_compaction: bool,
    internal: bool,
    message_sender: Option<Arc<dyn MessageSender>>,
    embedding_client: Option<EmbeddingClient>,
    brave_api_key: Option<String>,
    github_token: Option<String>,
    mcp_manager: Option<McpManager>,
    /// Session-scoped PR review dedup map (#821, #736).
    /// When `Some`, enables the session-scope dedup guard in the agent loop.
    pub pr_reviews_posted: Option<Arc<DashMap<String, HashSet<String>>>>,
}

impl EvalHarness {
    /// Create a new builder.
    pub fn builder() -> EvalHarnessBuilder {
        EvalHarnessBuilder::default()
    }

    /// Get a reference to the mock provider, panicking if using a real provider.
    ///
    /// Used by tests that need to call `clear_and_set()` or `captured_requests()`.
    pub fn mock(&self) -> &MockLlmProvider {
        self.mock_provider
            .as_ref()
            .expect("mock() called on EvalHarness with a real provider")
    }

    /// Run the agent loop with a user message and return the execution trace.
    pub async fn run(&self, message: &str) -> Result<AgentTrace> {
        let params = AgentParams {
            db: &self.db,
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            user_message: message,
            channel_type: "test",
            session_id: &self.session_id,
            home_dir: self.home_dir.path(),
            is_onboarding: self.is_onboarding,
            message_sender: self.message_sender.clone(),
            skip_compaction: self.skip_compaction,
            embedding_client: self.embedding_client.as_ref(),
            thinking: None,
            user_images: &[],
            brave_api_key: self.brave_api_key.as_deref(),
            github_token: self.github_token.as_deref(),
            github_app: None,
            skills_dirty: &self.skills_dirty,
            mcp_manager: self.mcp_manager.as_ref(),
            global_home_dir: None,
            is_callback_turn: self.is_callback_turn,
            settings: Some(&self.settings),
            trace_id: Some(self.trace_id.clone()),
            correlated_task_id: None,
            internal: self.internal,
            pr_reviews_posted: self.pr_reviews_posted.as_ref(),
            stream_tx: None,
        };

        let output = run_agent(&params).await?;
        AgentTrace::from_run(
            &self.db,
            &self.trace_id,
            self.mock_provider.as_deref(),
            output,
        )
        .await
    }

    /// Run the agent with an explicit deadline, exercising the deadline-exceeded
    /// code path without waiting for the production 5-minute budget.
    ///
    /// Pair with `tokio::time::pause()` and `delayed_response` mock entries to
    /// simulate slow provider calls under virtual time. See mika#848 and the
    /// `deadline_in_flight_llm_call` eval scenario for the canonical pattern.
    pub async fn run_with_deadline(&self, message: &str, deadline: Instant) -> Result<AgentTrace> {
        let params = AgentParams {
            db: &self.db,
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            user_message: message,
            channel_type: "test",
            session_id: &self.session_id,
            home_dir: self.home_dir.path(),
            is_onboarding: self.is_onboarding,
            message_sender: self.message_sender.clone(),
            skip_compaction: self.skip_compaction,
            embedding_client: self.embedding_client.as_ref(),
            thinking: None,
            user_images: &[],
            brave_api_key: self.brave_api_key.as_deref(),
            github_token: self.github_token.as_deref(),
            github_app: None,
            skills_dirty: &self.skills_dirty,
            mcp_manager: self.mcp_manager.as_ref(),
            global_home_dir: None,
            is_callback_turn: self.is_callback_turn,
            settings: Some(&self.settings),
            trace_id: Some(self.trace_id.clone()),
            correlated_task_id: None,
            internal: self.internal,
            pr_reviews_posted: self.pr_reviews_posted.as_ref(),
            stream_tx: None,
        };

        let output = run_agent_with_deadline(&params, deadline).await?;
        AgentTrace::from_run(
            &self.db,
            &self.trace_id,
            self.mock_provider.as_deref(),
            output,
        )
        .await
    }

    /// Run a subsequent turn on the same session with fresh mock responses.
    ///
    /// Uses `MockLlmProvider::clear_and_set()` to replace the response sequence.
    /// Panics if the harness was created with a real provider (no mock to reset).
    pub async fn run_turn(
        &self,
        message: &str,
        responses: Vec<MockResponse>,
    ) -> Result<AgentTrace> {
        let mock = self
            .mock_provider
            .as_ref()
            .expect("run_turn requires a mock provider — cannot be used with real providers");
        mock.clear_and_set(responses);

        // Generate a new trace_id for the new turn so DB queries are scoped
        let turn_trace_id = uuid::Uuid::new_v4().as_simple().to_string();

        let params = AgentParams {
            db: &self.db,
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            user_message: message,
            channel_type: "test",
            session_id: &self.session_id,
            home_dir: self.home_dir.path(),
            is_onboarding: false,
            message_sender: self.message_sender.clone(),
            skip_compaction: self.skip_compaction,
            embedding_client: None,
            thinking: None,
            user_images: &[],
            brave_api_key: None,
            github_token: None,
            github_app: None,
            skills_dirty: &self.skills_dirty,
            mcp_manager: None,
            global_home_dir: None,
            is_callback_turn: false,
            settings: Some(&self.settings),
            trace_id: Some(turn_trace_id.clone()),
            correlated_task_id: None,
            internal: self.internal,
            pr_reviews_posted: self.pr_reviews_posted.as_ref(),
            stream_tx: None,
        };

        let output = run_agent(&params).await?;
        AgentTrace::from_run(&self.db, &turn_trace_id, Some(mock.as_ref()), output).await
    }
}

/// Builder for `EvalHarness`.
pub struct EvalHarnessBuilder {
    responses: Vec<MockResponse>,
    tools: Option<ToolRegistry>,
    skills: Option<SkillRegistry>,
    session_id: Option<String>,
    is_onboarding: bool,
    is_callback_turn: bool,
    skip_compaction: bool,
    internal: bool,
    provider_name: Option<String>,
    model_name: Option<String>,
    message_sender: Option<Arc<dyn MessageSender>>,
    /// When set, uses this real provider instead of creating a MockLlmProvider.
    real_llm_provider: Option<Arc<dyn LlmProvider>>,
    embedding_client: Option<EmbeddingClient>,
    brave_api_key: Option<String>,
    github_token: Option<String>,
    mcp_manager: Option<McpManager>,
    pr_reviews_posted: Option<Arc<DashMap<String, HashSet<String>>>>,
}

impl Default for EvalHarnessBuilder {
    fn default() -> Self {
        Self {
            responses: Vec::new(),
            tools: None,
            skills: None,
            session_id: None,
            is_onboarding: false,
            is_callback_turn: false,
            skip_compaction: true, // Default: skip compaction to simplify mock sequences
            internal: false,
            provider_name: None,
            model_name: None,
            message_sender: None,
            real_llm_provider: None,
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            mcp_manager: None,
            pr_reviews_posted: None,
        }
    }
}

impl EvalHarnessBuilder {
    /// Set the mock LLM response sequence (required).
    pub fn responses(mut self, responses: Vec<MockResponse>) -> Self {
        self.responses = responses;
        self
    }

    /// Set a custom tool registry. Default: `default_tools()`.
    pub fn tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set a custom skill registry. Default: `SkillRegistry::empty()`.
    pub fn skills(mut self, skills: SkillRegistry) -> Self {
        self.skills = Some(skills);
        self
    }

    /// Set a custom session ID. Default: UUID.
    pub fn session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }

    /// Set onboarding mode. Default: `false`.
    pub fn onboarding(mut self, v: bool) -> Self {
        self.is_onboarding = v;
        self
    }

    /// Set callback turn mode. Default: `false`.
    pub fn callback_turn(mut self, v: bool) -> Self {
        self.is_callback_turn = v;
        self
    }

    /// Set whether to skip compaction. Default: `true`.
    pub fn skip_compaction(mut self, v: bool) -> Self {
        self.skip_compaction = v;
        self
    }

    /// Set internal message tagging. Default: `false`.
    pub fn internal(mut self, v: bool) -> Self {
        self.internal = v;
        self
    }

    /// Set the mock provider name. Default: `"mock"`.
    pub fn provider_name(mut self, name: impl Into<String>) -> Self {
        self.provider_name = Some(name.into());
        self
    }

    /// Set the mock model name. Default: `"mock-model"`.
    pub fn model_name(mut self, name: impl Into<String>) -> Self {
        self.model_name = Some(name.into());
        self
    }

    /// Set a custom message sender. Default: `None`.
    pub fn message_sender(mut self, sender: Arc<dyn MessageSender>) -> Self {
        self.message_sender = Some(sender);
        self
    }

    /// Use a real LLM provider instead of a mock. When set, `responses()` is ignored.
    ///
    /// Use this for real-API matrix tests. `captured_requests()` will be unavailable.
    pub fn llm_provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.real_llm_provider = Some(provider);
        self
    }

    /// Set an embedding client for Layer 3 hybrid search testing. Default: `None`.
    pub fn embedding_client(mut self, client: EmbeddingClient) -> Self {
        self.embedding_client = Some(client);
        self
    }

    /// Set a Brave Search API key for `web_search` tool testing. Default: `None`.
    pub fn brave_api_key(mut self, key: impl Into<String>) -> Self {
        self.brave_api_key = Some(key.into());
        self
    }

    /// Set a GitHub token for GitHub tool testing. Default: `None`.
    pub fn github_token(mut self, token: impl Into<String>) -> Self {
        self.github_token = Some(token.into());
        self
    }

    /// Set an MCP manager for MCP tool testing. Default: `None`.
    pub fn mcp_manager(mut self, mgr: McpManager) -> Self {
        self.mcp_manager = Some(mgr);
        self
    }

    /// Set a session-scoped PR review dedup map (#821, #736). Default: `None`.
    ///
    /// When set, enables the session-scope dedup guard in the agent loop,
    /// which prevents duplicate PR reviews across turns within the same session.
    pub fn pr_reviews_posted(mut self, map: Arc<DashMap<String, HashSet<String>>>) -> Self {
        self.pr_reviews_posted = Some(map);
        self
    }

    /// Build the harness, creating the in-memory DB and temp directories.
    pub async fn build(self) -> Result<EvalHarness> {
        // Create temp directory with minimal agent structure
        let home_dir = TempDir::new()?;

        // Create agent home structure
        let agent_dir = home_dir.path();
        std::fs::create_dir_all(agent_dir.join("skills"))?;
        std::fs::create_dir_all(agent_dir.join("data"))?;
        // Write empty soul.md so load_agent_context doesn't fail
        std::fs::write(agent_dir.join("soul.md"), "")?;

        // Create in-memory DB with session
        let session_id = self
            .session_id
            .unwrap_or_else(|| format!("eval-{}", uuid::Uuid::new_v4()));
        let db = Database::open_in_memory()?;
        db.create_session(&session_id, "mika", "test")?;
        let async_db = AsyncDatabase::new(db);

        // Build provider — either real or mock
        let (llm, mock_provider): (Arc<dyn LlmProvider>, Option<Arc<MockLlmProvider>>) =
            if let Some(real_provider) = self.real_llm_provider {
                (real_provider, None)
            } else {
                let mut builder = MockLlmProvider::builder();
                if let Some(name) = self.provider_name {
                    builder = builder.provider_name(name);
                }
                if let Some(name) = self.model_name {
                    builder = builder.model_name(name);
                }
                let mock = Arc::new(builder.responses(self.responses).build());
                (mock.clone() as Arc<dyn LlmProvider>, Some(mock))
            };

        let trace_id = uuid::Uuid::new_v4().as_simple().to_string();

        let settings = Settings::test_defaults();

        Ok(EvalHarness {
            db: async_db,
            llm,
            mock_provider,
            tools: self.tools.unwrap_or_else(default_tools),
            skills: self.skills.unwrap_or_else(SkillRegistry::empty),
            home_dir,
            session_id,
            settings,
            trace_id,
            skills_dirty: AtomicBool::new(false),
            is_onboarding: self.is_onboarding,
            is_callback_turn: self.is_callback_turn,
            skip_compaction: self.skip_compaction,
            internal: self.internal,
            message_sender: self.message_sender,
            embedding_client: self.embedding_client,
            brave_api_key: self.brave_api_key,
            github_token: self.github_token,
            mcp_manager: self.mcp_manager,
            pr_reviews_posted: self.pr_reviews_posted,
        })
    }
}
