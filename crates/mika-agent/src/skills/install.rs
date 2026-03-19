//! Core install/uninstall/update operations for marketplace skills.
//!
//! These functions handle filesystem operations and lock file updates.
//! CLI interaction (prompts, output formatting) belongs in `mika-cli`.

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
}
