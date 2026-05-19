use std::path::Path;

use anyhow::Result;
use mika_common::home;
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

/// Seed the bundled-skill library and materialize per-agent symlinks.
///
/// Shared between CLI and server startup paths. Performs two concerns
/// (mika#1213):
///
/// 1. **Library extraction** — `seed_bundled_skill_library` writes the
///    binary's bundled-skill manifest into the canonical library at
///    `<global_home>/skills/`. Sync-shaped (only the manifest set survives)
///    and hash-gated (idempotent across restarts).
///
/// 2. **Per-agent symlinks** — `materialize_agent_skill_links` ensures
///    `<agent_home>/skills/<skill>` is a symlink into the library for each
///    bundled skill in the agent's identity allowlist. Removes symlinks
///    for de-allowlisted bundled skills. Leaves non-bundled (marketplace)
///    directories and `--copy`-managed directories alone.
///
/// `agent_home` is expected to be at `<global_home>/agents/<name>/` in the
/// multi-agent layout (current default). When the legacy single-agent
/// layout is detected (no `agents/` parent), the agent home itself is the
/// global home — the library and the agent's skills dir collapse onto the
/// same path, and the symlink pass is a no-op.
///
/// When `disabled` is true, library extraction is skipped (drift detection
/// still runs against the agent's skills dir, preserving the
/// `MIKA_DISABLE_BUNDLED_SKILLS` debugging workflow). Support directories
/// (`_shared/`) are seeded unconditionally because they are infrastructure
/// (dispatch plumbing), not skill prompts. See mika#923.
pub fn seed_bundled_skills_if_needed(agent_home: &Path, disabled: bool) {
    let (global_home, is_multi_agent) = resolve_global_home(agent_home);
    let library_dir = home::library_skills_dir(&global_home);
    let agent_skills_dir = agent_home.join("skills");

    // Support dirs are infrastructure (dispatch plumbing), not skill prompts.
    // They must be seeded even when MIKA_DISABLE_BUNDLED_SKILLS=true — that
    // flag is for hot-patching skill prompts during dev, not for breaking
    // the dispatch pipeline. See mika#923, mika#984.
    if let Err(e) = std::fs::create_dir_all(&library_dir) {
        tracing::warn!(
            library = %library_dir.display(),
            error = %e,
            "failed to create bundled-skill library directory"
        );
        return;
    }
    crate::bundled_skills::seed_support_dirs(&library_dir);

    if disabled {
        tracing::warn!(
            "bundled skill seeding disabled by config \
             (MIKA_DISABLE_BUNDLED_SKILLS=true) — handler script security updates \
             will not be applied; set to false or remove to re-enable"
        );
        // Drift detection (#984): the agent-visible skills dir may resolve
        // through symlinks to the library, but the on-disk reader follows
        // them, so the check below is still meaningful.
        if agent_skills_dir.is_dir() {
            let drift_count = crate::bundled_skills::check_bundled_skill_drift(&agent_skills_dir);
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

    crate::bundled_skills::seed_bundled_skill_library(&library_dir);

    // Per-agent symlinks: legacy single-agent layout collapses the agent
    // home onto the global home, so the library IS the agent skills dir —
    // skip the materialize pass (it would be a no-op against itself).
    if !is_multi_agent {
        return;
    }
    let identity = crate::prompt::load_identity(agent_home);
    let allowlist = identity.skills.allowlist.as_deref();
    crate::bundled_skills::materialize_agent_skill_links(
        &library_dir,
        &agent_skills_dir,
        allowlist,
    );
}

/// Resolve the global Mika home (the directory containing `agents/`) from
/// an `agent_home` path. Returns `(global_home, is_multi_agent)`.
///
/// In the multi-agent layout `agent_home` lives at
/// `<global_home>/agents/<name>/`. When the path does not have that shape
/// (legacy single-agent layout, tests passing tmp dirs directly), the agent
/// home is treated as the global home.
fn resolve_global_home(agent_home: &Path) -> (std::path::PathBuf, bool) {
    if let Some(parent) = agent_home.parent()
        && parent.file_name().and_then(|s| s.to_str()) == Some("agents")
        && let Some(global) = parent.parent()
    {
        return (global.to_path_buf(), true);
    }
    (agent_home.to_path_buf(), false)
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
