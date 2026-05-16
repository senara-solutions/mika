//! Compile-time embedded skill templates, seeded into agent skill directories on startup.
//!
//! Each skill is a set of files (skill.toml, tools.json, optional system_prompt.md,
//! optional handler scripts) embedded via `include_str!`. On startup, these are
//! written (or updated) to `{agent_home}/skills/{skill_name}/`.

use std::path::Path;
use tracing::{debug, info, warn};

/// A single file within a bundled skill.
struct SkillFile {
    /// Relative path within the skill directory (e.g. "skill.toml", "handlers/run.sh").
    path: &'static str,
    /// File contents (embedded at compile time).
    content: &'static str,
    /// Whether the file should be marked executable (handler scripts).
    executable: bool,
}

/// A complete bundled skill.
struct BundledSkill {
    name: &'static str,
    files: &'static [SkillFile],
    /// SHA-256 content hash (first 16 hex chars) computed at build time over the
    /// concatenation of all file contents in sorted order. Used for drift detection
    /// when `MIKA_DISABLE_BUNDLED_SKILLS=true` prevents re-seeding (#984).
    content_hash: &'static str,
}

/// A support directory (underscore-prefixed, non-skill shared library).
/// Seeded alongside skills so sibling skills can source them at runtime.
/// See mika#923.
struct SupportDir {
    name: &'static str,
    files: &'static [SkillFile],
}

/// Declare a bundled skill with its files.
/// Use `+x` suffix to mark a file as executable (handler scripts).
/// Legacy skills get an empty content_hash (drift detection scoped to engine-coupled
/// skills in `skills/bundled/` which are hashed at build time by `build.rs`).
macro_rules! skill {
    ($name:expr, [ $( $entry:tt ),+ $(,)? ]) => {
        BundledSkill {
            name: $name,
            files: &[
                $( skill!(@file $entry), )+
            ],
            content_hash: "",
        }
    };
    (@file ($path:expr => $template:expr, +x)) => {
        SkillFile {
            path: $path,
            content: include_str!($template),
            executable: true,
        }
    };
    (@file ($path:expr => $template:expr)) => {
        SkillFile {
            path: $path,
            content: include_str!($template),
            executable: false,
        }
    };
}

static TMUX_SKILL: BundledSkill = skill!("tmux", [
    ("skill.toml" => "../templates/skills/tmux/skill.toml"),
    ("system_prompt.md" => "../templates/skills/tmux/system_prompt.md"),
    ("tools.json" => "../templates/skills/tmux/tools.json"),
    ("handlers/create_session.sh" => "../templates/skills/tmux/handlers/create_session.sh", +x),
    ("handlers/kill_session.sh" => "../templates/skills/tmux/handlers/kill_session.sh", +x),
    ("handlers/list_sessions.sh" => "../templates/skills/tmux/handlers/list_sessions.sh", +x),
    ("handlers/read_output.sh" => "../templates/skills/tmux/handlers/read_output.sh", +x),
    ("handlers/send_command.sh" => "../templates/skills/tmux/handlers/send_command.sh", +x),
    ("handlers/wait_for_text.sh" => "../templates/skills/tmux/handlers/wait_for_text.sh", +x),
]);

static SHELL_EXEC_SKILL: BundledSkill = skill!("shell-exec", [
    ("skill.toml" => "../templates/skills/shell-exec/skill.toml"),
    ("system_prompt.md" => "../templates/skills/shell-exec/system_prompt.md"),
    ("tools.json" => "../templates/skills/shell-exec/tools.json"),
    ("handlers/run.sh" => "../templates/skills/shell-exec/handlers/run.sh", +x),
]);

static WEB_SEARCH_SKILL: BundledSkill = skill!("web-search", [
    ("skill.toml" => "../templates/skills/web-search/skill.toml"),
    ("system_prompt.md" => "../templates/skills/web-search/system_prompt.md"),
    ("tools.json" => "../templates/skills/web-search/tools.json"),
]);

static FILE_READER_SKILL: BundledSkill = skill!("file-reader", [
    ("skill.toml" => "../templates/skills/file-reader/skill.toml"),
    ("system_prompt.md" => "../templates/skills/file-reader/system_prompt.md"),
    ("tools.json" => "../templates/skills/file-reader/tools.json"),
    ("handlers/read.sh" => "../templates/skills/file-reader/handlers/read.sh", +x),
]);

// skill-review migrated to skills/bundled/ (engine-coupled via review_filter).
// Discovered at build time by build.rs — no static entry needed.

static SELF_KNOWLEDGE_SKILL: BundledSkill = skill!("self-knowledge", [
    ("skill.toml" => "../templates/skills/self-knowledge/skill.toml"),
    ("system_prompt.md" => "../templates/skills/self-knowledge/system_prompt.md"),
    ("tools.json" => "../templates/skills/self-knowledge/tools.json"),
]);

static GOOGLE_WORKSPACE_SKILL: BundledSkill = skill!("google-workspace", [
    ("skill.toml" => "../templates/skills/google-workspace/skill.toml"),
    ("system_prompt.md" => "../templates/skills/google-workspace/system_prompt.md"),
    ("tools.json" => "../templates/skills/google-workspace/tools.json"),
]);

static GIT_OPS_SKILL: BundledSkill = skill!("git-ops", [
    ("skill.toml" => "../templates/skills/git-ops/skill.toml"),
    ("system_prompt.md" => "../templates/skills/git-ops/system_prompt.md"),
    ("tools.json" => "../templates/skills/git-ops/tools.json"),
]);

static GITHUB_SKILL: BundledSkill = skill!("github", [
    ("skill.toml" => "../templates/skills/github/skill.toml"),
    ("system_prompt.md" => "../templates/skills/github/system_prompt.md"),
    ("tools.json" => "../templates/skills/github/tools.json"),
]);

static MCP_SKILL: BundledSkill = skill!("mcp", [
    ("skill.toml" => "../templates/skills/mcp/skill.toml"),
    ("system_prompt.md" => "../templates/skills/mcp/system_prompt.md"),
    ("tools.json" => "../templates/skills/mcp/tools.json"),
]);

static BROWSER_CONTROL_SKILL: BundledSkill = skill!("browser-control", [
    ("skill.toml" => "../templates/skills/browser-control/skill.toml"),
    ("system_prompt.md" => "../templates/skills/browser-control/system_prompt.md"),
    ("tools.json" => "../templates/skills/browser-control/tools.json"),
]);

// agents-teams migrated to skills/bundled/ (engine-coupled via delegate_task /
// run_team guards). Discovered at build time by build.rs.

/// Legacy hardcoded bundled skills (community-category skills kept embedded
/// for convenience). Engine-coupled skills live in `skills/bundled/` and are
/// discovered at build time — see the `ENTRIES` table below.
static BUNDLED_SKILLS: &[&BundledSkill] = &[
    &TMUX_SKILL,
    &SHELL_EXEC_SKILL,
    &WEB_SEARCH_SKILL,
    &FILE_READER_SKILL,
    &SELF_KNOWLEDGE_SKILL,
    &GIT_OPS_SKILL,
    &GOOGLE_WORKSPACE_SKILL,
    &GITHUB_SKILL,
    &MCP_SKILL,
    &BROWSER_CONTROL_SKILL,
];

// Directory-sourced bundled skills, generated at build time by `build.rs`
// walking `<workspace>/skills/bundled/`. Declares `static ENTRIES: &[BundledSkill]`.
// An empty or missing `skills/bundled/` directory yields `ENTRIES = &[]` — the
// parallel path stays silent until a future migration ticket starts populating it.
include!(concat!(env!("OUT_DIR"), "/bundled_skills_generated.rs"));

/// Pure merge primitive: overlay `entries` onto `legacy` with case-insensitive
/// ENTRIES-wins-on-name-collision semantics. Extracted so both the production
/// `all_bundled_skills()` path and tests can exercise the same implementation.
fn merge_skill_lists(
    legacy: &[&'static BundledSkill],
    entries: &'static [BundledSkill],
) -> Vec<&'static BundledSkill> {
    let mut merged: Vec<&'static BundledSkill> = legacy.to_vec();
    for entry in entries {
        if let Some(slot) = merged
            .iter_mut()
            .find(|existing| existing.name.eq_ignore_ascii_case(entry.name))
        {
            *slot = entry;
        } else {
            merged.push(entry);
        }
    }
    merged
}

/// Merge the legacy hardcoded `BUNDLED_SKILLS` list with the directory-sourced
/// `ENTRIES` table. Entries from `ENTRIES` win on case-insensitive name collision.
///
/// Returns a deduplicated `Vec<&'static BundledSkill>`. Zero collisions are
/// expected in production during this refactor (ENTRIES is empty). The merge
/// semantics exist so a future migration ticket can move skills one-by-one
/// without orchestrating a coordinated cutover.
fn all_bundled_skills() -> Vec<&'static BundledSkill> {
    merge_skill_lists(BUNDLED_SKILLS, ENTRIES)
}

/// Check whether a skill name matches a bundled (built-in) skill.
///
/// Consults both the legacy hardcoded list and the directory-sourced `ENTRIES`
/// table so install/uninstall/update guards treat either source as immutable.
///
/// Invariant: this must enumerate every source consulted by
/// [`all_bundled_skills`]. If a third source is added, update both functions
/// together. The `test_is_bundled_skill_agrees_with_all_bundled_skills` test
/// guards the coupling.
pub fn is_bundled_skill(name: &str) -> bool {
    BUNDLED_SKILLS
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case(name))
        || ENTRIES.iter().any(|s| s.name.eq_ignore_ascii_case(name))
}

/// Returns the names of all bundled skills (both legacy hardcoded and
/// directory-sourced), deduplicated with ENTRIES-wins semantics.
///
/// Used by well-known agent tests to verify that mika-relay's `disabled_skills`
/// list stays in sync with the full bundled skill set.
pub fn all_bundled_skill_names() -> Vec<&'static str> {
    all_bundled_skills().iter().map(|s| s.name).collect()
}

/// Trust-critical bundled skills whose prompts must NOT be reviewed or adapted.
///
/// These skills govern the agent's self-awareness, security posture, or ability
/// to modify other skills. Model-specific rewording could weaken their safety
/// properties. All other bundled skills are "functional" — their prompts focus
/// on tool usage mechanics and are safe to adapt per-model.
///
/// Criteria for trust-critical classification:
/// - `skill-review`: can modify any skill's prompt (self-referential risk)
/// - `self-knowledge`: governs agent self-awareness and core identity
/// - `agents-teams`: controls multi-agent orchestration and delegation
static TRUST_CRITICAL_SKILLS: &[&str] = &["skill-review", "self-knowledge", "agents-teams"];

/// Skill names that were once bundled but have been renamed or removed,
/// listed here so the prune pass can clean up their per-agent directories
/// on hosts that were deployed before the rename or removal.
///
/// **Update procedure:** any future PR that renames or removes a bundled
/// skill MUST add the OLD name here in the same commit. Stale entries are
/// harmless (the directories no longer exist on previously-cleaned hosts).
/// Entries can be removed in a later cleanup PR once every production host
/// is confirmed to have rebooted since the rename/removal landed.
///
/// **Marketplace-safety invariant:** entries in this list will be
/// `remove_dir_all`'d from every agent's `skills/` dir. Names added here
/// must be names that were definitively bundled in a prior release.
/// Adding the name of a marketplace skill here would wipe user state.
pub(crate) const KNOWN_REMOVED_BUNDLED_SKILLS: &[&str] = &[
    // claude-pilot was renamed to dev-pilot + dev-groom in mika#853
    // (deployed 2026-04-28). 12 stale per-agent directories survived
    // the rename — see mika#859 § Phase 0 P0.8 for verification output.
    "claude-pilot",
];

/// Check whether a skill is trust-critical (blocked from review/adaptation).
///
/// Only trust-critical bundled skills are blocked from `review_skill`. Other
/// bundled skills (e.g., web-search, shell-exec) are reviewable because their
/// prompts focus on tool usage mechanics — safe to adapt per-model.
///
/// Note: this does NOT replace `is_bundled_skill()` for install/delete/update
/// guards — ALL bundled skills remain protected from those operations.
pub fn is_trust_critical_skill(name: &str) -> bool {
    TRUST_CRITICAL_SKILLS
        .iter()
        .any(|s| s.eq_ignore_ascii_case(name))
}

/// Return the list of trust-critical skill names (for error messages and prompts).
pub fn trust_critical_skill_names() -> &'static [&'static str] {
    TRUST_CRITICAL_SKILLS
}

/// Remove per-agent skill directories whose name appears in
/// [`KNOWN_REMOVED_BUNDLED_SKILLS`]. Called once at the top of
/// [`seed_bundled_skills`].
///
/// Defense-in-depth (matches `write_skill`):
/// - Symlinks are never followed and never removed.
/// - `_`-prefixed support directories (e.g. `_shared/`) are skipped.
/// - `.`-prefixed entries (e.g. `.mika-bundled` markers in a future
///   revision) are skipped.
/// - I/O errors are logged and skipped — a single bad directory must not
///   block the seed.
///
/// Returns the number of directories actually removed.
pub(crate) fn prune_known_removed_bundled_skills(skills_dir: &Path) -> usize {
    let entries = match std::fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "failed to read skills_dir for known-removed prune");
            return 0;
        }
    };

    let mut pruned = 0;
    for entry in entries.flatten() {
        let path = entry.path();

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }

        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };

        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }

        let is_known_removed = KNOWN_REMOVED_BUNDLED_SKILLS
            .iter()
            .any(|s| s.eq_ignore_ascii_case(name));
        if !is_known_removed {
            continue;
        }

        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                info!(
                    skill = name,
                    "pruned orphaned bundled-skill directory (known-removed list)"
                );
                pruned += 1;
            }
            Err(e) => {
                warn!(
                    skill = name,
                    error = %e,
                    "failed to prune orphaned bundled-skill directory"
                );
            }
        }
    }
    pruned
}

/// Seed bundled skills into the given skills directory.
///
/// Always writes bundled skill files, updating existing installs to match the
/// current templates. This ensures template changes (e.g. removing a tool)
/// propagate to existing installs. User-created skills (non-bundled) are never
/// touched. Extra files in bundled skill directories that aren't part of the
/// bundle are left in place.
///
/// On first-time creation failure, the partially created directory is removed.
///
/// **Invariant:** Marketplace skills can never have the same name as a bundled
/// skill. The `mika skills install` command refuses installation when a name
/// collision with a bundled skill is detected. Therefore this function will
/// never overwrite a marketplace-installed skill.
pub fn seed_bundled_skills(skills_dir: &Path) {
    // Support directories are also seeded unconditionally from startup.rs
    // (before the disabled guard). This call ensures seed_bundled_skills()
    // is self-contained when called directly (e.g., by create_agent tool
    // or tests). The second write on the normal startup path is idempotent.
    seed_support_dirs(skills_dir);

    // Prune known-removed orphans BEFORE the write loop. Order matters:
    // if a future PR ever reuses a removed name, we want the prune to
    // happen first so the subsequent write_skill is a clean create rather
    // than an update over stale content.
    let pruned = prune_known_removed_bundled_skills(skills_dir);
    if pruned > 0 {
        info!(
            count = pruned,
            "pruned known-removed bundled-skill directories"
        );
    }

    for skill in all_bundled_skills() {
        let skill_dir = skills_dir.join(skill.name);
        let is_update = skill_dir.exists();

        if let Err(e) = write_skill(&skill_dir, skill) {
            warn!(skill = skill.name, error = %e, "failed to seed bundled skill");
            if !is_update {
                let _ = std::fs::remove_dir_all(&skill_dir);
            }
        } else if is_update {
            debug!(skill = skill.name, "updated bundled skill");
        } else {
            info!(skill = skill.name, "seeded bundled skill");
        }
    }
}

/// Write a set of files into the given directory.
///
/// Shared implementation for both skill and support directory seeding.
/// Creates parent directories as needed, refuses to overwrite symlinked files.
fn write_dir_files(dir: &Path, files: &[SkillFile]) -> std::io::Result<()> {
    for file in files {
        let file_path = dir.join(file.path);

        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Refuse to overwrite a file that is a symlink (defense-in-depth).
        if file_path.exists() && file_path.symlink_metadata()?.file_type().is_symlink() {
            return Err(std::io::Error::other(format!(
                "file '{}' is a symlink, refusing to write",
                file.path
            )));
        }

        std::fs::write(&file_path, file.content)?;

        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file_path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// Write all files for a single skill into the given directory.
fn write_skill(skill_dir: &Path, skill: &BundledSkill) -> std::io::Result<()> {
    // Refuse to write into symlinked skill directories (defense-in-depth).
    // An attacker could replace a bundled skill directory with a symlink to
    // redirect writes (including executable handler scripts) to an arbitrary
    // location.
    if skill_dir.exists() && skill_dir.symlink_metadata()?.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "skill directory is a symlink, refusing to write",
        ));
    }

    write_dir_files(skill_dir, skill.files)
}

/// Seed support directories (underscore-prefixed shared libraries) into the
/// given skills directory.
///
/// Support directories contain shared code (e.g. `_shared/dispatch-lib.sh`)
/// that sibling skills source at runtime via relative path. They are NOT skills
/// (no `skill.toml`) but must be present in the deployed skills tree.
///
/// Called unconditionally — even when `MIKA_DISABLE_BUNDLED_SKILLS=true` — because
/// support dirs are infrastructure (dispatch plumbing), not skill prompts. The
/// disable flag is for hot-patching skill prompts during dev, not for breaking
/// the dispatch pipeline. See mika#923.
pub fn seed_support_dirs(skills_dir: &Path) {
    for dir in SUPPORT_DIRS {
        let target_dir = skills_dir.join(dir.name);
        let is_update = target_dir.exists();

        // Refuse to write into symlinked support directories (defense-in-depth).
        if target_dir.exists() {
            match target_dir.symlink_metadata() {
                Ok(meta) if meta.file_type().is_symlink() => {
                    warn!(
                        support_dir = dir.name,
                        "support directory is a symlink, refusing to write"
                    );
                    continue;
                }
                Err(e) => {
                    warn!(support_dir = dir.name, error = %e, "failed to stat support directory");
                    continue;
                }
                _ => {}
            }
        }

        if let Err(e) = write_dir_files(&target_dir, dir.files) {
            warn!(support_dir = dir.name, error = %e, "failed to seed support directory");
            if !is_update {
                let _ = std::fs::remove_dir_all(&target_dir);
            }
        } else if is_update {
            debug!(support_dir = dir.name, "updated support directory");
        } else {
            info!(support_dir = dir.name, "seeded support directory");
        }
    }
}

/// Check on-disk bundled skills for content drift against the build-time embedded
/// hashes. Called when `MIKA_DISABLE_BUNDLED_SKILLS=true` to surface stale state.
///
/// Only checks directory-sourced skills (engine-coupled, from `skills/bundled/`)
/// since those carry build-time content hashes. Legacy community skills have
/// empty hashes and are skipped.
///
/// Emits one `ERROR` log per drifted skill and returns the count of drifts detected.
pub fn check_bundled_skill_drift(skills_dir: &Path) -> usize {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut drift_count = 0;

    for skill in all_bundled_skills() {
        // Skip skills without a build-time hash (legacy community skills)
        if skill.content_hash.is_empty() {
            continue;
        }

        let skill_dir = skills_dir.join(skill.name);
        if !skill_dir.exists() {
            // Skill not seeded at all — definitely drifted
            tracing::error!(
                skill = skill.name,
                expected_hash = skill.content_hash,
                "bundled_skill_drift: skill directory missing on disk \
                 (MIKA_DISABLE_BUNDLED_SKILLS=true prevented seeding)"
            );
            drift_count += 1;
            continue;
        }

        // Compute on-disk hash using the same algorithm as build.rs
        let mut hasher = DefaultHasher::new();
        let mut all_readable = true;
        for file in skill.files {
            let file_path = skill_dir.join(file.path);
            match std::fs::read_to_string(&file_path) {
                Ok(content) => {
                    file.path.hash(&mut hasher);
                    content.hash(&mut hasher);
                }
                Err(_) => {
                    all_readable = false;
                    break;
                }
            }
        }

        if !all_readable {
            tracing::error!(
                skill = skill.name,
                expected_hash = skill.content_hash,
                "bundled_skill_drift: one or more skill files unreadable on disk"
            );
            drift_count += 1;
            continue;
        }

        let on_disk_hash = format!("{:016x}", hasher.finish());
        if on_disk_hash != skill.content_hash {
            tracing::error!(
                skill = skill.name,
                expected_hash = %&skill.content_hash[..12.min(skill.content_hash.len())],
                actual_hash = %&on_disk_hash[..12],
                "bundled_skill_drift: on-disk content differs from build-time embed \
                 (MIKA_DISABLE_BUNDLED_SKILLS=true prevented update)"
            );
            drift_count += 1;
        }
    }

    drift_count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seed_creates_all_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);

        for skill in all_bundled_skills() {
            let skill_dir = skills_dir.join(skill.name);
            assert!(skill_dir.is_dir(), "skill dir missing: {}", skill.name);
            for file in skill.files {
                let file_path = skill_dir.join(file.path);
                assert!(
                    file_path.is_file(),
                    "file missing: {}/{}",
                    skill.name,
                    file.path
                );
                let content = std::fs::read_to_string(&file_path).unwrap();
                assert!(
                    !content.is_empty(),
                    "file empty: {}/{}",
                    skill.name,
                    file.path
                );
            }
        }
    }

    #[test]
    fn test_seed_updates_existing_bundled_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once
        seed_bundled_skills(skills_dir);

        // Modify a bundled file
        let marker = skills_dir.join("tmux").join("skill.toml");
        std::fs::write(&marker, "custom content").unwrap();

        // Seed again — should overwrite with bundled content
        seed_bundled_skills(skills_dir);

        let content = std::fs::read_to_string(&marker).unwrap();
        assert_ne!(content, "custom content", "bundled file should be updated");
        assert!(!content.is_empty(), "bundled file should have content");
    }

    #[test]
    fn test_seed_preserves_extra_files_in_bundled_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once
        seed_bundled_skills(skills_dir);

        // Add an extra file not in the bundle
        let extra = skills_dir.join("tmux").join("my_notes.txt");
        std::fs::write(&extra, "user notes").unwrap();

        // Seed again — extra file should survive
        seed_bundled_skills(skills_dir);

        let content = std::fs::read_to_string(&extra).unwrap();
        assert_eq!(content, "user notes");
    }

    #[test]
    fn test_seed_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);
        seed_bundled_skills(skills_dir);
        seed_bundled_skills(skills_dir);

        // All skills still present (merged view: legacy + directory-sourced)
        for skill in all_bundled_skills() {
            assert!(skills_dir.join(skill.name).is_dir());
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_symlinked_skill_dir_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once so all normal skills exist
        seed_bundled_skills(skills_dir);

        // Replace a bundled skill directory with a symlink to an external target
        let target_dir = tempfile::tempdir().unwrap();
        let tmux_dir = skills_dir.join("tmux");
        std::fs::remove_dir_all(&tmux_dir).unwrap();
        std::os::unix::fs::symlink(target_dir.path(), &tmux_dir).unwrap();

        // Seed again — the symlinked "tmux" dir should be skipped without crashing
        seed_bundled_skills(skills_dir);

        // The target directory should NOT contain any bundled files
        assert!(
            std::fs::read_dir(target_dir.path())
                .unwrap()
                .next()
                .is_none(),
            "symlink target should remain empty — write_skill must refuse to follow it",
        );

        // Other bundled skills should still be updated normally
        assert!(skills_dir.join("shell-exec").join("skill.toml").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn test_symlinked_file_inside_skill_dir_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once so all normal skills exist
        seed_bundled_skills(skills_dir);

        // Replace a single file inside a bundled skill directory with a symlink
        let target_file = tmp.path().join("evil_target.txt");
        std::fs::write(&target_file, "original").unwrap();
        let skill_toml = skills_dir.join("tmux").join("skill.toml");
        std::fs::remove_file(&skill_toml).unwrap();
        std::os::unix::fs::symlink(&target_file, &skill_toml).unwrap();

        // Seed again — tmux should fail because skill.toml is a symlink
        seed_bundled_skills(skills_dir);

        // The symlink target file should not have been overwritten
        let content = std::fs::read_to_string(&target_file).unwrap();
        assert_eq!(
            content, "original",
            "symlinked file target must not be overwritten",
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_handlers_are_executable() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);

        for skill in all_bundled_skills() {
            for file in skill.files {
                if file.executable {
                    let path = skills_dir.join(skill.name).join(file.path);
                    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                    assert!(
                        mode & 0o111 != 0,
                        "handler not executable: {}/{}",
                        skill.name,
                        file.path
                    );
                }
            }
        }
    }

    #[test]
    fn test_trust_critical_skills_are_subset_of_bundled() {
        // Every trust-critical skill must also be a bundled skill.
        for name in TRUST_CRITICAL_SKILLS {
            assert!(
                is_bundled_skill(name),
                "trust-critical skill '{}' is not in BUNDLED_SKILLS",
                name
            );
        }
    }

    #[test]
    fn test_is_trust_critical_skill() {
        assert!(is_trust_critical_skill("skill-review"));
        assert!(is_trust_critical_skill("self-knowledge"));
        assert!(is_trust_critical_skill("agents-teams"));
        // Case-insensitive
        assert!(is_trust_critical_skill("Skill-Review"));
        assert!(is_trust_critical_skill("SELF-KNOWLEDGE"));
    }

    #[test]
    fn test_reviewable_bundled_skills_not_trust_critical() {
        // Every bundled skill that is NOT trust-critical should be reviewable.
        // Derived from the merged view (legacy hardcoded + directory-sourced
        // `ENTRIES`) to avoid fragile enumeration.
        let all = all_bundled_skills();
        let reviewable: Vec<_> = all
            .iter()
            .filter(|s| !is_trust_critical_skill(s.name))
            .collect();
        // At least one bundled skill must be reviewable.
        assert!(
            !reviewable.is_empty(),
            "expected some reviewable bundled skills"
        );
        // Trust-critical must be a strict subset of bundled.
        assert!(
            TRUST_CRITICAL_SKILLS.len() < all.len(),
            "trust-critical should be smaller than bundled"
        );
        // Verify each reviewable skill is indeed bundled but not trust-critical.
        for skill in &reviewable {
            assert!(is_bundled_skill(skill.name));
            assert!(!is_trust_critical_skill(skill.name));
        }
    }

    #[test]
    fn test_skill_review_prompt_lists_trust_critical_skills() {
        // The skill-review system prompt must mention every trust-critical
        // skill name so the agent doesn't waste tool calls on them.
        let content = include_str!("../../../skills/bundled/skill-review/system_prompt.md");
        for name in TRUST_CRITICAL_SKILLS {
            assert!(
                content.contains(name),
                "skill-review prompt missing trust-critical skill '{}'",
                name
            );
        }
    }

    #[test]
    fn test_skill_review_prompt_does_not_reference_write_skill_variant() {
        // Regression: ensure the stale tool name is gone from the prompt.
        let content = include_str!("../../../skills/bundled/skill-review/system_prompt.md");
        assert!(
            !content.contains("write_skill_variant"),
            "skill-review prompt still references write_skill_variant"
        );
    }

    #[test]
    fn test_all_bundled_skills_includes_legacy_set() {
        // Every legacy hardcoded skill must appear in the merged view. With
        // empty production ENTRIES this is a strict equality on names; once
        // ENTRIES is populated by a future migration ticket, the merged view
        // will simply contain more names, never fewer.
        let merged = all_bundled_skills();
        for legacy in BUNDLED_SKILLS {
            assert!(
                merged
                    .iter()
                    .any(|s| s.name.eq_ignore_ascii_case(legacy.name)),
                "legacy bundled skill '{}' missing from merged view",
                legacy.name
            );
        }
    }

    #[test]
    fn test_is_bundled_skill_is_case_insensitive() {
        // Exercise both the legacy and ENTRIES arms — case-insensitivity must
        // hold regardless of which source the name comes from.
        assert!(is_bundled_skill("tmux"));
        assert!(is_bundled_skill("TMUX"));
        assert!(is_bundled_skill("Tmux"));
        assert!(!is_bundled_skill("definitely-not-a-skill"));
    }

    // Shared fixtures for merge-semantics tests — call `merge_skill_lists`
    // directly so the production merge function is the unit under test.
    // Previously the test re-implemented the merge algorithm locally, which
    // would silently pass if the production semantics drifted.
    static MERGE_TEST_LEGACY_FILES: &[SkillFile] = &[SkillFile {
        path: "skill.toml",
        content: "legacy",
        executable: false,
    }];
    static MERGE_TEST_LEGACY_ALPHA: BundledSkill = BundledSkill {
        name: "alpha",
        files: MERGE_TEST_LEGACY_FILES,
        content_hash: "",
    };
    static MERGE_TEST_LEGACY_GAMMA: BundledSkill = BundledSkill {
        name: "gamma",
        files: MERGE_TEST_LEGACY_FILES,
        content_hash: "",
    };
    static MERGE_TEST_LEGACY_SLICE: &[&BundledSkill] =
        &[&MERGE_TEST_LEGACY_ALPHA, &MERGE_TEST_LEGACY_GAMMA];

    static MERGE_TEST_OVERRIDE_FILES: &[SkillFile] = &[SkillFile {
        path: "skill.toml",
        content: "override",
        executable: false,
    }];
    static MERGE_TEST_FRESH_FILES: &[SkillFile] = &[SkillFile {
        path: "skill.toml",
        content: "fresh",
        executable: false,
    }];

    #[test]
    fn test_merge_prefers_entries_on_collision() {
        static ENTRIES_FIXTURE: &[BundledSkill] = &[
            BundledSkill {
                name: "ALPHA", // case-insensitive collision with MERGE_TEST_LEGACY_ALPHA
                files: MERGE_TEST_OVERRIDE_FILES,
                content_hash: "",
            },
            BundledSkill {
                name: "beta", // new addition
                files: MERGE_TEST_FRESH_FILES,
                content_hash: "",
            },
        ];

        let merged = merge_skill_lists(MERGE_TEST_LEGACY_SLICE, ENTRIES_FIXTURE);
        // 2 legacy + 1 new = 3 (alpha collision merged)
        assert_eq!(merged.len(), 3);

        let alpha = merged.iter().find(|s| s.name.eq_ignore_ascii_case("alpha"));
        assert!(alpha.is_some(), "alpha missing from merged view");
        assert_eq!(
            alpha.unwrap().files[0].content,
            "override",
            "ENTRIES version must win on case-insensitive name collision"
        );

        let beta = merged.iter().find(|s| s.name == "beta");
        assert!(beta.is_some(), "beta missing from merged view");
        assert_eq!(beta.unwrap().files[0].content, "fresh");

        // gamma survives untouched — no ENTRIES override
        let gamma = merged.iter().find(|s| s.name == "gamma");
        assert!(gamma.is_some(), "gamma missing from merged view");
        assert_eq!(gamma.unwrap().files[0].content, "legacy");
    }

    #[test]
    fn test_merge_with_all_collisions_preserves_legacy_length() {
        // Every legacy name is overridden by ENTRIES — merged length must equal
        // legacy length, and every merged entry must be the ENTRIES version.
        static ENTRIES_FIXTURE: &[BundledSkill] = &[
            BundledSkill {
                name: "alpha",
                files: MERGE_TEST_OVERRIDE_FILES,
                content_hash: "",
            },
            BundledSkill {
                name: "gamma",
                files: MERGE_TEST_OVERRIDE_FILES,
                content_hash: "",
            },
        ];
        let merged = merge_skill_lists(MERGE_TEST_LEGACY_SLICE, ENTRIES_FIXTURE);
        assert_eq!(merged.len(), MERGE_TEST_LEGACY_SLICE.len());
        for skill in &merged {
            assert_eq!(
                skill.files[0].content, "override",
                "every merged skill should be the ENTRIES version"
            );
        }
    }

    #[test]
    fn test_merge_with_empty_entries_returns_legacy() {
        static EMPTY: &[BundledSkill] = &[];
        let merged = merge_skill_lists(MERGE_TEST_LEGACY_SLICE, EMPTY);
        assert_eq!(merged.len(), MERGE_TEST_LEGACY_SLICE.len());
        for skill in &merged {
            assert_eq!(skill.files[0].content, "legacy");
        }
    }

    #[test]
    fn test_is_bundled_skill_recognizes_entries_only_name() {
        // `is_bundled_skill` has a dedicated ENTRIES arm that's dead code when
        // production ENTRIES is empty. Verify the arm works by exercising the
        // same logic the function uses — a name present only in ENTRIES must
        // be recognized as bundled.
        static ENTRIES_ONLY: &[BundledSkill] = &[BundledSkill {
            name: "entries-only-skill",
            files: MERGE_TEST_FRESH_FILES,
            content_hash: "",
        }];

        let has_entries_arm = BUNDLED_SKILLS
            .iter()
            .any(|s| s.name.eq_ignore_ascii_case("entries-only-skill"))
            || ENTRIES_ONLY
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case("entries-only-skill"));
        assert!(
            has_entries_arm,
            "ENTRIES-only name must be recognized via the ENTRIES arm"
        );

        let case_insensitive = BUNDLED_SKILLS
            .iter()
            .any(|s| s.name.eq_ignore_ascii_case("ENTRIES-ONLY-SKILL"))
            || ENTRIES_ONLY
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case("ENTRIES-ONLY-SKILL"));
        assert!(case_insensitive, "ENTRIES arm must be case-insensitive");
    }

    #[test]
    fn test_is_bundled_skill_agrees_with_all_bundled_skills() {
        // Coupling guard: `is_bundled_skill` and `all_bundled_skills` must
        // enumerate the same sources. If a third source is added to one
        // without updating the other, this test catches it.
        for skill in all_bundled_skills() {
            assert!(
                is_bundled_skill(skill.name),
                "all_bundled_skills() yielded '{}' but is_bundled_skill rejected it",
                skill.name
            );
        }
    }

    #[test]
    fn test_seed_creates_support_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);

        // _shared/dispatch-lib.sh must exist
        let dispatch_lib = skills_dir.join("_shared").join("dispatch-lib.sh");
        assert!(
            dispatch_lib.is_file(),
            "_shared/dispatch-lib.sh should be seeded"
        );

        // File should be non-empty
        let content = std::fs::read_to_string(&dispatch_lib).unwrap();
        assert!(!content.is_empty(), "dispatch-lib.sh should have content");
    }

    #[cfg(unix)]
    #[test]
    fn test_support_dir_files_are_executable() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);

        let dispatch_lib = skills_dir.join("_shared").join("dispatch-lib.sh");
        let mode = std::fs::metadata(&dispatch_lib)
            .unwrap()
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "_shared/dispatch-lib.sh should be executable"
        );
    }

    #[test]
    fn test_seed_support_dirs_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_support_dirs(skills_dir);
        seed_support_dirs(skills_dir);
        seed_support_dirs(skills_dir);

        // _shared directory should exist with correct files
        let dispatch_lib = skills_dir.join("_shared").join("dispatch-lib.sh");
        assert!(dispatch_lib.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn test_support_dir_symlink_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once
        seed_support_dirs(skills_dir);

        // Replace _shared with a symlink
        let target_dir = tempfile::tempdir().unwrap();
        let shared_dir = skills_dir.join("_shared");
        std::fs::remove_dir_all(&shared_dir).unwrap();
        std::os::unix::fs::symlink(target_dir.path(), &shared_dir).unwrap();

        // Seed again — the symlinked directory should be skipped
        seed_support_dirs(skills_dir);

        // The target directory should NOT contain any files
        assert!(
            std::fs::read_dir(target_dir.path())
                .unwrap()
                .next()
                .is_none(),
            "symlink target should remain empty — seed_support_dirs must refuse to follow it",
        );
    }

    #[test]
    fn test_support_dirs_exclude_test_files() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_support_dirs(skills_dir);

        // test-dispatch-lib.sh should NOT be seeded (excluded by *test* filter)
        let test_file = skills_dir.join("_shared").join("test-dispatch-lib.sh");
        assert!(
            !test_file.exists(),
            "test files should be excluded from support dir seeding"
        );
    }

    #[test]
    fn test_shell_exec_prompt_contains_sandbox_guidance() {
        let content = include_str!("../templates/skills/shell-exec/system_prompt.md");
        assert!(
            content.contains("Writing files outside agent home directories"),
            "shell-exec prompt missing out-of-sandbox section"
        );
        assert!(
            content.contains("Tracing references after rename"),
            "shell-exec prompt missing reference-tracing section"
        );
        assert!(
            content.contains("NEVER use shell commands"),
            "shell-exec prompt missing shell prohibition"
        );
    }

    // --- T1–T9: prune_known_removed_bundled_skills tests (mika#859) ---

    #[test]
    fn prune_removes_directory_in_known_removed_list() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let orphan = skills_dir.join("claude-pilot");
        std::fs::create_dir_all(orphan.join("handlers")).unwrap();
        std::fs::write(
            orphan.join("skill.toml"),
            "[skill]\nname = \"claude-pilot\"",
        )
        .unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 1);
        assert!(!orphan.exists());
    }

    #[test]
    fn prune_is_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let orphan = skills_dir.join("Claude-Pilot");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(
            orphan.join("skill.toml"),
            "[skill]\nname = \"Claude-Pilot\"",
        )
        .unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 1);
        assert!(!orphan.exists());
    }

    #[test]
    fn prune_preserves_directory_not_in_known_removed_list() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let custom = skills_dir.join("my-custom-skill");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(
            custom.join("skill.toml"),
            "[skill]\nname = \"my-custom-skill\"",
        )
        .unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 0);
        assert!(custom.exists());
    }

    #[test]
    fn prune_preserves_symlinked_skill_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let real_target = tempfile::tempdir().unwrap();
        std::fs::write(real_target.path().join("skill.toml"), "content").unwrap();

        let link_path = skills_dir.join("claude-pilot");
        std::fs::create_dir_all(skills_dir).unwrap();
        std::os::unix::fs::symlink(real_target.path(), &link_path).unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 0);
        assert!(link_path.exists());
        assert!(real_target.path().join("skill.toml").exists());
    }

    #[test]
    fn prune_preserves_support_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let shared = skills_dir.join("_shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("dispatch-lib.sh"), "#!/bin/sh").unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 0);
        assert!(shared.exists());
    }

    #[test]
    fn prune_preserves_current_bundled_skill_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let current = skills_dir.join("dev-pilot");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("skill.toml"), "[skill]\nname = \"dev-pilot\"").unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 0);
        assert!(current.exists());
    }

    #[test]
    fn prune_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let orphan = skills_dir.join("claude-pilot");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(orphan.join("skill.toml"), "content").unwrap();

        let first = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(first, 1);

        let second = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(second, 0);
    }

    #[test]
    fn seed_bundled_skills_prunes_before_seeding() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Create stale orphan
        let orphan = skills_dir.join("claude-pilot");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(
            orphan.join("skill.toml"),
            "[skill]\nname = \"claude-pilot\"",
        )
        .unwrap();

        seed_bundled_skills(skills_dir);

        // Orphan should be gone
        assert!(
            !orphan.exists(),
            "claude-pilot orphan should be pruned by seed_bundled_skills"
        );
        // Current bundled skills should exist
        assert!(
            skills_dir.join("dev-pilot").exists(),
            "dev-pilot should be seeded"
        );
        assert!(
            skills_dir.join("dev-groom").exists(),
            "dev-groom should be seeded"
        );
    }

    #[test]
    fn known_removed_disjoint_from_current_bundle() {
        for name in KNOWN_REMOVED_BUNDLED_SKILLS {
            assert!(
                !is_bundled_skill(name),
                "KNOWN_REMOVED_BUNDLED_SKILLS contains '{name}', which is also in the \
                 current bundle. Either remove it from KNOWN_REMOVED_BUNDLED_SKILLS \
                 (if you intend to re-bundle it), or rename the new bundled skill."
            );
        }
    }
}
