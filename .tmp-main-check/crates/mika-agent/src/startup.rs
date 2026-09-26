use std::path::Path;

use anyhow::Result;
use mika_common::home;
use mika_common::llm::ProviderKind;
use tracing::{info, warn};

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
    // One-shot repair of orphaned generated skill variants (mika#1663). Runs
    // in all modes — including MIKA_DISABLE_BUNDLED_SKILLS — because orphaned
    // variants exist independently of bundled-skill seeding and should be
    // repaired regardless of that debugging flag.
    migrate_generated_variant_provider_dirs(agent_home);

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

/// One-shot migration: rename generated-variant provider directories whose
/// name is a non-canonical `ProviderKind` alias (e.g. OpenRouter's `z-ai`) to
/// the canonical config-key form (`zai`) that `scan_generated_variants`
/// accepts (mika#1663).
///
/// Variants written under an aggregator-namespace provider segment were
/// silently orphaned: the writer used the raw OpenRouter namespace string
/// (`z-ai`), but the loader only recognizes `ProviderKind::from_str`-parseable
/// config-key names (`zai`). The writer is fixed forward
/// (`resolve_canonical_provider_model` now normalizes the aggregator split);
/// this migration repairs variants already on disk.
///
/// Walks `<agent_home>/skills/*/generated/<provider>/`. A provider dir
/// qualifies when its name parses to a `ProviderKind` but differs from that
/// kind's `config_prefix()` — exactly the alias case (e.g. `z-ai` → `zai`).
/// Skill directories are commonly symlinks into the shared library, so the
/// physical rename lands in the library; idempotent across agents that share
/// it.
///
/// Idempotent: a directory already in canonical form is skipped. When both the
/// alias dir and the canonical dir exist, model subdirectories are merged
/// (canonical wins on collision) and the alias dir is removed. All filesystem
/// errors are warn-and-continue — a migration failure must never block startup.
pub fn migrate_generated_variant_provider_dirs(agent_home: &Path) {
    let skills_dir = agent_home.join("skills");
    let skill_entries = match std::fs::read_dir(&skills_dir) {
        Ok(rd) => rd,
        Err(_) => return, // no skills dir yet (fresh install) — nothing to migrate
    };

    for skill_entry in skill_entries.flatten() {
        let skill_name = skill_entry.file_name().to_string_lossy().into_owned();
        // Skip support dirs (`_shared/`) and dotfiles — they hold no variants.
        if skill_name.starts_with('_') || skill_name.starts_with('.') {
            continue;
        }
        let generated_root = skill_entry.path().join("generated");
        let provider_dirs = match std::fs::read_dir(&generated_root) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for provider_entry in provider_dirs.flatten() {
            let provider_path = provider_entry.path();
            if !provider_path.is_dir() {
                continue;
            }
            let name = match provider_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Only alias dirs qualify: parseable to a ProviderKind but not
            // already the canonical config-key form.
            let Ok(kind) = name.parse::<ProviderKind>() else {
                continue;
            };
            let canonical = kind.config_prefix();
            if name == canonical {
                continue;
            }

            let canonical_path = generated_root.join(canonical);
            if !canonical_path.exists() {
                match std::fs::rename(&provider_path, &canonical_path) {
                    Ok(()) => info!(
                        skill = %skill_name,
                        from = %name,
                        to = %canonical,
                        "migrated orphaned generated variant provider dir (mika#1663)"
                    ),
                    Err(e) => warn!(
                        skill = %skill_name,
                        from = %name,
                        to = %canonical,
                        error = %e,
                        "failed to migrate generated variant provider dir"
                    ),
                }
                continue;
            }

            // Canonical dir already exists — merge per-model subdirs.
            let model_dirs = match std::fs::read_dir(&provider_path) {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            for model_entry in model_dirs.flatten() {
                let src_model = model_entry.path();
                let model_name = match src_model.file_name() {
                    Some(n) => n.to_owned(),
                    None => continue,
                };
                let dst_model = canonical_path.join(&model_name);
                if dst_model.exists() {
                    // Canonical wins — drop the stale alias copy.
                    let _ = std::fs::remove_dir_all(&src_model);
                } else if let Err(e) = std::fs::rename(&src_model, &dst_model) {
                    warn!(
                        skill = %skill_name,
                        model = %model_name.to_string_lossy(),
                        error = %e,
                        "failed to merge generated variant model dir during migration"
                    );
                }
            }
            // Remove the (now hopefully empty) alias provider dir.
            if let Err(e) = std::fs::remove_dir(&provider_path) {
                warn!(
                    skill = %skill_name,
                    dir = %name,
                    error = %e,
                    "failed to remove migrated alias provider dir (may be non-empty)"
                );
            } else {
                info!(
                    skill = %skill_name,
                    from = %name,
                    to = %canonical,
                    "merged orphaned generated variants into canonical provider dir (mika#1663)"
                );
            }
        }
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

    fn write_variant(agent_home: &Path, skill: &str, provider: &str, model: &str, body: &str) {
        let dir = agent_home
            .join("skills")
            .join(skill)
            .join("generated")
            .join(provider)
            .join(model);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("system_prompt.md"), body).unwrap();
    }

    #[test]
    fn test_migrate_renames_zai_alias_dir() {
        // mika#1663: an orphaned `z-ai/` variant dir is renamed to canonical `zai/`.
        let tmp = tempfile::tempdir().unwrap();
        write_variant(tmp.path(), "dev-pilot", "z-ai", "glm-5.2", "GLM variant");

        migrate_generated_variant_provider_dirs(tmp.path());

        let gen_dir = tmp.path().join("skills/dev-pilot/generated");
        assert!(
            !gen_dir.join("z-ai").exists(),
            "alias dir should be removed after migration"
        );
        let migrated = gen_dir.join("zai/glm-5.2/system_prompt.md");
        assert!(migrated.is_file(), "variant should now live under zai/");
        assert_eq!(std::fs::read_to_string(migrated).unwrap(), "GLM variant");
    }

    #[test]
    fn test_migrate_is_idempotent_and_skips_canonical() {
        // A canonical `zai/` dir is untouched; a second run is a no-op.
        let tmp = tempfile::tempdir().unwrap();
        write_variant(
            tmp.path(),
            "dev-pilot",
            "zai",
            "glm-5.2",
            "already canonical",
        );

        migrate_generated_variant_provider_dirs(tmp.path());
        migrate_generated_variant_provider_dirs(tmp.path());

        let path = tmp
            .path()
            .join("skills/dev-pilot/generated/zai/glm-5.2/system_prompt.md");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "already canonical");
    }

    #[test]
    fn test_migrate_merges_into_existing_canonical_dir() {
        // When both alias and canonical dirs exist, non-colliding models move
        // over and the canonical copy wins on collision; alias dir is removed.
        let tmp = tempfile::tempdir().unwrap();
        // Collision: same model in both — canonical must win.
        write_variant(tmp.path(), "dev-pilot", "zai", "glm-5.2", "canonical wins");
        write_variant(tmp.path(), "dev-pilot", "z-ai", "glm-5.2", "stale alias");
        // Non-colliding model only in the alias dir — must migrate.
        write_variant(tmp.path(), "dev-pilot", "z-ai", "glm-4.6", "alias only");

        migrate_generated_variant_provider_dirs(tmp.path());

        let gen_dir = tmp.path().join("skills/dev-pilot/generated");
        assert!(!gen_dir.join("z-ai").exists(), "alias dir should be gone");
        assert_eq!(
            std::fs::read_to_string(gen_dir.join("zai/glm-5.2/system_prompt.md")).unwrap(),
            "canonical wins",
            "canonical content must win on collision"
        );
        assert_eq!(
            std::fs::read_to_string(gen_dir.join("zai/glm-4.6/system_prompt.md")).unwrap(),
            "alias only",
            "non-colliding model must migrate into canonical dir"
        );
    }

    #[test]
    fn test_migrate_ignores_unknown_and_support_dirs() {
        // Unknown provider names (not a ProviderKind) and `_shared/` are left alone.
        let tmp = tempfile::tempdir().unwrap();
        write_variant(tmp.path(), "dev-pilot", "mystery-vendor", "m1", "keep me");
        write_variant(tmp.path(), "_shared", "z-ai", "glm-5.2", "support dir");

        migrate_generated_variant_provider_dirs(tmp.path());

        assert!(
            tmp.path()
                .join("skills/dev-pilot/generated/mystery-vendor/m1/system_prompt.md")
                .is_file(),
            "unknown provider dir must be left untouched"
        );
        assert!(
            tmp.path()
                .join("skills/_shared/generated/z-ai/glm-5.2/system_prompt.md")
                .is_file(),
            "support dirs must be skipped by the migration"
        );
    }

    #[test]
    fn test_migrate_no_skills_dir_is_noop() {
        // Fresh install with no skills dir must not panic.
        let tmp = tempfile::tempdir().unwrap();
        migrate_generated_variant_provider_dirs(tmp.path());
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
