//! Compile-time embedded skill templates, seeded into agent skill directories on startup.
//!
//! Each skill is a set of files (skill.toml, tools.json, optional system_prompt.md,
//! optional handler scripts) embedded via `include_str!`. On first run, these are
//! written to `{agent_home}/skills/{skill_name}/` if the directory doesn't already exist.

use std::path::Path;
use tracing::{info, warn};

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
    ("skill.toml" => "../../../templates/skills/tmux/skill.toml"),
    ("system_prompt.md" => "../../../templates/skills/tmux/system_prompt.md"),
    ("tools.json" => "../../../templates/skills/tmux/tools.json"),
    ("handlers/create_session.sh" => "../../../templates/skills/tmux/handlers/create_session.sh", +x),
    ("handlers/kill_session.sh" => "../../../templates/skills/tmux/handlers/kill_session.sh", +x),
    ("handlers/list_sessions.sh" => "../../../templates/skills/tmux/handlers/list_sessions.sh", +x),
    ("handlers/read_output.sh" => "../../../templates/skills/tmux/handlers/read_output.sh", +x),
    ("handlers/send_command.sh" => "../../../templates/skills/tmux/handlers/send_command.sh", +x),
    ("handlers/wait_for_text.sh" => "../../../templates/skills/tmux/handlers/wait_for_text.sh", +x),
]);

static SHELL_EXEC_SKILL: BundledSkill = skill!("shell-exec", [
    ("skill.toml" => "../../../templates/skills/shell-exec/skill.toml"),
    ("system_prompt.md" => "../../../templates/skills/shell-exec/system_prompt.md"),
    ("tools.json" => "../../../templates/skills/shell-exec/tools.json"),
    ("handlers/run.sh" => "../../../templates/skills/shell-exec/handlers/run.sh", +x),
]);

static WEB_SEARCH_SKILL: BundledSkill = skill!("web-search", [
    ("skill.toml" => "../../../templates/skills/web-search/skill.toml"),
    ("system_prompt.md" => "../../../templates/skills/web-search/system_prompt.md"),
    ("tools.json" => "../../../templates/skills/web-search/tools.json"),
    ("handlers/search.sh" => "../../../templates/skills/web-search/handlers/search.sh", +x),
]);

static FILE_READER_SKILL: BundledSkill = skill!("file-reader", [
    ("skill.toml" => "../../../templates/skills/file-reader/skill.toml"),
    ("system_prompt.md" => "../../../templates/skills/file-reader/system_prompt.md"),
    ("tools.json" => "../../../templates/skills/file-reader/tools.json"),
    ("handlers/read.sh" => "../../../templates/skills/file-reader/handlers/read.sh", +x),
]);

static CALENDAR_SKILL: BundledSkill = skill!("calendar", [
    ("skill.toml" => "../../../templates/skills/calendar/skill.toml"),
    ("system_prompt.md" => "../../../templates/skills/calendar/system_prompt.md"),
    ("tools.json" => "../../../templates/skills/calendar/tools.json"),
]);

/// All bundled skills.
static BUNDLED_SKILLS: &[&BundledSkill] = &[
    &TMUX_SKILL,
    &SHELL_EXEC_SKILL,
    &WEB_SEARCH_SKILL,
    &FILE_READER_SKILL,
    &CALENDAR_SKILL,
];

/// Seed bundled skills into the given skills directory.
///
/// For each bundled skill, if its directory already exists, it is skipped (never overwritten).
/// On partial failure, the partially created directory is removed and the next skill is attempted.
pub fn seed_bundled_skills(skills_dir: &Path) {
    for skill in BUNDLED_SKILLS {
        let skill_dir = skills_dir.join(skill.name);

        if skill_dir.exists() {
            continue;
        }

        if let Err(e) = write_skill(&skill_dir, skill) {
            warn!(skill = skill.name, error = %e, "failed to seed bundled skill, removing partial dir");
            let _ = std::fs::remove_dir_all(&skill_dir);
        } else {
            info!(skill = skill.name, "seeded bundled skill");
        }
    }
}

/// Write all files for a single skill into the given directory.
fn write_skill(skill_dir: &Path, skill: &BundledSkill) -> std::io::Result<()> {
    for file in skill.files {
        let file_path = skill_dir.join(file.path);

        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
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

        for skill in BUNDLED_SKILLS {
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
    fn test_seed_does_not_overwrite_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once
        seed_bundled_skills(skills_dir);

        // Modify a file in the tmux skill
        let marker = skills_dir.join("tmux").join("skill.toml");
        std::fs::write(&marker, "custom content").unwrap();

        // Seed again — should not overwrite
        seed_bundled_skills(skills_dir);

        let content = std::fs::read_to_string(&marker).unwrap();
        assert_eq!(content, "custom content");
    }

    #[test]
    fn test_seed_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);
        seed_bundled_skills(skills_dir);
        seed_bundled_skills(skills_dir);

        // All skills still present
        for skill in BUNDLED_SKILLS {
            assert!(skills_dir.join(skill.name).is_dir());
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_handlers_are_executable() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);

        for skill in BUNDLED_SKILLS {
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
}
