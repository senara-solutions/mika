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
#[allow(dead_code)]
pub struct DiscoveredSkill {
    /// Skill directory basename (used as the skill name).
    pub name: String,
    /// Absolute path to the skill directory. Used by `build.rs` to emit
    /// `cargo:rerun-if-changed` for the skill directory itself (so adding a
    /// file inside the dir triggers a rebuild without requiring the parent
    /// `skills/bundled/` mtime to change).
    pub abs_dir: PathBuf,
    /// Files belonging to this skill, in stable alphabetical order.
    pub files: Vec<DiscoveredFile>,
}

/// A single discovered support directory (underscore-prefixed, non-skill).
///
/// `#[allow(dead_code)]` for the same reason as `DiscoveredFile` — this module
/// is `include!`d from multiple compilation units (build.rs + integration tests).
#[allow(dead_code)]
pub struct DiscoveredSupportDir {
    /// Directory basename (e.g. "_shared").
    pub name: String,
    /// Absolute path to the support directory.
    pub abs_dir: PathBuf,
    /// Files belonging to this support directory, in stable alphabetical order.
    pub files: Vec<DiscoveredFile>,
}

/// Walk `base` and return underscore-prefixed support directories sorted alphabetically.
///
/// Rules:
/// - Missing or non-directory `base` returns an empty vec (no panic).
/// - Only immediate subdirectories starting with `_` are considered.
/// - Symlinks at the directory level are skipped (defense-in-depth).
/// - Dotfile directories are skipped.
/// - Files are collected recursively (support dirs may have shallow nesting).
/// - `.sh` files are marked `executable: true`.
/// - Files matching `*test*` (case-insensitive) are excluded (test fixtures).
#[allow(dead_code)]
pub fn discover_support_dirs(base: &Path) -> Vec<DiscoveredSupportDir> {
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
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        // Only underscore-prefixed directories
        if !name_str.starts_with('_') {
            continue;
        }
        dirs.push(entry.path());
    }
    dirs.sort();

    let mut support_dirs = Vec::with_capacity(dirs.len());
    for dir in dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| panic!("non-utf8 support dir: {}", dir.display()))
            .to_string();

        let mut files = collect_support_dir_files(&dir, "");
        files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        support_dirs.push(DiscoveredSupportDir {
            name,
            abs_dir: dir,
            files,
        });
    }
    support_dirs
}

/// Collect files from a support directory recursively.
/// Skips symlinks, dotfiles, and test files (case-insensitive *test* match).
#[allow(dead_code)]
fn collect_support_dir_files(dir: &Path, rel_prefix: &str) -> Vec<DiscoveredFile> {
    let mut files = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => panic!("failed to read {}: {e}", dir.display()),
    };

    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("failed to read entry in {}: {e}", dir.display()));
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if name.starts_with('.') {
            continue;
        }

        // Exclude test files (test fixtures, not production runtime dependencies)
        if name.to_ascii_lowercase().contains("test") {
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
            // Recurse into subdirectories
            files.extend(collect_support_dir_files(&abs_path, &rel_path));
            continue;
        }

        let executable = rel_path.ends_with(".sh");
        files.push(DiscoveredFile {
            rel_path,
            abs_path,
            executable,
        });
    }

    files
}

/// Returns `true` if `name` is a valid bundled-skill directory name.
///
/// Rejects empty names, dotfile prefixes (`.`), and underscore prefixes (`_`).
/// Used by both build-time discovery ([`discover_bundled_skills`]) and runtime
/// skill scanning (`scan_skills_dir` in `src/skills/index.rs`).
#[allow(dead_code)]
pub fn is_bundled_skill_dir(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.starts_with('_')
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
        let name_str = name.to_string_lossy();
        // Skip dotfile directories (.gitkeep, .DS_Store, etc.)
        if name_str.starts_with('.') {
            continue;
        }
        // Skip underscore-prefixed directories (_shared/, _templates/, etc.)
        // These are convention-reserved for non-skill support directories.
        if name_str.starts_with('_') {
            continue;
        }
        let path = entry.path();
        // Use symlink_metadata on the skill.toml path to stay consistent with
        // the collector's symlink-skipping behavior below. A skill.toml that
        // is itself a symlink shouldn't count a directory as a valid skill.
        let toml_path = path.join("skill.toml");
        let toml_is_real_file = fs::symlink_metadata(&toml_path)
            .map(|m| m.file_type().is_file())
            .unwrap_or(false);
        if !toml_is_real_file {
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
        skills.push(DiscoveredSkill {
            name,
            abs_dir: skill_dir,
            files,
        });
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
