//! Marketplace lock file for tracking skills installed from Git repositories.
//!
//! The lock file (`marketplace.lock`) lives at `{agent_home}/marketplace.lock`
//! and tracks installed marketplace skills with their source URL, path within
//! the repo, pinned commit hash, and timestamps.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::warn;

/// Lock file tracking marketplace-installed skills.
#[derive(Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct MarketplaceLock {
    #[serde(default)]
    pub skills: BTreeMap<String, MarketplaceEntry>,
}

/// A single marketplace skill entry in the lock file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketplaceEntry {
    /// Git clone URL (e.g. "https://github.com/user/repo.git").
    pub url: String,
    /// Path within the repo to the skill directory ("." for root-level skills).
    pub path: String,
    /// Pinned commit hash at install/update time.
    pub commit: String,
    /// ISO 8601 timestamp of first install.
    pub installed_at: String,
    /// ISO 8601 timestamp of last update.
    pub updated_at: String,
}

const LOCK_FILE_NAME: &str = "marketplace.lock";

/// Read the marketplace lock file from an agent's home directory.
///
/// Returns an empty `MarketplaceLock` if the file doesn't exist.
/// Logs a warning and returns empty if the file is corrupt.
pub fn read_lock(agent_home: &Path) -> MarketplaceLock {
    let path = agent_home.join(LOCK_FILE_NAME);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return MarketplaceLock::default(),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to read marketplace lock file");
            return MarketplaceLock::default();
        }
    };

    match toml::from_str(&content) {
        Ok(lock) => lock,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "corrupt marketplace lock file, treating as empty");
            MarketplaceLock::default()
        }
    }
}

/// Write the marketplace lock file atomically.
///
/// Writes to a temporary file first, then renames. Sets 0o600 permissions on Unix.
pub fn write_lock(agent_home: &Path, lock: &MarketplaceLock) -> anyhow::Result<()> {
    let path = agent_home.join(LOCK_FILE_NAME);
    let tmp_path = agent_home.join(format!("{LOCK_FILE_NAME}.tmp"));

    let content = toml::to_string_pretty(lock)?;
    std::fs::write(&tmp_path, &content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
    }

    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

/// Check if a skill name is marketplace-installed.
pub fn is_marketplace_skill(agent_home: &Path, name: &str) -> bool {
    let lock = read_lock(agent_home);
    lock.skills.contains_key(name)
}

/// A skill candidate discovered by scanning a cloned repository.
#[derive(Debug, Clone)]
pub struct SkillCandidate {
    /// Manifest name from skill.toml.
    pub name: String,
    /// Description from skill.toml.
    pub description: String,
    /// Path relative to repo root (e.g. ".", "weather", "skills/weather").
    pub relative_path: String,
    /// Absolute path to the skill directory in the temp clone.
    pub absolute_path: std::path::PathBuf,
    /// Whether the skill has exec handler tools.
    pub has_exec_handlers: bool,
}

/// Scan a cloned repo for skill.toml files up to depth 2.
///
/// Depth 0: `./skill.toml` (single-skill repo at root)
/// Depth 1: `./foo/skill.toml` (multi-skill repo, one level)
/// Depth 2: `./foo/bar/skill.toml` (nested, two levels)
///
/// The parent directory of each `skill.toml` is treated as the skill directory.
/// Skills with invalid manifests are skipped with a warning.
pub fn scan_repo_for_skills(repo_dir: &Path) -> Vec<SkillCandidate> {
    let mut candidates = Vec::new();

    // Check depth 0: root-level skill.toml
    if let Some(candidate) = try_load_candidate(repo_dir, repo_dir) {
        candidates.push(candidate);
    }

    // Check depth 1 and 2
    if let Ok(entries) = std::fs::read_dir(repo_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || is_hidden_or_git(&path) {
                continue;
            }

            // Depth 1
            if let Some(candidate) = try_load_candidate(&path, repo_dir) {
                candidates.push(candidate);
            }

            // Depth 2
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                for sub_entry in sub_entries.flatten() {
                    let sub_path = sub_entry.path();
                    if !sub_path.is_dir() || is_hidden_or_git(&sub_path) {
                        continue;
                    }
                    if let Some(candidate) = try_load_candidate(&sub_path, repo_dir) {
                        candidates.push(candidate);
                    }
                }
            }
        }
    }

    candidates
}

/// Try to load a skill candidate from a directory that may contain skill.toml.
fn try_load_candidate(skill_dir: &Path, repo_dir: &Path) -> Option<SkillCandidate> {
    use super::manifest::{SkillManifest, SkillToolDef, ToolHandler};

    let manifest_path = skill_dir.join("skill.toml");
    let content = std::fs::read_to_string(&manifest_path).ok()?;

    let manifest: SkillManifest = match toml::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            warn!(path = %manifest_path.display(), error = %e, "invalid skill.toml in repo, skipping");
            return None;
        }
    };

    // Validate skill name
    if let Err(e) = super::super::tools::create_skill::validate_skill_name(&manifest.skill.name) {
        warn!(name = %manifest.skill.name, error = %e, "invalid skill name in repo, skipping");
        return None;
    }

    let relative_path = skill_dir
        .strip_prefix(repo_dir)
        .unwrap_or(Path::new("."))
        .to_string_lossy()
        .to_string();
    let relative_path = if relative_path.is_empty() {
        ".".to_string()
    } else {
        relative_path
    };

    // Check for exec handlers in tools.json
    let has_exec_handlers = skill_dir.join("tools.json").exists()
        && std::fs::read_to_string(skill_dir.join("tools.json"))
            .ok()
            .and_then(|c| serde_json::from_str::<Vec<SkillToolDef>>(&c).ok())
            .map(|tools| {
                tools
                    .iter()
                    .any(|t| matches!(t.handler, ToolHandler::Exec { .. }))
            })
            .unwrap_or(false);

    Some(SkillCandidate {
        name: manifest.skill.name,
        description: manifest.skill.description,
        relative_path,
        absolute_path: skill_dir.to_path_buf(),
        has_exec_handlers,
    })
}

/// Check if a path is a hidden directory or .git directory.
fn is_hidden_or_git(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_lock() -> MarketplaceLock {
        let mut skills = BTreeMap::new();
        skills.insert(
            "web-scraper".to_string(),
            MarketplaceEntry {
                url: "https://github.com/user/mika-skill-web-scraper.git".to_string(),
                path: ".".to_string(),
                commit: "abc123def456".to_string(),
                installed_at: "2026-03-02T10:30:00Z".to_string(),
                updated_at: "2026-03-02T10:30:00Z".to_string(),
            },
        );
        MarketplaceLock { skills }
    }

    #[test]
    fn test_serde_roundtrip() {
        let lock = make_lock();
        let serialized = toml::to_string_pretty(&lock).unwrap();
        let deserialized: MarketplaceLock = toml::from_str(&serialized).unwrap();
        assert_eq!(lock, deserialized);
    }

    #[test]
    fn test_read_lock_missing_file() {
        let tmp = TempDir::new().unwrap();
        let lock = read_lock(tmp.path());
        assert!(lock.skills.is_empty());
    }

    #[test]
    fn test_read_lock_corrupt_file() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(LOCK_FILE_NAME), "{{not valid toml").unwrap();
        let lock = read_lock(tmp.path());
        assert!(lock.skills.is_empty());
    }

    #[test]
    fn test_write_and_read_lock() {
        let tmp = TempDir::new().unwrap();
        let lock = make_lock();
        write_lock(tmp.path(), &lock).unwrap();
        let read_back = read_lock(tmp.path());
        assert_eq!(lock, read_back);
    }

    #[test]
    fn test_write_lock_atomic() {
        let tmp = TempDir::new().unwrap();
        let lock = make_lock();
        write_lock(tmp.path(), &lock).unwrap();

        // Tmp file should not exist after successful write
        assert!(!tmp.path().join(format!("{LOCK_FILE_NAME}.tmp")).exists());
        // Lock file should exist
        assert!(tmp.path().join(LOCK_FILE_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_write_lock_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let lock = make_lock();
        write_lock(tmp.path(), &lock).unwrap();

        let mode = fs::metadata(tmp.path().join(LOCK_FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "lock file should be 0600");
    }

    #[test]
    fn test_is_marketplace_skill() {
        let tmp = TempDir::new().unwrap();
        let lock = make_lock();
        write_lock(tmp.path(), &lock).unwrap();

        assert!(is_marketplace_skill(tmp.path(), "web-scraper"));
        assert!(!is_marketplace_skill(tmp.path(), "nonexistent"));
    }

    #[test]
    fn test_is_marketplace_skill_no_lock_file() {
        let tmp = TempDir::new().unwrap();
        assert!(!is_marketplace_skill(tmp.path(), "anything"));
    }

    // --- Scanner tests ---

    fn write_skill_toml(dir: &Path, name: &str, description: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("skill.toml"),
            format!(
                "[skill]\nname = \"{name}\"\ndescription = \"{description}\"\n\n[triggers]\nkeywords = [\"{name}\"]\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn test_scan_single_skill_at_root() {
        let tmp = TempDir::new().unwrap();
        write_skill_toml(tmp.path(), "my-skill", "A cool skill");

        let candidates = scan_repo_for_skills(tmp.path());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "my-skill");
        assert_eq!(candidates[0].relative_path, ".");
    }

    #[test]
    fn test_scan_multi_skill_repo() {
        let tmp = TempDir::new().unwrap();
        write_skill_toml(&tmp.path().join("weather"), "weather", "Weather skill");
        write_skill_toml(&tmp.path().join("news"), "news", "News skill");

        let candidates = scan_repo_for_skills(tmp.path());
        assert_eq!(candidates.len(), 2);
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"weather"));
        assert!(names.contains(&"news"));
    }

    #[test]
    fn test_scan_depth_2() {
        let tmp = TempDir::new().unwrap();
        write_skill_toml(
            &tmp.path().join("skills").join("weather"),
            "weather",
            "Nested weather",
        );

        let candidates = scan_repo_for_skills(tmp.path());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "weather");
        assert!(candidates[0].relative_path.contains("weather"));
    }

    #[test]
    fn test_scan_does_not_recurse_beyond_depth_2() {
        let tmp = TempDir::new().unwrap();
        write_skill_toml(
            &tmp.path().join("a").join("b").join("c"),
            "deep",
            "Too deep",
        );

        let candidates = scan_repo_for_skills(tmp.path());
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_scan_skips_invalid_manifests() {
        let tmp = TempDir::new().unwrap();
        write_skill_toml(&tmp.path().join("good"), "good", "Valid");

        let bad_dir = tmp.path().join("bad");
        fs::create_dir_all(&bad_dir).unwrap();
        fs::write(bad_dir.join("skill.toml"), "not valid toml {{").unwrap();

        let candidates = scan_repo_for_skills(tmp.path());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "good");
    }

    #[test]
    fn test_scan_skips_hidden_and_git_dirs() {
        let tmp = TempDir::new().unwrap();
        write_skill_toml(&tmp.path().join(".git"), "hidden", "Should be skipped");
        write_skill_toml(&tmp.path().join(".hidden"), "hidden2", "Should be skipped");
        write_skill_toml(&tmp.path().join("visible"), "visible", "Should be found");

        let candidates = scan_repo_for_skills(tmp.path());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "visible");
    }

    #[test]
    fn test_scan_empty_repo() {
        let tmp = TempDir::new().unwrap();
        let candidates = scan_repo_for_skills(tmp.path());
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_scan_detects_exec_handlers() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("my-skill");
        write_skill_toml(&skill_dir, "my-skill", "Has exec handlers");
        fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "run_action",
                "description": "Do something",
                "input_schema": {"type": "object", "properties": {}},
                "handler": {"type": "exec", "command": "handlers/run.sh"}
            }]"#,
        )
        .unwrap();

        let candidates = scan_repo_for_skills(tmp.path());
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].has_exec_handlers);
    }

    #[test]
    fn test_scan_no_exec_handlers() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("my-skill");
        write_skill_toml(&skill_dir, "my-skill", "No exec handlers");
        // No tools.json

        let candidates = scan_repo_for_skills(tmp.path());
        assert_eq!(candidates.len(), 1);
        assert!(!candidates[0].has_exec_handlers);
    }

    #[test]
    fn test_scan_skips_invalid_skill_name() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("bad-name");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            "[skill]\nname = \"has spaces\"\ndescription = \"Bad name\"\n",
        )
        .unwrap();

        let candidates = scan_repo_for_skills(tmp.path());
        assert!(candidates.is_empty());
    }
}
