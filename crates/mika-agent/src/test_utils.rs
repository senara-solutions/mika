#[cfg(test)]
pub mod test_helpers {
    use crate::async_db::AsyncDatabase;
    use crate::db::Database;
    use crate::tools::ToolContext;
    use std::sync::Once;
    use std::sync::atomic::AtomicU32;

    static INIT_VEC: Once = Once::new();

    /// Ensure sqlite-vec is registered exactly once per test process.
    fn ensure_sqlite_vec() {
        INIT_VEC.call_once(crate::db::init_sqlite_vec);
    }

    /// Create an in-memory database for tests (sync — for db module tests).
    pub fn test_db() -> Database {
        ensure_sqlite_vec();
        Database::open_in_memory().unwrap()
    }

    /// Create an async database for tool/agent tests.
    pub fn test_async_db() -> AsyncDatabase {
        ensure_sqlite_vec();
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
        ToolContext {
            db,
            session_id: "test-session",
            home_dir: std::path::Path::new(HOME_DIR),
            core_memory_edit_count: edit_count,
            is_onboarding,
            message_sender: None,
            embedding_client: None,
        }
    }

    /// Test harness that owns the database and edit counter, reducing
    /// boilerplate in tool tests. Use `harness.ctx()` to get a `ToolContext`
    /// and `harness.db` for direct database access during test setup.
    pub struct TestHarness {
        pub db: AsyncDatabase,
        pub counter: AtomicU32,
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
    }
}
