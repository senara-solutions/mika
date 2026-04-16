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
}

/// Declare a bundled skill with its files.
/// Use `+x` suffix to mark a file as executable (handler scripts).
macro_rules! skill {
    ($name:expr, [ $( $entry:tt ),+ $(,)? ]) => {
        BundledSkill {
            name: $name,
            files: &[
                $( skill!(@file $entry), )+
            ],
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

static SKILL_REVIEW_SKILL: BundledSkill = skill!("skill-review", [
    ("skill.toml" => "../templates/skills/skill-review/skill.toml"),
    ("system_prompt.md" => "../templates/skills/skill-review/system_prompt.md"),
    ("tools.json" => "../templates/skills/skill-review/tools.json"),
]);

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

static AGENTS_TEAMS_SKILL: BundledSkill = skill!("agents-teams", [
    ("skill.toml" => "../templates/skills/agents-teams/skill.toml"),
    ("system_prompt.md" => "../templates/skills/agents-teams/system_prompt.md"),
    ("tools.json" => "../templates/skills/agents-teams/tools.json"),
]);

/// All bundled skills.
static BUNDLED_SKILLS: &[&BundledSkill] = &[
    &TMUX_SKILL,
    &SHELL_EXEC_SKILL,
    &WEB_SEARCH_SKILL,
    &FILE_READER_SKILL,
    &SKILL_REVIEW_SKILL,
    &SELF_KNOWLEDGE_SKILL,
    &GIT_OPS_SKILL,
    &GOOGLE_WORKSPACE_SKILL,
    &GITHUB_SKILL,
    &MCP_SKILL,
    &BROWSER_CONTROL_SKILL,
    &AGENTS_TEAMS_SKILL,
];

// Directory-sourced bundled skills, generated at build time by `build.rs`
// walking `<workspace>/skills/bundled/`. Declares `static ENTRIES: &[BundledSkill]`.
// An empty or missing `skills/bundled/` directory yields `ENTRIES = &[]` — the
// parallel path stays silent until a future migration ticket starts populating it.
include!(concat!(env!("OUT_DIR"), "/bundled_skills_generated.rs"));

/// Merge the legacy hardcoded `BUNDLED_SKILLS` list with the directory-sourced
/// `ENTRIES` table. Entries from `ENTRIES` win on case-insensitive name collision.
///
/// Returns a deduplicated `Vec<&'static BundledSkill>`. Zero collisions are
/// expected in production during this refactor (ENTRIES is empty). The merge
/// semantics exist so a future migration ticket can move skills one-by-one
/// without orchestrating a coordinated cutover.
fn all_bundled_skills() -> Vec<&'static BundledSkill> {
    let mut merged: Vec<&'static BundledSkill> = BUNDLED_SKILLS.to_vec();
    for entry in ENTRIES {
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

/// Check whether a skill name matches a bundled (built-in) skill.
///
/// Consults both the legacy hardcoded list and the directory-sourced `ENTRIES`
/// table so install/uninstall/update guards treat either source as immutable.
pub fn is_bundled_skill(name: &str) -> bool {
    BUNDLED_SKILLS
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case(name))
        || ENTRIES.iter().any(|s| s.name.eq_ignore_ascii_case(name))
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

    for file in skill.files {
        let file_path = skill_dir.join(file.path);

        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Refuse to overwrite a file that is a symlink (same defense-in-depth).
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
        let content = include_str!("../templates/skills/skill-review/system_prompt.md");
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
        let content = include_str!("../templates/skills/skill-review/system_prompt.md");
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

    #[test]
    fn test_merge_prefers_entries_on_collision() {
        // Build synthetic legacy and ENTRIES-like inputs and apply the same
        // merge logic to verify the invariant. This is a unit test for the
        // semantics; real ENTRIES stays empty in production this PR.
        fn merge(
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

        static LEGACY_FILES: &[SkillFile] = &[SkillFile {
            path: "skill.toml",
            content: "legacy",
            executable: false,
        }];
        static LEGACY: BundledSkill = BundledSkill {
            name: "alpha",
            files: LEGACY_FILES,
        };
        static LEGACY_SLICE: &[&BundledSkill] = &[&LEGACY];

        static OVERRIDE_FILES: &[SkillFile] = &[SkillFile {
            path: "skill.toml",
            content: "override",
            executable: false,
        }];
        static FRESH_FILES: &[SkillFile] = &[SkillFile {
            path: "skill.toml",
            content: "fresh",
            executable: false,
        }];
        static ENTRIES_FIXTURE: &[BundledSkill] = &[
            BundledSkill {
                name: "ALPHA", // case-insensitive collision with LEGACY
                files: OVERRIDE_FILES,
            },
            BundledSkill {
                name: "beta", // new addition
                files: FRESH_FILES,
            },
        ];

        let merged = merge(LEGACY_SLICE, ENTRIES_FIXTURE);
        assert_eq!(merged.len(), 2);

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
    }

    #[test]
    fn test_production_entries_is_empty_until_migration() {
        // Guard against accidentally shipping production bundled skills before
        // the migration ticket lands. The single point of change is `skills/bundled/`
        // at the workspace root — it must stay empty (beyond `.gitkeep`) in this PR.
        assert!(
            ENTRIES.is_empty(),
            "ENTRIES is non-empty ({} skill(s)). If this was intentional, this guard \
             should move to the migration PR. Otherwise, remove files from \
             `skills/bundled/` at the workspace root.",
            ENTRIES.len()
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
}
