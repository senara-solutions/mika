use std::path::Path;

use anyhow::Result;
use tracing::info;

use crate::db::Database;

/// Seed core memory from `user.md` if the database has no core memory entries.
///
/// Shared between CLI and server startup paths to avoid duplication.
/// Reads `{home_dir}/user.md`, filters out the default template header,
/// and calls `Database::seed_core_memory`.
pub fn seed_core_memory_if_empty(db: &Database, home_dir: &Path) -> Result<()> {
    if db.get_all_core_memory()?.is_empty() {
        let user_md_path = home_dir.join("user.md");
        let user_md_content = std::fs::read_to_string(&user_md_path).ok();
        let user_md_ref = user_md_content
            .as_deref()
            .filter(|s| !s.starts_with("# Tell Mika about yourself"));
        db.seed_core_memory(user_md_ref)?;
        info!("seeded core memory for new database");
    }
    Ok(())
}

/// Seed bundled skills into `{home_dir}/skills/` if the directory exists.
///
/// Shared between CLI and server startup paths. Only writes skills whose
/// directories don't already exist (never overwrites user customizations).
pub fn seed_bundled_skills_if_needed(home_dir: &Path) {
    let skills_dir = home_dir.join("skills");
    if skills_dir.is_dir() {
        crate::bundled_skills::seed_bundled_skills(&skills_dir);
    }
}
