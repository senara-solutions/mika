use std::path::{Path, PathBuf};
use tracing::warn;

use super::manifest::SkillManifest;

/// A loaded skill entry with its manifest and pre-processed data.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub manifest: SkillManifest,
    pub dir: PathBuf,
    /// Pre-lowercased keywords for fast substring matching.
    pub keywords_lower: Vec<String>,
}

/// Scan a skills directory and load all valid skill manifests.
///
/// Each immediate subdirectory is expected to contain a `skill.toml`.
/// Invalid skills are logged at `warn` and skipped — never break startup.
pub fn scan_skills_dir(skills_dir: &Path) -> Vec<SkillEntry> {
    let read_dir = match std::fs::read_dir(skills_dir) {
        Ok(rd) => rd,
        Err(e) => {
            warn!(path = %skills_dir.display(), error = %e, "cannot read skills directory");
            return Vec::new();
        }
    };

    let mut entries = Vec::new();
    for dir_entry in read_dir {
        let dir_entry = match dir_entry {
            Ok(de) => de,
            Err(e) => {
                warn!(error = %e, "error reading skills directory entry");
                continue;
            }
        };

        let path = dir_entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("skill.toml");
        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "cannot read skill manifest");
                continue;
            }
        };

        let manifest: SkillManifest = match toml::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "invalid skill manifest");
                continue;
            }
        };

        let keywords_lower = manifest
            .triggers
            .keywords
            .iter()
            .map(|k| k.to_lowercase())
            .collect();

        entries.push(SkillEntry {
            manifest,
            dir: path,
            keywords_lower,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_valid_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("memory");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            name = "memory"
            description = "Memory tools"
            [triggers]
            keywords = ["Remember", "MEMORY"]
            [handler]
            type = "builtin"
            tools = ["store_fact"]
            "#,
        )
        .unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.name, "memory");
        assert_eq!(entries[0].keywords_lower, vec!["remember", "memory"]);
        assert_eq!(entries[0].dir, skill_dir);
    }

    #[test]
    fn test_scan_skips_invalid_manifests() {
        let tmp = tempfile::tempdir().unwrap();

        // Valid skill
        let valid = tmp.path().join("good");
        fs::create_dir_all(&valid).unwrap();
        fs::write(
            valid.join("skill.toml"),
            r#"
            name = "good"
            description = "Valid"
            [handler]
            type = "builtin"
            tools = []
            "#,
        )
        .unwrap();

        // Invalid skill (bad TOML)
        let bad = tmp.path().join("bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("skill.toml"), "this is not valid toml {{{}}}").unwrap();

        // Missing manifest
        let missing = tmp.path().join("missing");
        fs::create_dir_all(&missing).unwrap();

        let entries = scan_skills_dir(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.name, "good");
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = scan_skills_dir(tmp.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn test_scan_nonexistent_dir() {
        let entries = scan_skills_dir(Path::new("/nonexistent/skills"));
        assert!(entries.is_empty());
    }

    #[test]
    fn test_scan_ignores_files() {
        let tmp = tempfile::tempdir().unwrap();
        // A file (not a directory) in the skills dir should be skipped
        fs::write(tmp.path().join("readme.txt"), "not a skill").unwrap();
        let entries = scan_skills_dir(tmp.path());
        assert!(entries.is_empty());
    }
}
