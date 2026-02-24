#[cfg(test)]
pub mod test_helpers {
    use crate::db::Database;
    use crate::tools::ToolContext;
    use std::sync::atomic::AtomicU32;

    /// Create an in-memory database for tests.
    pub fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    /// Create a ToolContext for tests (non-onboarding).
    pub fn test_ctx<'a>(db: &'a Database, edit_count: &'a AtomicU32) -> ToolContext<'a> {
        test_ctx_with_onboarding(db, edit_count, false)
    }

    /// Create a ToolContext for tests with configurable onboarding flag.
    pub fn test_ctx_with_onboarding<'a>(
        db: &'a Database,
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
        }
    }
}
