//! Shared discovery logic for bundled skills.
//!
//! Included by both `build.rs` (to generate the `ENTRIES` table) and integration
//! tests (to verify the logic against a fixture tree). Lives outside `src/` so
//! `build.rs` can consume it without a circular dependency, and is pulled into
//! tests via `#[path = "..."]` mod attributes.
//!
//! Pure function: takes a base path, returns a sorted list of skill descriptors.
//! No side effects beyond filesystem reads. Panics on I/O errors (desirable in
//! build context — a failed walk should fail the build, not emit empty output).

use std::fs;
use std::path::{Path, PathBuf};

/// Single file within a bundled skill (discovery-time descriptor).
///
/// `#[allow(dead_code)]` because this module is `include!`d from both `build.rs`
/// (which reads every field) and integration tests (which may read only some).
/// Each consumer is a separate compilation unit, so unused-field warnings fire
/// per-consumer even when every field is used in aggregate.
#[allow(dead_code)]
pub struct DiscoveredFile {
    /// Relative path within the skill directory (e.g. "skill.toml", "handlers/run.sh").
    pub rel_path: String,
    /// Absolute path on disk (for `include_str!` / `include_bytes!` embedding).
    pub abs_path: PathBuf,
    /// Whether the file should be marked executable at seed time.
    pub executable: bool,
}

/// A single discovered bundled skill.
pub struct DiscoveredSkill {
    /// Skill directory basename (used as the skill name).
    pub name: String,
    /// Files belonging to this skill, in stable alphabetical order.
    pub files: Vec<DiscoveredFile>,
}

/// Walk `base` and return bundled skills sorted alphabetically by name.
///
/// Rules:
/// - Missing or non-directory `base` returns an empty vec (no panic).
/// - Only immediate subdirectories are considered skill candidates.
/// - A subdirectory is only a skill if it contains a `skill.toml` file.
/// - Symlinks at the skill-directory level are skipped (defense-in-depth).
/// - Dotfile directories (e.g. `.gitkeep` as a dir) are skipped.
/// - File discovery is shallow plus one level of `handlers/` nesting.
/// - Handler files matching `handlers/*.sh` are marked `executable: true`.
pub fn discover_bundled_skills(base: &Path) -> Vec<DiscoveredSkill> {
    if !base.exists() || !base.is_dir() {
        return Vec::new();
    }

    let mut dirs: Vec<PathBuf> = Vec::new();
    let entries =
        fs::read_dir(base).unwrap_or_else(|e| panic!("failed to read {}: {e}", base.display()));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("failed to read entry in {}: {e}", base.display()));
        let file_type = entry
            .file_type()
            .unwrap_or_else(|e| panic!("cannot stat {}: {e}", entry.path().display()));
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if !path.join("skill.toml").is_file() {
            continue;
        }
        dirs.push(path);
    }
    dirs.sort();

    let mut skills = Vec::with_capacity(dirs.len());
    for skill_dir in dirs {
        let name = skill_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| panic!("non-utf8 skill dir: {}", skill_dir.display()))
            .to_string();

        let mut files = collect_skill_files(&skill_dir, "");
        files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        skills.push(DiscoveredSkill { name, files });
    }
    skills
}

/// Collect files from a skill directory. Shallow plus one level of `handlers/`
/// nesting. Skips symlinks and dotfiles.
fn collect_skill_files(skill_dir: &Path, rel_prefix: &str) -> Vec<DiscoveredFile> {
    let mut files = Vec::new();
    let entries = match fs::read_dir(skill_dir) {
        Ok(entries) => entries,
        Err(e) => panic!("failed to read {}: {e}", skill_dir.display()),
    };

    for entry in entries {
        let entry = entry
            .unwrap_or_else(|e| panic!("failed to read entry in {}: {e}", skill_dir.display()));
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if name.starts_with('.') {
            continue;
        }

        let file_type = entry
            .file_type()
            .unwrap_or_else(|e| panic!("cannot stat {}: {e}", entry.path().display()));
        if file_type.is_symlink() {
            continue;
        }

        let abs_path = entry.path();
        let rel_path = if rel_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{rel_prefix}/{name}")
        };

        if file_type.is_dir() {
            // Recurse into `handlers/` exactly once — matches today's template layout.
            if rel_prefix.is_empty() && name == "handlers" {
                files.extend(collect_skill_files(&abs_path, "handlers"));
            }
            continue;
        }

        let executable = rel_path.starts_with("handlers/") && rel_path.ends_with(".sh");
        files.push(DiscoveredFile {
            rel_path,
            abs_path,
            executable,
        });
    }

    files
}
