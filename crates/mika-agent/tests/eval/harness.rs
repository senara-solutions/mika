//! EvalHarness — builder for running `run_agent()` with a `MockLlmProvider`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use tempfile::TempDir;

use mika_agent::agent::{AgentParams, run_agent};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::skills::SkillRegistry;
use mika_agent::tools::{ToolRegistry, default_tools};
use mika_common::config::Settings;
use mika_common::llm::ProviderKind;
use mika_common::llm::mock::{MockLlmProvider, MockResponse};

use super::trace::AgentTrace;

/// Integration test harness for the agent loop.
///
/// Wraps `run_agent()` with a `MockLlmProvider`, in-memory SQLite, and sensible defaults.
/// Use `EvalHarness::builder()` to configure, then `.run("message")` to execute.
pub struct EvalHarness {
    pub db: AsyncDatabase,
    pub mock_provider: Arc<MockLlmProvider>,
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
}

impl EvalHarness {
    /// Create a new builder.
    pub fn builder() -> EvalHarnessBuilder {
        EvalHarnessBuilder::default()
    }

    /// Run the agent loop with a user message and return the execution trace.
    pub async fn run(&self, message: &str) -> Result<AgentTrace> {
        let params = AgentParams {
            db: &self.db,
            llm: self.mock_provider.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            user_message: message,
            channel_type: "test",
            session_id: &self.session_id,
            home_dir: self.home_dir.path(),
            is_onboarding: self.is_onboarding,
            message_sender: None,
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
            is_callback_turn: self.is_callback_turn,
            settings: Some(&self.settings),
            trace_id: Some(self.trace_id.clone()),
            correlated_task_id: None,
            internal: self.internal,
        };

        let output = run_agent(&params).await?;
        AgentTrace::from_run(&self.db, &self.trace_id, &self.mock_provider, output).await
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

        // Build mock provider
        let mut builder = MockLlmProvider::builder();
        if let Some(name) = self.provider_name {
            builder = builder.provider_name(name);
        }
        if let Some(name) = self.model_name {
            builder = builder.model_name(name);
        }
        let mock_provider = Arc::new(builder.responses(self.responses).build());

        let trace_id = uuid::Uuid::new_v4().as_simple().to_string();

        let settings = dummy_settings();

        Ok(EvalHarness {
            db: async_db,
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
        })
    }
}

/// Minimal Settings for eval tests (mirrors test_utils::dummy_settings).
fn dummy_settings() -> Settings {
    Settings {
        llm_provider: ProviderKind::Anthropic,
        llm_max_tokens: 4096,
        anthropic_model: None,
        anthropic_api_key: None,
        anthropic_base_url: None,
        openai_model: None,
        openai_base_url: None,
        openrouter_model: None,
        openrouter_api_key: None,
        openrouter_base_url: None,
        groq_model: None,
        groq_api_key: None,
        groq_base_url: None,
        ollama_model: None,
        ollama_api_key: None,
        ollama_base_url: None,
        mistral_model: None,
        mistral_api_key: None,
        mistral_base_url: None,
        google_model: None,
        google_api_key: None,
        google_base_url: None,
        deepseek_model: None,
        deepseek_api_key: None,
        deepseek_base_url: None,
        minimax_model: None,
        minimax_api_key: None,
        minimax_base_url: None,
        kimi_model: None,
        kimi_api_key: None,
        kimi_base_url: None,
        qwen_model: None,
        qwen_api_key: None,
        qwen_base_url: None,
        db_path: PathBuf::from("test.db"),
        log_level: "info".to_string(),
        log_format: "json".to_string(),
        routing_url: None,
        customer_id: None,
        server_port: 8080,
        internal_token: None,
        dashboard_token: None,
        openai_api_key: None,
        embedding_model: "text-embedding-3-small".to_string(),
        embedding_dimensions: 512,
        brave_api_key: None,
        github_token: None,
        investigate_github_token: None,
        github_repo: None,
        github_app_id: None,
        github_app_private_key: None,
        github_app_installation_id: None,
        github_app_login: None,
        home_dir: PathBuf::from("/tmp"),
        server_log_file: None,
        dashboard_enabled: false,
        disable_bundled_skills: false,
        telemetry_enabled: false,
        otlp_endpoint: None,
        otlp_auth_header: None,
        store_llm_calls: true,
        store_tool_calls: true,
        log_llm_bodies: false,
    }
}
