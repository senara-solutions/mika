use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// Default agent name used when no agent is specified.
pub const DEFAULT_AGENT: &str = "main";

/// Validate an agent name.
///
/// Rules: lowercase alphanumeric + hyphens, 1-32 chars,
/// no leading/trailing/consecutive hyphens.
pub fn validate_agent_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Agent name cannot be empty");
    }
    if name.len() > 32 {
        bail!("Agent name cannot exceed 32 characters");
    }
    if name.starts_with('-') || name.ends_with('-') {
        bail!("Agent name cannot start or end with a hyphen");
    }
    if name.contains("--") {
        bail!("Agent name cannot contain consecutive hyphens");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!("Agent name must contain only lowercase letters, digits, and hyphens");
    }
    Ok(())
}

/// Normalize an agent name: trim whitespace and lowercase.
pub fn normalize_agent_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Returns the directory path for a named agent: `{home_dir}/agents/{name}/`
pub fn agent_dir(home_dir: &Path, name: &str) -> PathBuf {
    home_dir.join("agents").join(name)
}

/// Check if a named agent exists (has a database file).
pub fn agent_exists(home_dir: &Path, name: &str) -> bool {
    agent_dir(home_dir, name)
        .join("data")
        .join("mika.db")
        .exists()
}

/// List all agents in `{home_dir}/agents/`, returning sorted names
/// of directories that contain a database file.
pub fn list_agents(home_dir: &Path) -> Vec<String> {
    let agents_dir = home_dir.join("agents");
    let Ok(entries) = std::fs::read_dir(&agents_dir) else {
        return Vec::new();
    };

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().join("data").join("mika.db").exists() {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_validate_valid_names() {
        assert!(validate_agent_name("main").is_ok());
        assert!(validate_agent_name("work").is_ok());
        assert!(validate_agent_name("my-agent").is_ok());
        assert!(validate_agent_name("agent1").is_ok());
        assert!(validate_agent_name("a").is_ok());
        assert!(validate_agent_name("a-b-c").is_ok());
        assert!(validate_agent_name("agent-123-test").is_ok());
    }

    #[test]
    fn test_validate_invalid_names() {
        // Empty
        assert!(validate_agent_name("").is_err());
        // Too long
        assert!(validate_agent_name(&"a".repeat(33)).is_err());
        // Leading hyphen
        assert!(validate_agent_name("-agent").is_err());
        // Trailing hyphen
        assert!(validate_agent_name("agent-").is_err());
        // Consecutive hyphens
        assert!(validate_agent_name("my--agent").is_err());
        // Uppercase
        assert!(validate_agent_name("Main").is_err());
        // Spaces
        assert!(validate_agent_name("my agent").is_err());
        // Special chars
        assert!(validate_agent_name("agent@work").is_err());
        assert!(validate_agent_name("agent.test").is_err());
        assert!(validate_agent_name("agent_test").is_err());
    }

    #[test]
    fn test_normalize_agent_name() {
        assert_eq!(normalize_agent_name("  Main  "), "main");
        assert_eq!(normalize_agent_name("WORK"), "work");
        assert_eq!(normalize_agent_name("my-agent"), "my-agent");
    }

    #[test]
    fn test_agent_dir() {
        let home = Path::new("/home/user/.mika");
        assert_eq!(
            agent_dir(home, "main"),
            PathBuf::from("/home/user/.mika/agents/main")
        );
        assert_eq!(
            agent_dir(home, "work"),
            PathBuf::from("/home/user/.mika/agents/work")
        );
    }

    #[test]
    fn test_agent_exists_false_when_no_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!agent_exists(tmp.path(), "main"));
    }

    #[test]
    fn test_agent_exists_true_when_db_present() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = agent_dir(tmp.path(), "main");
        fs::create_dir_all(agent.join("data")).unwrap();
        fs::write(agent.join("data").join("mika.db"), "fake").unwrap();
        assert!(agent_exists(tmp.path(), "main"));
    }

    #[test]
    fn test_agent_exists_false_when_dir_but_no_db() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = agent_dir(tmp.path(), "main");
        fs::create_dir_all(agent.join("data")).unwrap();
        assert!(!agent_exists(tmp.path(), "main"));
    }

    #[test]
    fn test_list_agents_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(list_agents(tmp.path()).is_empty());
    }

    #[test]
    fn test_list_agents_returns_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        // Create agents out of order
        for name in &["work", "main", "code"] {
            let agent = agent_dir(tmp.path(), name);
            fs::create_dir_all(agent.join("data")).unwrap();
            fs::write(agent.join("data").join("mika.db"), "fake").unwrap();
        }
        assert_eq!(list_agents(tmp.path()), vec!["code", "main", "work"]);
    }

    #[test]
    fn test_list_agents_skips_dirs_without_db() {
        let tmp = tempfile::tempdir().unwrap();
        // Agent with DB
        let main = agent_dir(tmp.path(), "main");
        fs::create_dir_all(main.join("data")).unwrap();
        fs::write(main.join("data").join("mika.db"), "fake").unwrap();
        // Agent without DB (incomplete)
        let incomplete = agent_dir(tmp.path(), "incomplete");
        fs::create_dir_all(incomplete.join("data")).unwrap();

        assert_eq!(list_agents(tmp.path()), vec!["main"]);
    }
}
