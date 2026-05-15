use std::path::Path;

use anyhow::Result;
use tracing::info;

use crate::db::Database;

/// Seed core memory from `user.md` if the database has no core memory entries.
///
/// Shared between CLI and server startup paths to avoid duplication.
/// Reads `{home_dir}/user.md`, filters out the default template header,
/// and calls `Database::seed_core_memory`.
pub fn seed_core_memory_if_empty(db: &Database, home_dir: &Path, agent_name: &str) -> Result<()> {
    // Migrate legacy "persona" key to "self_model" before checking emptiness
    db.migrate_persona_to_self_model(agent_name)?;
    if db.get_all_core_memory(agent_name)?.is_empty() {
        let user_md_path = home_dir.join("user.md");
        let user_md_content = std::fs::read_to_string(&user_md_path).ok();
        let user_md_ref = user_md_content
            .as_deref()
            .filter(|s| !s.starts_with("# Tell Mika about yourself"));
        db.seed_core_memory(agent_name, user_md_ref)?;
        info!("seeded core memory for new database");
    }
    Ok(())
}

/// Seed bundled skills into `{home_dir}/skills/` if the directory exists.
///
/// Shared between CLI and server startup paths. Only writes skills whose
/// directories don't already exist (never overwrites user customizations).
/// When `disabled` is true, skips seeding entirely (useful for debugging handlers).
///
/// Support directories (underscore-prefixed shared libraries like `_shared/`)
/// are seeded unconditionally — even when `disabled` is true — because they
/// are infrastructure (dispatch plumbing), not skill prompts. See mika#923.
pub fn seed_bundled_skills_if_needed(home_dir: &Path, disabled: bool) {
    let skills_dir = home_dir.join("skills");

    // Support dirs are infrastructure (dispatch plumbing), not skill prompts.
    // They must be seeded even when MIKA_DISABLE_BUNDLED_SKILLS=true — that
    // flag is for hot-patching skill prompts during dev, not for breaking
    // the dispatch pipeline. See mika#923, mika#984.
    if skills_dir.is_dir() {
        crate::bundled_skills::seed_support_dirs(&skills_dir);
    }

    if disabled {
        tracing::warn!(
            "bundled skill seeding disabled by config \
             (MIKA_DISABLE_BUNDLED_SKILLS=true) — handler script security updates \
             will not be applied; set to false or remove to re-enable"
        );
        // Drift detection (#984): check on-disk state against build-time hashes
        // to surface stale schemas that would cause validate_required_fields no-ops.
        if skills_dir.is_dir() {
            let drift_count = crate::bundled_skills::check_bundled_skill_drift(&skills_dir);
            if drift_count > 0 {
                tracing::error!(
                    drift_count,
                    "bundled_skill_drift_summary: {drift_count} bundled skill(s) have \
                     stale on-disk content. Tool schema validation may silently no-op. \
                     Remove MIKA_DISABLE_BUNDLED_SKILLS or re-deploy to fix."
                );
            }
        }
        return;
    }
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
        // Verify no skill directories were created (only support dirs)
        let entries: Vec<_> = std::fs::read_dir(&skills_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('_'))
            .collect();
        assert_eq!(
            entries.len(),
            0,
            "no skill directories should be created when disabled"
        );
    }

    #[test]
    fn test_support_dirs_seeded_when_bundled_skills_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        seed_bundled_skills_if_needed(tmp.path(), true);
        // Support dirs should exist even when disabled
        let dispatch_lib = skills_dir.join("_shared").join("dispatch-lib.sh");
        assert!(
            dispatch_lib.is_file(),
            "_shared/dispatch-lib.sh should be seeded even when MIKA_DISABLE_BUNDLED_SKILLS=true"
        );
        // But actual skill directories should NOT exist
        let skill_dirs: Vec<_> = std::fs::read_dir(&skills_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('_'))
            .collect();
        assert!(
            skill_dirs.is_empty(),
            "skill directories should not be created when disabled"
        );
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
