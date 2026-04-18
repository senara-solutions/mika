#[cfg(test)]
pub mod test_helpers {
    use crate::async_db::AsyncDatabase;
    use crate::db::Database;
    use crate::tools::ToolContext;
    use mika_common::config::Settings;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU32};

    /// Create an in-memory database for tests (sync — for db module tests).
    pub fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    /// Create an async database for tool/agent tests.
    /// Pre-creates a "test-session" session for agent "mika" so FK constraints are satisfied.
    pub fn test_async_db() -> AsyncDatabase {
        let db = Database::open_in_memory().unwrap();
        db.create_session("test-session", "mika", "cli").unwrap();
        AsyncDatabase::new(db)
    }

    /// Create a ToolContext for tests (non-onboarding).
    pub fn test_ctx<'a>(db: &'a AsyncDatabase, edit_count: &'a AtomicU32) -> ToolContext<'a> {
        test_ctx_with_onboarding(db, edit_count, false)
    }

    /// Create a ToolContext for tests with configurable onboarding flag.
    pub fn test_ctx_with_onboarding<'a>(
        db: &'a AsyncDatabase,
        edit_count: &'a AtomicU32,
        is_onboarding: bool,
    ) -> ToolContext<'a> {
        static HOME_DIR: &str = "/tmp/mika-test";
        static SKILLS_DIRTY: AtomicBool = AtomicBool::new(false);
        ToolContext {
            db,
            session_id: "test-session",
            trace_id: "00000000000000000000000000000000",
            home_dir: std::path::Path::new(HOME_DIR),
            global_home_dir: None,
            core_memory_edit_count: edit_count,
            is_onboarding,
            message_sender: None,
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            skills_dirty: &SKILLS_DIRTY,
            is_reflection: false,
            is_task_context: false,
            is_callback_turn: false,
            provider_name: "anthropic",
            model_name: "claude-sonnet-4-6",
        }
    }

    /// Test harness that owns the database and edit counter, reducing
    /// boilerplate in tool tests. Use `harness.ctx()` to get a `ToolContext`
    /// and `harness.db` for direct database access during test setup.
    pub struct TestHarness {
        pub db: AsyncDatabase,
        pub counter: AtomicU32,
    }

    impl Default for TestHarness {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TestHarness {
        pub fn new() -> Self {
            Self {
                db: test_async_db(),
                counter: AtomicU32::new(0),
            }
        }

        /// Create a harness with a specific agent ID.
        pub fn with_agent(agent_id: &str) -> Self {
            let db = Database::open_in_memory().unwrap();
            // Ensure the agent exists (default "mika" is seeded by migrate)
            if agent_id != "mika" {
                db.register_agent(agent_id, agent_id, "").unwrap();
            }
            db.create_session("test-session", agent_id, "cli").unwrap();
            Self {
                db: AsyncDatabase::new_with_agent(db, agent_id),
                counter: AtomicU32::new(0),
            }
        }

        /// Create a non-onboarding ToolContext borrowing from this harness.
        pub fn ctx(&self) -> ToolContext<'_> {
            test_ctx(&self.db, &self.counter)
        }

        /// Create a ToolContext with configurable onboarding flag.
        pub fn ctx_with_onboarding(&self, is_onboarding: bool) -> ToolContext<'_> {
            test_ctx_with_onboarding(&self.db, &self.counter, is_onboarding)
        }

        /// Create a ToolContext in reflection mode.
        pub fn ctx_with_reflection(&self) -> ToolContext<'_> {
            static SKILLS_DIRTY: AtomicBool = AtomicBool::new(false);
            ToolContext {
                db: &self.db,
                session_id: "test-session",
                trace_id: "00000000000000000000000000000000",
                home_dir: std::path::Path::new("/tmp/mika-test"),
                global_home_dir: None,
                core_memory_edit_count: &self.counter,
                is_onboarding: false,
                message_sender: None,
                embedding_client: None,
                brave_api_key: None,
                github_token: None,
                skills_dirty: &SKILLS_DIRTY,
                is_reflection: true,
                is_task_context: false,
                is_callback_turn: false,
                provider_name: "anthropic",
                model_name: "claude-sonnet-4-6",
            }
        }

        /// Create a ToolContext with a custom home directory.
        pub fn ctx_with_home<'a>(&'a self, home: &'a std::path::Path) -> ToolContext<'a> {
            static SKILLS_DIRTY: AtomicBool = AtomicBool::new(false);
            ToolContext {
                db: &self.db,
                session_id: "test-session",
                trace_id: "00000000000000000000000000000000",
                home_dir: home,
                global_home_dir: None,
                core_memory_edit_count: &self.counter,
                is_onboarding: false,
                message_sender: None,
                embedding_client: None,
                brave_api_key: None,
                github_token: None,
                skills_dirty: &SKILLS_DIRTY,
                is_reflection: false,
                is_task_context: false,
                is_callback_turn: false,
                provider_name: "anthropic",
                model_name: "claude-sonnet-4-6",
            }
        }
        /// Create a ToolContext with custom home and global home directories.
        /// Used for testing cross-agent file access.
        pub fn ctx_with_home_and_global<'a>(
            &'a self,
            home: &'a std::path::Path,
            global: &'a std::path::Path,
        ) -> ToolContext<'a> {
            static SKILLS_DIRTY: AtomicBool = AtomicBool::new(false);
            ToolContext {
                db: &self.db,
                session_id: "test-session",
                trace_id: "00000000000000000000000000000000",
                home_dir: home,
                global_home_dir: Some(global),
                core_memory_edit_count: &self.counter,
                is_onboarding: false,
                message_sender: None,
                embedding_client: None,
                brave_api_key: None,
                github_token: None,
                skills_dirty: &SKILLS_DIRTY,
                is_reflection: false,
                is_task_context: false,
                is_callback_turn: false,
                provider_name: "anthropic",
                model_name: "claude-sonnet-4-6",
            }
        }
    }

    /// Create a manual task in the test DB and return its ID.
    pub async fn create_test_task(db: &crate::async_db::AsyncDatabase) -> String {
        use crate::db::NewTask;
        use crate::task_engine::types::{action_type, trigger_type};

        let task = NewTask {
            agent_id: db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "test task".to_string(),
            trigger_type: trigger_type::MANUAL.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::NONE.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
        };
        db.create_task(task).await.unwrap()
    }

    /// Minimal Settings for validation-only tests (no API key needed).
    pub fn dummy_settings() -> Settings {
        Settings {
            llm_provider: mika_common::llm::ProviderKind::Anthropic,
            llm_max_tokens: 4096,
            // Per-provider fields (all None = use defaults)
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
            // Non-provider settings
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
}
