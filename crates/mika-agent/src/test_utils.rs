#[cfg(test)]
pub mod test_helpers {
    use crate::async_db::AsyncDatabase;
    use crate::db::Database;
    use crate::teams::types::{RunStatus, TaskAssignment, TaskStatus, TeamRun};
    use crate::tools::ToolContext;
    use mika_common::config::Settings;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU32};

    /// Create an in-memory database for tests (sync — for db module tests).
    pub fn test_db() -> Database {
        crate::db::init_sqlite_vec();
        Database::open_in_memory().unwrap()
    }

    /// Create an async database for tool/agent tests.
    pub fn test_async_db() -> AsyncDatabase {
        crate::db::init_sqlite_vec();
        let db = Database::open_in_memory().unwrap();
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
            home_dir: std::path::Path::new(HOME_DIR),
            core_memory_edit_count: edit_count,
            is_onboarding,
            message_sender: None,
            embedding_client: None,
            brave_api_key: None,
            skills_dirty: &SKILLS_DIRTY,
            is_reflection: false,
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
                home_dir: std::path::Path::new("/tmp/mika-test"),
                core_memory_edit_count: &self.counter,
                is_onboarding: false,
                message_sender: None,
                embedding_client: None,
                brave_api_key: None,
                skills_dirty: &SKILLS_DIRTY,
                is_reflection: true,
            }
        }

        /// Create a ToolContext with a custom home directory.
        pub fn ctx_with_home<'a>(&'a self, home: &'a std::path::Path) -> ToolContext<'a> {
            static SKILLS_DIRTY: AtomicBool = AtomicBool::new(false);
            ToolContext {
                db: &self.db,
                session_id: "test-session",
                home_dir: home,
                core_memory_edit_count: &self.counter,
                is_onboarding: false,
                message_sender: None,
                embedding_client: None,
                brave_api_key: None,
                skills_dirty: &SKILLS_DIRTY,
                is_reflection: false,
            }
        }
    }

    /// Minimal Settings for validation-only tests (no API key needed).
    pub fn dummy_settings() -> Settings {
        Settings {
            anthropic_api_key: None,
            claude_model: "claude-sonnet-4-6".to_string(),
            claude_max_tokens: 4096,
            db_path: PathBuf::from("test.db"),
            log_level: "info".to_string(),
            routing_url: None,
            customer_id: None,
            server_port: 8080,
            internal_token: None,
            openai_api_key: None,
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_dimensions: 512,
            brave_api_key: None,
            home_dir: PathBuf::from("/tmp"),
            server_log_file: None,
            disable_bundled_skills: false,
        }
    }

    /// Sample TeamRun for history/status tests.
    pub fn test_team_run() -> TeamRun {
        TeamRun {
            run_id: "abcd1234".to_string(),
            team_name: "dev-team".to_string(),
            goal: "Test goal".to_string(),
            status: RunStatus::Completed,
            iteration: 1,
            max_iterations: 3,
            tasks: vec![TaskAssignment {
                agent: "researcher".to_string(),
                role: "specialist".to_string(),
                task: "Research".to_string(),
                output_file: "research.md".to_string(),
                status: TaskStatus::Completed,
            }],
            started_at: "2026-02-25T10:00:00Z".to_string(),
            ended_at: Some("2026-02-25T10:05:00Z".to_string()),
            deliverable: Some("Summary".to_string()),
        }
    }
}
