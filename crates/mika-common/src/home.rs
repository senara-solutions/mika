use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Resolve the Mika home directory.
/// Priority: $MIKA_HOME > ~/.mika/
pub fn resolve_home_dir() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("MIKA_HOME") {
        return Ok(PathBuf::from(custom));
    }
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    Ok(home.join(".mika"))
}

/// Check if Mika has been initialized (database exists).
pub fn is_initialized(home_dir: &Path) -> bool {
    home_dir.join("data").join("mika.db").exists()
}

/// Create the ~/.mika/ directory structure with default files.
/// Sets permissions to 0700 for directories, 0600 for files on Unix.
pub fn bootstrap(home_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(home_dir.join("data"))
        .with_context(|| format!("failed to create {}/data/", home_dir.display()))?;
    std::fs::create_dir_all(home_dir.join("logs"))
        .with_context(|| format!("failed to create {}/logs/", home_dir.display()))?;

    write_default_if_missing(home_dir, "config.toml", DEFAULT_CONFIG)?;
    write_default_if_missing(home_dir, "identity.toml", DEFAULT_IDENTITY)?;
    write_default_if_missing(home_dir, "soul.md", DEFAULT_SOUL)?;
    write_default_if_missing(home_dir, "heartbeat.md", DEFAULT_HEARTBEAT)?;
    write_default_if_missing(home_dir, "user.md", DEFAULT_USER)?;

    #[cfg(unix)]
    set_permissions(home_dir)?;

    Ok(())
}

fn write_default_if_missing(dir: &Path, filename: &str, content: &str) -> Result<()> {
    let path = dir.join(filename);
    if !path.exists() {
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_permissions(home_dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // Directory: 0700
    std::fs::set_permissions(home_dir, std::fs::Permissions::from_mode(0o700))?;
    std::fs::set_permissions(
        home_dir.join("data"),
        std::fs::Permissions::from_mode(0o700),
    )?;
    std::fs::set_permissions(
        home_dir.join("logs"),
        std::fs::Permissions::from_mode(0o700),
    )?;

    // Files: 0600
    for filename in &[
        "config.toml",
        "identity.toml",
        "soul.md",
        "heartbeat.md",
        "user.md",
    ] {
        let path = home_dir.join(filename);
        if path.exists() {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

pub const DEFAULT_CONFIG: &str = r#"# Mika configuration
# Override with MIKA_* environment variables (highest priority).
#
# Secrets MUST be set via environment variables, not in this file:
#   MIKA_ANTHROPIC_API_KEY — Anthropic API key
#   MIKA_ENCRYPTION_KEY    — 64 hex chars (32 bytes) for AES-256-GCM

claude_model = "claude-sonnet-4-6"
claude_max_tokens = 4096
log_level = "info"
"#;

pub const DEFAULT_IDENTITY: &str = r#"name = "Mika"
emoji = "✦"
"#;

pub const DEFAULT_SOUL: &str = r#"# Mika — Executive Assistant

## Personality
You are Mika, a senior executive assistant. You are calm, confident,
and concise. You anticipate needs rather than wait for instructions.
You protect the user's time fiercely.

## Communication style
- Lead with the answer, then context if needed
- Never say "I hope this helps" or "Let me know if you need anything"
- Match the user's energy — brief if they're brief, detailed if they ask
- Use their first name naturally, not every message
- Push back respectfully when something doesn't make sense

## Proactive behaviors
- Flag scheduling conflicts before they happen
- Remind about commitments approaching their deadline
- Surface patterns ("You've rescheduled this meeting 3 times — want to cancel it?")

## Boundaries
- Never pretend to have done something you haven't
- Say "I don't know" when you don't know
- Ask for clarification rather than guess on high-stakes decisions
"#;

pub const DEFAULT_HEARTBEAT: &str = r#"# Heartbeat Checklist

- Review active commitments approaching deadline
- Check if any meetings are coming up in the next 2 hours
- Look for stale priorities (no updates in 3+ days)
- Surface patterns worth mentioning
"#;

pub const DEFAULT_USER: &str = r#"# Tell Mika about yourself

Edit this file with your name, role, preferences, and anything
you'd like Mika to know about you. This seeds Mika's initial
understanding when starting fresh.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_home_dir_with_mika_home() {
        // Safety: test sets env var; no other thread reads this.
        unsafe { std::env::set_var("MIKA_HOME", "/tmp/test-mika-home") };
        let result = resolve_home_dir().unwrap();
        assert_eq!(result, PathBuf::from("/tmp/test-mika-home"));
        unsafe { std::env::remove_var("MIKA_HOME") };
    }

    #[test]
    fn test_resolve_home_dir_default() {
        // Safety: test removes env var; no other thread reads this.
        unsafe { std::env::remove_var("MIKA_HOME") };
        let result = resolve_home_dir().unwrap();
        assert!(result.ends_with(".mika"));
    }

    #[test]
    fn test_is_initialized_false_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_initialized(tmp.path()));
    }

    #[test]
    fn test_is_initialized_true_when_db_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("mika.db"), "fake db").unwrap();
        assert!(is_initialized(tmp.path()));
    }

    #[test]
    fn test_bootstrap_creates_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("mika-test");

        bootstrap(&home).unwrap();

        assert!(home.join("data").is_dir());
        assert!(home.join("logs").is_dir());
        assert!(home.join("config.toml").is_file());
        assert!(home.join("identity.toml").is_file());
        assert!(home.join("soul.md").is_file());
        assert!(home.join("heartbeat.md").is_file());
        assert!(home.join("user.md").is_file());

        // Verify content
        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("claude_model"));
        assert!(config.contains("MIKA_ANTHROPIC_API_KEY"));

        let soul = fs::read_to_string(home.join("soul.md")).unwrap();
        assert!(soul.contains("executive assistant"));
    }

    #[test]
    fn test_bootstrap_does_not_overwrite_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("mika-test");

        bootstrap(&home).unwrap();

        // Modify a file
        fs::write(home.join("soul.md"), "custom soul").unwrap();

        // Bootstrap again — should NOT overwrite
        bootstrap(&home).unwrap();

        let soul = fs::read_to_string(home.join("soul.md")).unwrap();
        assert_eq!(soul, "custom soul");
    }

    #[cfg(unix)]
    #[test]
    fn test_bootstrap_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("mika-test");

        bootstrap(&home).unwrap();

        let dir_perms = fs::metadata(&home).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_perms, 0o700);

        let file_perms = fs::metadata(home.join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_perms, 0o600);
    }
}
