//! Core install/uninstall/update operations for marketplace skills.
//!
//! These functions handle filesystem operations and lock file updates.
//! CLI interaction (prompts, output formatting) belongs in `mika-cli`.

use std::collections::{HashSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::Utc;

use super::git;
use super::marketplace::{
    MarketplaceEntry, SkillCandidate, is_local_source, read_lock, scan_repo_for_skills,
    url_to_local_path, write_lock,
};
use crate::bundled_skills::is_bundled_skill;
use crate::tools::create_skill::validate_skill_name;

/// Maximum dependency depth to prevent pathological chains.
const MAX_DEP_DEPTH: usize = 10;

/// A resolved dependency ready for installation.
#[derive(Debug, Clone)]
pub struct ResolvedDep {
    /// The skill candidate to install.
    pub candidate: SkillCandidate,
    /// Depth in the dependency tree (0 = root, 1 = direct dep, 2+ = transitive).
    pub depth: usize,
    /// Name of the skill that depends on this one.
    pub required_by: String,
}

/// Resolve the full transitive dependency tree for a set of skill candidates.
///
/// Returns an ordered list of `ResolvedDep` in install order (dependencies first).
///
/// Resolution strategy per dependency name:
/// 1. Already installed in agent's `skills_dir` → skip (passthrough)
/// 2. Bundled skill name → skip (always available)
/// 3. Found in `available_candidates` (same-source siblings) → include for install
/// 4. Not found → error with clear message
///
/// Cycle detection: tracks the resolution chain, errors with full cycle path.
/// Max depth: 10 levels (guard against pathological chains).
pub fn resolve_dependencies(
    roots: &[&SkillCandidate],
    available_candidates: &[SkillCandidate],
    skills_dir: &Path,
) -> Result<Vec<ResolvedDep>> {
    let mut result: Vec<ResolvedDep> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();

    // Mark roots as visited so they are not pulled in as deps of each other
    for root in roots {
        visited.insert(root.name.to_lowercase());
    }

    // BFS: process roots, then their deps breadth-first
    let mut queue: VecDeque<(String, usize, Vec<String>)> = VecDeque::new();

    // Seed queue with direct dependencies of all roots
    for root in roots {
        for dep_name in &root.dependencies {
            let dep_lower = dep_name.to_lowercase();
            // Self-dependency check
            if dep_lower == root.name.to_lowercase() {
                bail!(
                    "Skill \"{}\" lists itself as a dependency. Remove the self-reference from skill.toml.",
                    root.name
                );
            }
            if !visited.contains(&dep_lower) {
                queue.push_back((
                    dep_name.clone(),
                    1,
                    vec![root.name.clone(), dep_name.clone()],
                ));
            }
        }
    }

    while let Some((dep_name, depth, chain)) = queue.pop_front() {
        let dep_lower = dep_name.to_lowercase();

        // Cycle detection FIRST: check if dep_name appears earlier in the chain
        // (before the last entry), excluding the root at index 0 since roots are
        // being installed directly and a dep pointing back to a root is fine.
        // This must run before the visited check so mutual dependency cycles are caught
        // even when one node was already visited via a different BFS path.
        let cycle_in_deps = chain.len() > 2
            && chain[1..chain.len() - 1]
                .iter()
                .any(|n| n.to_lowercase() == dep_lower);
        if cycle_in_deps {
            bail!("Circular dependency detected: {}", chain.join(" -> "));
        }

        // Depth guard
        if depth > MAX_DEP_DEPTH {
            bail!(
                "Dependency chain exceeds maximum depth ({MAX_DEP_DEPTH}). Chain: {}",
                chain.join(" -> ")
            );
        }

        // Already resolved (handles diamonds and root-as-dep-of-dep)
        if visited.contains(&dep_lower) {
            continue;
        }

        // 1. Already installed → skip
        let installed_dir = skills_dir.join(&dep_name);
        if installed_dir.exists() || installed_dir.symlink_metadata().is_ok() {
            visited.insert(dep_lower);
            continue;
        }

        // 2. Bundled skill → skip
        if is_bundled_skill(&dep_name) {
            visited.insert(dep_lower);
            continue;
        }

        // 3. Find in available candidates (case-insensitive)
        let found = available_candidates
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(&dep_name));

        match found {
            Some(candidate) => {
                let parent = chain
                    .get(chain.len().saturating_sub(2))
                    .cloned()
                    .unwrap_or_default();
                visited.insert(dep_lower);
                result.push(ResolvedDep {
                    candidate: candidate.clone(),
                    depth,
                    required_by: parent,
                });

                // Enqueue this candidate's own dependencies
                for sub_dep in &candidate.dependencies {
                    let sub_lower = sub_dep.to_lowercase();
                    if sub_lower == candidate.name.to_lowercase() {
                        bail!(
                            "Skill \"{}\" lists itself as a dependency. Remove the self-reference from skill.toml.",
                            candidate.name
                        );
                    }
                    let mut new_chain = chain.clone();
                    new_chain.push(sub_dep.clone());
                    queue.push_back((sub_dep.clone(), depth + 1, new_chain));
                }
            }
            None => {
                let parent = chain
                    .get(chain.len().saturating_sub(2))
                    .cloned()
                    .unwrap_or_default();
                bail!(
                    "Dependency \"{}\" required by \"{}\" not found in source or as bundled skill. Install it manually first.",
                    dep_name,
                    parent
                );
            }
        }
    }

    Ok(result)
}

/// Find orphaned dependencies after uninstalling a skill.
///
/// Returns the names of skills that were installed as dependencies of `removed_skill`
/// and are no longer needed by any other installed skill.
pub fn find_orphaned_deps(agent_home: &Path, removed_skill: &str) -> Vec<String> {
    let mut lock = read_lock(agent_home);
    let mut orphans = Vec::new();

    for (name, entry) in &mut lock.skills {
        if entry
            .installed_as_dependency_of
            .contains(&removed_skill.to_string())
        {
            // Remove the uninstalled skill from the dependency_of list
            let remaining: Vec<String> = entry
                .installed_as_dependency_of
                .iter()
                .filter(|s| s != &removed_skill)
                .cloned()
                .collect();

            if remaining.is_empty() {
                // This skill was only a dependency of the removed skill → orphan
                orphans.push(name.clone());
            }
        }
    }

    orphans
}

/// Remove a skill from the `installed_as_dependency_of` lists in the lock file.
///
/// Called after uninstalling a skill to clean up dependency tracking.
pub fn remove_from_dependency_lists(agent_home: &Path, removed_skill: &str) -> Result<()> {
    let mut lock = read_lock(agent_home);
    let mut changed = false;

    for entry in lock.skills.values_mut() {
        if entry
            .installed_as_dependency_of
            .contains(&removed_skill.to_string())
        {
            entry
                .installed_as_dependency_of
                .retain(|s| s != removed_skill);
            changed = true;
        }
    }

    if changed {
        write_lock(agent_home, &lock)?;
    }

    Ok(())
}

/// Mark a skill as a dependency of another skill in the lock file.
///
/// If the skill already has a lock entry, appends the parent (diamond case).
/// If the skill is manually installed (empty vec), it stays manually installed.
pub fn mark_as_dependency(agent_home: &Path, skill_name: &str, parent_name: &str) -> Result<()> {
    let mut lock = read_lock(agent_home);

    if let Some(entry) = lock.skills.get_mut(skill_name)
        && !entry
            .installed_as_dependency_of
            .contains(&parent_name.to_string())
    {
        entry
            .installed_as_dependency_of
            .push(parent_name.to_string());
        write_lock(agent_home, &lock)?;
    }

    Ok(())
}

/// Result of a successful skill installation.
#[derive(Debug)]
pub struct InstallResult {
    pub name: String,
    pub url: String,
    pub commit: String,
    pub has_exec_handlers: bool,
    /// Whether the skill was installed as a symlink.
    pub linked: bool,
}

/// Result of a successful skill update.
#[derive(Debug)]
pub enum UpdateResult {
    /// Skill was updated (git or local snapshot re-copy).
    Updated {
        name: String,
        old_commit: String,
        new_commit: String,
    },
    /// Linked skill — no-op, source changes are always current.
    LinkedNoOp { name: String },
}

/// Install a single skill from a scanned candidate into the agent's skills directory.
///
/// - Validates the install name
/// - Checks for collisions with bundled and existing skills
/// - Copies the skill directory (excluding `.git`, with symlink escape checks)
/// - Updates the marketplace lock file
pub fn install_skill(
    agent_home: &Path,
    skills_dir: &Path,
    candidate: &SkillCandidate,
    install_name: Option<&str>,
    url: &str,
    commit: &str,
) -> Result<InstallResult> {
    install_skill_inner(
        agent_home,
        skills_dir,
        candidate,
        install_name,
        url,
        commit,
        false,
    )
}

/// Install a single skill as a symlink (--link mode).
///
/// Creates an absolute symlink from the skills directory to the source directory.
/// The symlink always points to the directory containing `skill.toml`.
pub fn install_skill_linked(
    agent_home: &Path,
    skills_dir: &Path,
    candidate: &SkillCandidate,
    install_name: Option<&str>,
    url: &str,
) -> Result<InstallResult> {
    install_skill_inner(
        agent_home,
        skills_dir,
        candidate,
        install_name,
        url,
        "",
        true,
    )
}

fn install_skill_inner(
    agent_home: &Path,
    skills_dir: &Path,
    candidate: &SkillCandidate,
    install_name: Option<&str>,
    url: &str,
    commit: &str,
    linked: bool,
) -> Result<InstallResult> {
    let name = install_name.unwrap_or(&candidate.name);

    // Validate name
    validate_skill_name(name).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Check bundled collision
    if is_bundled_skill(name) {
        bail!(
            "'{name}' collides with a built-in skill. Use --name <alias> to install under a different name."
        );
    }

    // Check existing skill directory (or symlink)
    let target_dir = skills_dir.join(name);
    if target_dir.exists() || target_dir.symlink_metadata().is_ok() {
        bail!(
            "Skill '{name}' already exists. Use --name for a different name, or `mika skills update` to update."
        );
    }

    // Reject self-referential installs (source inside skills_dir)
    let canonical_src = std::fs::canonicalize(&candidate.absolute_path)
        .unwrap_or_else(|_| candidate.absolute_path.clone());
    let canonical_skills_dir =
        std::fs::canonicalize(skills_dir).unwrap_or_else(|_| skills_dir.to_path_buf());
    if canonical_src.starts_with(&canonical_skills_dir) {
        bail!(
            "Cannot install a skill from inside the skills directory. \
             Source '{}' is under '{}'.",
            candidate.absolute_path.display(),
            skills_dir.display()
        );
    }

    if linked {
        // Create symlink
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&candidate.absolute_path, &target_dir).with_context(
                || {
                    format!(
                        "failed to create symlink {} -> {}",
                        target_dir.display(),
                        candidate.absolute_path.display()
                    )
                },
            )?;
        }
        #[cfg(not(unix))]
        {
            bail!("--link is only supported on Unix systems");
        }
    } else {
        // Copy skill directory
        copy_skill_dir(&candidate.absolute_path, &target_dir)?;
    }

    // Update lock file
    let mut lock = read_lock(agent_home);
    let now = Utc::now().to_rfc3339();
    lock.skills.insert(
        name.to_string(),
        MarketplaceEntry {
            url: url.to_string(),
            path: candidate.relative_path.clone(),
            commit: commit.to_string(),
            linked,
            installed_as_dependency_of: vec![],
            installed_at: now.clone(),
            updated_at: now,
        },
    );
    write_lock(agent_home, &lock)?;

    Ok(InstallResult {
        name: name.to_string(),
        url: url.to_string(),
        commit: commit.to_string(),
        has_exec_handlers: candidate.has_exec_handlers,
        linked,
    })
}

/// Uninstall a marketplace skill.
///
/// Returns an error if the skill is bundled, not marketplace-installed, or not found.
/// Handles both regular directories and symlinks (from --link installs).
pub fn uninstall_skill(agent_home: &Path, skills_dir: &Path, name: &str) -> Result<()> {
    // Protect bundled skills
    if is_bundled_skill(name) {
        bail!(
            "Cannot uninstall built-in skill '{name}'. Use `mika skills disable {name}` instead."
        );
    }

    let mut lock = read_lock(agent_home);
    let skill_dir = skills_dir.join(name);
    let in_lock = lock.skills.contains_key(name);

    if !in_lock {
        // Check both real dirs and broken symlinks
        let exists = skill_dir.exists() || skill_dir.symlink_metadata().is_ok();
        if exists {
            bail!(
                "Skill '{name}' is not a marketplace skill. Remove it manually or use the delete_skill agent tool."
            );
        } else {
            bail!("Skill '{name}' not found.");
        }
    }

    // Lock entry exists but directory/symlink doesn't — clean up stale entry
    let meta_exists = skill_dir.exists() || skill_dir.symlink_metadata().is_ok();
    if !meta_exists {
        lock.skills.remove(name);
        write_lock(agent_home, &lock)?;
        bail!("Cleaned up stale lock entry for '{name}' (directory was already removed).");
    }

    // Check if it's a symlink (from --link install)
    let is_symlink = skill_dir
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);

    if is_symlink {
        // Linked skill: remove symlink only (not the target directory)
        std::fs::remove_file(&skill_dir)
            .with_context(|| format!("failed to remove symlink {}", skill_dir.display()))?;
    } else {
        // Regular directory: existing verify + remove_dir_all
        crate::tools::create_skill::verify_skill_path(skills_dir, &skill_dir)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        std::fs::remove_dir_all(&skill_dir)
            .with_context(|| format!("failed to remove {}", skill_dir.display()))?;
    }

    // Update lock file
    lock.skills.remove(name);
    write_lock(agent_home, &lock)?;

    Ok(())
}

/// Update a single marketplace skill to the latest version.
///
/// Dispatch:
/// - Linked skills → no-op (source changes are always current)
/// - Local snapshot skills → re-copy from original path
/// - Git skills → re-clone and compare commit hashes
pub fn update_skill(agent_home: &Path, skills_dir: &Path, name: &str) -> Result<UpdateResult> {
    let mut lock = read_lock(agent_home);
    let entry = lock
        .skills
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Skill '{name}' is not a marketplace-installed skill."))?
        .clone();

    // Linked skills: no-op
    if entry.linked {
        return Ok(UpdateResult::LinkedNoOp {
            name: name.to_string(),
        });
    }

    // Local snapshot: re-copy from original path
    if is_local_source(&entry) {
        let src = url_to_local_path(&entry.url).ok_or_else(|| {
            anyhow::anyhow!("invalid local source URL in lock file: {}", entry.url)
        })?;
        if !src.exists() {
            bail!(
                "Source directory '{}' no longer exists. \
                 The installed snapshot is still available. \
                 Remove and reinstall from the new location.",
                src.display()
            );
        }
        let target_dir = skills_dir.join(name);
        if target_dir.exists() {
            crate::tools::create_skill::verify_skill_path(skills_dir, &target_dir)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            std::fs::remove_dir_all(&target_dir)
                .with_context(|| format!("failed to remove {}", target_dir.display()))?;
        }
        copy_skill_dir(&src, &target_dir)?;

        // Update lock timestamp
        if let Some(entry) = lock.skills.get_mut(name) {
            entry.updated_at = Utc::now().to_rfc3339();
        }
        write_lock(agent_home, &lock)?;

        return Ok(UpdateResult::Updated {
            name: name.to_string(),
            old_commit: String::new(),
            new_commit: String::new(),
        });
    }

    // Git source: re-clone and compare
    let tmp = git::clone_to_temp(&entry.url)
        .with_context(|| format!("failed to clone {} for update", entry.url))?;

    let candidates = scan_repo_for_skills(tmp.path());
    let candidate = candidates
        .iter()
        .find(|c| c.relative_path == entry.path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Skill not found at path '{}' in repo. It may have been moved or removed.",
                entry.path
            )
        })?;

    let new_commit = git::get_head_commit(tmp.path())?;
    if new_commit == entry.commit {
        return Ok(UpdateResult::Updated {
            name: name.to_string(),
            old_commit: entry.commit.clone(),
            new_commit: entry.commit.clone(),
        });
    }

    let old_commit = entry.commit.clone();
    let target_dir = skills_dir.join(name);

    if target_dir.exists() {
        crate::tools::create_skill::verify_skill_path(skills_dir, &target_dir)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        std::fs::remove_dir_all(&target_dir)
            .with_context(|| format!("failed to remove {}", target_dir.display()))?;
    }

    copy_skill_dir(&candidate.absolute_path, &target_dir)?;

    if let Some(entry) = lock.skills.get_mut(name) {
        entry.commit = new_commit.clone();
        entry.updated_at = Utc::now().to_rfc3339();
    }
    write_lock(agent_home, &lock)?;

    Ok(UpdateResult::Updated {
        name: name.to_string(),
        old_commit,
        new_commit,
    })
}

/// Recursively copy a skill directory, excluding `.git` directories.
///
/// Validates that no symlinks escape the source directory.
fn copy_skill_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;

    let src_canonical = src
        .canonicalize()
        .with_context(|| format!("failed to canonicalize source {}", src.display()))?;

    copy_dir_recursive(&src_canonical, src, dst)
}

fn copy_dir_recursive(src_root: &Path, src: &Path, dst: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src)
        .with_context(|| format!("failed to read directory {}", src.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip .git directory (keep .gitignore and .gitattributes — they're harmless)
        if name_str == ".git" {
            continue;
        }

        let src_path = entry.path();
        let dst_path = dst.join(&name);

        // Symlink escape check: canonicalize and verify it's under src_root
        let canonical = src_path.canonicalize().with_context(|| {
            format!(
                "failed to resolve path {} (possible broken symlink)",
                src_path.display()
            )
        })?;
        if !canonical.starts_with(src_root) {
            bail!(
                "Symlink escape detected: {} points outside the skill directory. Aborting.",
                src_path.display()
            );
        }

        let ft = entry.file_type()?;
        if ft.is_dir() {
            std::fs::create_dir_all(&dst_path)
                .with_context(|| format!("failed to create directory {}", dst_path.display()))?;
            copy_dir_recursive(src_root, &src_path, &dst_path)?;
        } else if ft.is_file() {
            std::fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "failed to copy {} -> {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;

            // Preserve execute permissions on Unix
            #[cfg(unix)]
            {
                let src_meta = std::fs::metadata(&src_path)?;
                std::fs::set_permissions(&dst_path, src_meta.permissions())?;
            }
        }
        // Skip symlinks, fifos, etc.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: create a minimal skill directory with skill.toml.
    fn create_skill_dir(dir: &Path, name: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("skill.toml"),
            format!(
                "[skill]\nname = \"{name}\"\ndescription = \"Test skill\"\n\n[triggers]\nkeywords = [\"{name}\"]\n"
            ),
        )
        .unwrap();
        fs::write(dir.join("system_prompt.md"), "Use this skill.").unwrap();
    }

    #[test]
    fn test_copy_skill_dir_basic() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src-skill");
        create_skill_dir(&src, "test");
        fs::create_dir_all(src.join("handlers")).unwrap();
        fs::write(src.join("handlers/run.sh"), "#!/bin/sh\necho ok").unwrap();

        let dst = tmp.path().join("dst-skill");
        copy_skill_dir(&src, &dst).unwrap();

        assert!(dst.join("skill.toml").exists());
        assert!(dst.join("system_prompt.md").exists());
        assert!(dst.join("handlers/run.sh").exists());
    }

    #[test]
    fn test_copy_skill_dir_excludes_git() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src-skill");
        create_skill_dir(&src, "test");
        fs::create_dir_all(src.join(".git/objects")).unwrap();
        fs::write(src.join(".git/HEAD"), "ref: refs/heads/main").unwrap();

        let dst = tmp.path().join("dst-skill");
        copy_skill_dir(&src, &dst).unwrap();

        assert!(dst.join("skill.toml").exists());
        assert!(!dst.join(".git").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_copy_skill_dir_symlink_escape() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src-skill");
        create_skill_dir(&src, "test");

        // Create a symlink pointing outside the skill directory
        std::os::unix::fs::symlink("/etc/passwd", src.join("escape")).unwrap();

        let dst = tmp.path().join("dst-skill");
        let result = copy_skill_dir(&src, &dst);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("escape") || err_msg.contains("outside"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn test_install_rejects_bundled_name() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let skill_src = tmp.path().join("repo-skill");
        create_skill_dir(&skill_src, "tmux");

        let candidate = SkillCandidate {
            name: "tmux".to_string(),
            description: "Test".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src,
            has_exec_handlers: false,
            dependencies: vec![],
        };

        let result = install_skill(
            tmp.path(),
            &skills_dir,
            &candidate,
            None,
            "https://example.com/repo.git",
            "abc123",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("built-in"));
    }

    #[test]
    fn test_install_rejects_existing_name() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        create_skill_dir(&skills_dir.join("my-skill"), "my-skill");

        let skill_src = tmp.path().join("repo-skill");
        create_skill_dir(&skill_src, "my-skill");

        let candidate = SkillCandidate {
            name: "my-skill".to_string(),
            description: "Test".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src,
            has_exec_handlers: false,
            dependencies: vec![],
        };

        let result = install_skill(
            tmp.path(),
            &skills_dir,
            &candidate,
            None,
            "https://example.com/repo.git",
            "abc123",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_install_and_uninstall() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let skill_src = tmp.path().join("repo-skill");
        create_skill_dir(&skill_src, "my-cool-skill");

        let candidate = SkillCandidate {
            name: "my-cool-skill".to_string(),
            description: "Cool skill".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src,
            has_exec_handlers: false,
            dependencies: vec![],
        };

        // Install
        let result = install_skill(
            tmp.path(),
            &skills_dir,
            &candidate,
            None,
            "https://github.com/user/repo.git",
            "abc123def456789012345678901234567890abcd",
        )
        .unwrap();

        assert_eq!(result.name, "my-cool-skill");
        assert!(skills_dir.join("my-cool-skill/skill.toml").exists());

        // Verify lock
        let lock = read_lock(tmp.path());
        assert!(lock.skills.contains_key("my-cool-skill"));
        assert_eq!(
            lock.skills["my-cool-skill"].commit,
            "abc123def456789012345678901234567890abcd"
        );

        // Uninstall
        uninstall_skill(tmp.path(), &skills_dir, "my-cool-skill").unwrap();
        assert!(!skills_dir.join("my-cool-skill").exists());
        let lock = read_lock(tmp.path());
        assert!(!lock.skills.contains_key("my-cool-skill"));
    }

    #[test]
    fn test_install_with_alias() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let skill_src = tmp.path().join("repo-skill");
        create_skill_dir(&skill_src, "original-name");

        let candidate = SkillCandidate {
            name: "original-name".to_string(),
            description: "Test".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src,
            has_exec_handlers: false,
            dependencies: vec![],
        };

        let result = install_skill(
            tmp.path(),
            &skills_dir,
            &candidate,
            Some("my-alias"),
            "https://example.com/repo.git",
            "abc123",
        )
        .unwrap();

        assert_eq!(result.name, "my-alias");
        assert!(skills_dir.join("my-alias/skill.toml").exists());
        assert!(!skills_dir.join("original-name").exists());
    }

    #[test]
    fn test_uninstall_bundled_rejected() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let result = uninstall_skill(tmp.path(), &skills_dir, "tmux");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("built-in"));
    }

    #[test]
    fn test_uninstall_non_marketplace() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        create_skill_dir(&skills_dir.join("custom-skill"), "custom-skill");

        let result = uninstall_skill(tmp.path(), &skills_dir, "custom-skill");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not a marketplace")
        );
    }

    #[test]
    fn test_uninstall_not_found() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let result = uninstall_skill(tmp.path(), &skills_dir, "nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[cfg(unix)]
    #[test]
    fn test_install_linked() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let skill_src = tmp.path().join("repo-skill");
        create_skill_dir(&skill_src, "linked-skill");

        let candidate = SkillCandidate {
            name: "linked-skill".to_string(),
            description: "Test linked".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src.clone(),
            has_exec_handlers: false,
            dependencies: vec![],
        };

        let result = install_skill_linked(
            tmp.path(),
            &skills_dir,
            &candidate,
            None,
            &format!("file://{}", skill_src.display()),
        )
        .unwrap();

        assert_eq!(result.name, "linked-skill");
        assert!(result.linked);

        // Verify it's a symlink
        let target = skills_dir.join("linked-skill");
        let meta = target.symlink_metadata().unwrap();
        assert!(meta.file_type().is_symlink());

        // Verify lock file
        let lock = read_lock(tmp.path());
        assert!(lock.skills["linked-skill"].linked);
        assert_eq!(lock.skills["linked-skill"].commit, "");
    }

    #[cfg(unix)]
    #[test]
    fn test_uninstall_linked_removes_symlink_only() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let skill_src = tmp.path().join("repo-skill");
        create_skill_dir(&skill_src, "linked-skill");

        let candidate = SkillCandidate {
            name: "linked-skill".to_string(),
            description: "Test linked".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src.clone(),
            has_exec_handlers: false,
            dependencies: vec![],
        };

        install_skill_linked(
            tmp.path(),
            &skills_dir,
            &candidate,
            None,
            &format!("file://{}", skill_src.display()),
        )
        .unwrap();

        // Uninstall
        uninstall_skill(tmp.path(), &skills_dir, "linked-skill").unwrap();

        // Symlink should be gone
        assert!(!skills_dir.join("linked-skill").exists());
        // Source directory should still exist
        assert!(skill_src.join("skill.toml").exists());
        // Lock entry should be gone
        let lock = read_lock(tmp.path());
        assert!(!lock.skills.contains_key("linked-skill"));
    }

    #[test]
    fn test_install_local_snapshot() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let skill_src = tmp.path().join("local-skill");
        create_skill_dir(&skill_src, "local-skill");

        let candidate = SkillCandidate {
            name: "local-skill".to_string(),
            description: "Test local".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src.clone(),
            has_exec_handlers: false,
            dependencies: vec![],
        };

        let result = install_skill(
            tmp.path(),
            &skills_dir,
            &candidate,
            None,
            &format!("file://{}", skill_src.display()),
            "",
        )
        .unwrap();

        assert_eq!(result.name, "local-skill");
        assert!(!result.linked);

        // Verify it's a real directory (not symlink)
        let target = skills_dir.join("local-skill");
        assert!(target.is_dir());
        let meta = target.symlink_metadata().unwrap();
        assert!(!meta.file_type().is_symlink());

        // Verify lock file
        let lock = read_lock(tmp.path());
        assert!(!lock.skills["local-skill"].linked);
        assert!(lock.skills["local-skill"].url.starts_with("file://"));
    }

    #[test]
    fn test_install_rejects_self_referential() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create a skill inside the skills directory itself
        let skill_src = skills_dir.join("self-ref");
        create_skill_dir(&skill_src, "self-ref");

        let candidate = SkillCandidate {
            name: "self-ref".to_string(),
            description: "Self-referential".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src,
            has_exec_handlers: false,
            dependencies: vec![],
        };

        let result = install_skill(
            tmp.path(),
            &skills_dir,
            &candidate,
            Some("self-ref-copy"),
            "file:///tmp/fake",
            "",
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("inside the skills directory")
        );
    }

    #[test]
    fn test_update_local_snapshot() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let skill_src = tmp.path().join("local-skill");
        create_skill_dir(&skill_src, "local-skill");

        let candidate = SkillCandidate {
            name: "local-skill".to_string(),
            description: "Test local".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src.clone(),
            has_exec_handlers: false,
            dependencies: vec![],
        };

        install_skill(
            tmp.path(),
            &skills_dir,
            &candidate,
            None,
            &format!("file://{}", skill_src.display()),
            "",
        )
        .unwrap();

        // Modify source
        fs::write(skill_src.join("new_file.txt"), "new content").unwrap();

        // Update should re-copy
        let result = update_skill(tmp.path(), &skills_dir, "local-skill").unwrap();
        assert!(matches!(result, UpdateResult::Updated { .. }));

        // New file should be in the installed copy
        assert!(skills_dir.join("local-skill/new_file.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_update_linked_is_noop() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let skill_src = tmp.path().join("linked-skill");
        create_skill_dir(&skill_src, "linked-skill");

        let candidate = SkillCandidate {
            name: "linked-skill".to_string(),
            description: "Test linked".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src.clone(),
            has_exec_handlers: false,
            dependencies: vec![],
        };

        install_skill_linked(
            tmp.path(),
            &skills_dir,
            &candidate,
            None,
            &format!("file://{}", skill_src.display()),
        )
        .unwrap();

        let result = update_skill(tmp.path(), &skills_dir, "linked-skill").unwrap();
        assert!(matches!(result, UpdateResult::LinkedNoOp { .. }));
    }

    #[test]
    fn test_update_local_missing_source() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let skill_src = tmp.path().join("disappearing-skill");
        create_skill_dir(&skill_src, "disappearing-skill");

        let candidate = SkillCandidate {
            name: "disappearing-skill".to_string(),
            description: "Test".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src.clone(),
            has_exec_handlers: false,
            dependencies: vec![],
        };

        install_skill(
            tmp.path(),
            &skills_dir,
            &candidate,
            None,
            &format!("file://{}", skill_src.display()),
            "",
        )
        .unwrap();

        // Remove source
        fs::remove_dir_all(&skill_src).unwrap();

        // Update should fail with clear message
        let result = update_skill(tmp.path(), &skills_dir, "disappearing-skill");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no longer exists"));
    }

    // --- Dependency resolution tests ---

    fn make_candidate(name: &str, deps: &[&str]) -> SkillCandidate {
        SkillCandidate {
            name: name.to_string(),
            description: format!("{name} skill"),
            relative_path: ".".to_string(),
            absolute_path: std::path::PathBuf::from(format!("/tmp/test/{name}")),
            has_exec_handlers: false,
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn test_resolve_no_dependencies() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let root = make_candidate("skill-a", &[]);
        let roots = vec![&root];
        let available = vec![root.clone()];

        let result = resolve_dependencies(&roots, &available, &skills_dir).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_resolve_simple_dependency() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let root = make_candidate("skill-a", &["skill-b"]);
        let dep = make_candidate("skill-b", &[]);
        let roots = vec![&root];
        let available = vec![root.clone(), dep];

        let result = resolve_dependencies(&roots, &available, &skills_dir).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].candidate.name, "skill-b");
        assert_eq!(result[0].required_by, "skill-a");
        assert_eq!(result[0].depth, 1);
    }

    #[test]
    fn test_resolve_transitive_dependencies() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let root = make_candidate("skill-a", &["skill-b"]);
        let mid = make_candidate("skill-b", &["skill-c"]);
        let leaf = make_candidate("skill-c", &[]);
        let roots = vec![&root];
        let available = vec![root.clone(), mid, leaf];

        let result = resolve_dependencies(&roots, &available, &skills_dir).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].candidate.name, "skill-b");
        assert_eq!(result[0].depth, 1);
        assert_eq!(result[1].candidate.name, "skill-c");
        assert_eq!(result[1].depth, 2);
    }

    #[test]
    fn test_resolve_circular_dependency() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // A depends on B, B depends on C, C depends on B (cycle in deps, not involving root)
        let a = make_candidate("skill-a", &["skill-b"]);
        let b = make_candidate("skill-b", &["skill-c"]);
        let c = make_candidate("skill-c", &["skill-b"]);
        let roots = vec![&a];
        let available = vec![a.clone(), b, c];

        let result = resolve_dependencies(&roots, &available, &skills_dir);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Circular dependency"), "got: {err}");
    }

    #[test]
    fn test_resolve_root_as_dep_of_dep_is_not_circular() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // A depends on B, B depends on A → B is resolved, A is skipped (root)
        let a = make_candidate("skill-a", &["skill-b"]);
        let b = make_candidate("skill-b", &["skill-a"]);
        let roots = vec![&a];
        let available = vec![a.clone(), b];

        let result = resolve_dependencies(&roots, &available, &skills_dir).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].candidate.name, "skill-b");
    }

    #[test]
    fn test_resolve_diamond_dependency() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // A and B both depend on C
        let a = make_candidate("skill-a", &["shared"]);
        let b = make_candidate("skill-b", &["shared"]);
        let shared = make_candidate("shared", &[]);
        let roots = vec![&a, &b];
        let available = vec![a.clone(), b.clone(), shared];

        let result = resolve_dependencies(&roots, &available, &skills_dir).unwrap();
        // shared should appear only once
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].candidate.name, "shared");
    }

    #[test]
    fn test_resolve_self_dependency() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let root = make_candidate("skill-a", &["skill-a"]);
        let roots = vec![&root];
        let available = vec![root.clone()];

        let result = resolve_dependencies(&roots, &available, &skills_dir);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("lists itself as a dependency"), "got: {err}");
    }

    #[test]
    fn test_resolve_already_installed_skipped() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Pre-install skill-b
        create_skill_dir(&skills_dir.join("skill-b"), "skill-b");

        let root = make_candidate("skill-a", &["skill-b"]);
        let dep = make_candidate("skill-b", &[]);
        let roots = vec![&root];
        let available = vec![root.clone(), dep];

        let result = resolve_dependencies(&roots, &available, &skills_dir).unwrap();
        assert!(result.is_empty(), "already-installed dep should be skipped");
    }

    #[test]
    fn test_resolve_bundled_skipped() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // "tmux" is a bundled skill
        let root = make_candidate("skill-a", &["tmux"]);
        let roots = vec![&root];
        let available = vec![root.clone()];

        let result = resolve_dependencies(&roots, &available, &skills_dir).unwrap();
        assert!(result.is_empty(), "bundled dep should be skipped");
    }

    #[test]
    fn test_resolve_missing_dependency() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let root = make_candidate("skill-a", &["nonexistent"]);
        let roots = vec![&root];
        let available = vec![root.clone()];

        let result = resolve_dependencies(&roots, &available, &skills_dir);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent"), "got: {err}");
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn test_resolve_max_depth_exceeded() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create a chain of 12 deep: a -> b -> c -> ... -> l
        let names: Vec<String> = (0..12)
            .map(|i| format!("skill-{}", (b'a' + i) as char))
            .collect();
        let mut candidates: Vec<SkillCandidate> = Vec::new();
        for i in 0..12 {
            let deps = if i < 11 {
                vec![names[i + 1].as_str()]
            } else {
                vec![]
            };
            candidates.push(make_candidate(&names[i], &deps));
        }

        let roots = vec![&candidates[0]];
        let result = resolve_dependencies(&roots, &candidates, &skills_dir);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("maximum depth"), "got: {err}");
    }

    // --- Lock file dependency tracking tests ---

    #[test]
    fn test_lock_backward_compat_no_installed_as_dependency_of() {
        // Existing lock files without installed_as_dependency_of should deserialize fine
        let toml_str = r#"
[skills.web-scraper]
url = "https://github.com/user/repo.git"
path = "."
commit = "abc123"
installed_at = "2026-03-02T10:30:00Z"
updated_at = "2026-03-02T10:30:00Z"
"#;
        let lock: super::super::marketplace::MarketplaceLock = toml::from_str(toml_str).unwrap();
        let entry = lock.skills.get("web-scraper").unwrap();
        assert!(entry.installed_as_dependency_of.is_empty());
        assert!(!entry.linked);
    }

    #[test]
    fn test_mark_as_dependency() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let skill_src = tmp.path().join("repo-skill");
        create_skill_dir(&skill_src, "dep-skill");

        let candidate = SkillCandidate {
            name: "dep-skill".to_string(),
            description: "Dep".to_string(),
            relative_path: ".".to_string(),
            absolute_path: skill_src,
            has_exec_handlers: false,
            dependencies: vec![],
        };

        install_skill(
            tmp.path(),
            &skills_dir,
            &candidate,
            None,
            "file:///tmp/test",
            "",
        )
        .unwrap();

        mark_as_dependency(tmp.path(), "dep-skill", "parent-a").unwrap();

        let lock = read_lock(tmp.path());
        let entry = &lock.skills["dep-skill"];
        assert_eq!(entry.installed_as_dependency_of, vec!["parent-a"]);

        // Diamond: add second parent
        mark_as_dependency(tmp.path(), "dep-skill", "parent-b").unwrap();
        let lock = read_lock(tmp.path());
        let entry = &lock.skills["dep-skill"];
        assert_eq!(
            entry.installed_as_dependency_of,
            vec!["parent-a", "parent-b"]
        );

        // Duplicate parent is not added twice
        mark_as_dependency(tmp.path(), "dep-skill", "parent-a").unwrap();
        let lock = read_lock(tmp.path());
        let entry = &lock.skills["dep-skill"];
        assert_eq!(
            entry.installed_as_dependency_of,
            vec!["parent-a", "parent-b"]
        );
    }

    #[test]
    fn test_find_orphaned_deps() {
        let tmp = TempDir::new().unwrap();

        // Simulate lock file with dependency tracking
        let mut lock = super::super::marketplace::MarketplaceLock::default();
        lock.skills.insert(
            "parent-a".to_string(),
            MarketplaceEntry {
                url: "file:///test".to_string(),
                path: ".".to_string(),
                commit: String::new(),
                linked: false,
                installed_as_dependency_of: vec![],
                installed_at: "2026-03-02T10:30:00Z".to_string(),
                updated_at: "2026-03-02T10:30:00Z".to_string(),
            },
        );
        lock.skills.insert(
            "dep-only-of-a".to_string(),
            MarketplaceEntry {
                url: "file:///test".to_string(),
                path: ".".to_string(),
                commit: String::new(),
                linked: false,
                installed_as_dependency_of: vec!["parent-a".to_string()],
                installed_at: "2026-03-02T10:30:00Z".to_string(),
                updated_at: "2026-03-02T10:30:00Z".to_string(),
            },
        );
        lock.skills.insert(
            "dep-of-a-and-b".to_string(),
            MarketplaceEntry {
                url: "file:///test".to_string(),
                path: ".".to_string(),
                commit: String::new(),
                linked: false,
                installed_as_dependency_of: vec!["parent-a".to_string(), "parent-b".to_string()],
                installed_at: "2026-03-02T10:30:00Z".to_string(),
                updated_at: "2026-03-02T10:30:00Z".to_string(),
            },
        );
        write_lock(tmp.path(), &lock).unwrap();

        let orphans = find_orphaned_deps(tmp.path(), "parent-a");
        assert_eq!(orphans, vec!["dep-only-of-a"]);
        // dep-of-a-and-b is not orphaned because parent-b still depends on it
    }

    #[test]
    fn test_remove_from_dependency_lists() {
        let tmp = TempDir::new().unwrap();

        let mut lock = super::super::marketplace::MarketplaceLock::default();
        lock.skills.insert(
            "dep-skill".to_string(),
            MarketplaceEntry {
                url: "file:///test".to_string(),
                path: ".".to_string(),
                commit: String::new(),
                linked: false,
                installed_as_dependency_of: vec!["parent-a".to_string(), "parent-b".to_string()],
                installed_at: "2026-03-02T10:30:00Z".to_string(),
                updated_at: "2026-03-02T10:30:00Z".to_string(),
            },
        );
        write_lock(tmp.path(), &lock).unwrap();

        remove_from_dependency_lists(tmp.path(), "parent-a").unwrap();

        let lock = read_lock(tmp.path());
        assert_eq!(
            lock.skills["dep-skill"].installed_as_dependency_of,
            vec!["parent-b"]
        );
    }
}
