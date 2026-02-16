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
/// When `disabled` is true, skips seeding entirely (useful for debugging handlers).
pub fn seed_bundled_skills_if_needed(home_dir: &Path, disabled: bool) {
    if disabled {
        tracing::warn!("bundled skill seeding disabled by config");
        return;
    }
    let skills_dir = home_dir.join("skills");
    if skills_dir.is_dir() {
        crate::bundled_skills::seed_bundled_skills(&skills_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seed_bundled_skills_skipped_when_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        seed_bundled_skills_if_needed(tmp.path(), true);
        // Verify no skill directories were created
        assert_eq!(std::fs::read_dir(&skills_dir).unwrap().count(), 0);
    }

    #[test]
    fn test_seed_bundled_skills_runs_when_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        seed_bundled_skills_if_needed(tmp.path(), false);
        // Verify skills were seeded (at least one directory created)
        assert!(std::fs::read_dir(&skills_dir).unwrap().count() > 0);
    }
}
