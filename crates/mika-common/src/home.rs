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

/// Path to the single shared container database: `{home_dir}/data/mika.db`.
/// This is the unified schema v1 database that replaces per-agent databases.
pub fn container_db_path(home_dir: &Path) -> PathBuf {
    home_dir.join("data").join("mika.db")
}

/// Check if Mika has been initialized (database exists).
/// Supports both legacy layout (data/mika.db at root) and multi-agent layout.
pub fn is_initialized(home_dir: &Path) -> bool {
    // Legacy layout
    if home_dir.join("data").join("mika.db").exists() {
        return true;
    }
    // Multi-agent layout: at least one bootstrapped agent
    !crate::agent::list_agents(home_dir).is_empty()
}

/// Check if the home directory uses the multi-agent layout (has `agents/` dir).
pub fn is_multi_agent_layout(home_dir: &Path) -> bool {
    home_dir.join("agents").is_dir()
}

/// Check if the home directory uses the legacy layout (has `data/mika.db` at root, no `agents/` dir).
pub fn is_legacy_layout(home_dir: &Path) -> bool {
    home_dir.join("data").join("mika.db").exists() && !is_multi_agent_layout(home_dir)
}

/// Bootstrap a fresh installation with multi-agent layout.
///
/// Creates the `agents/` directory, initializes the default agent,
/// sets it as active, and writes the root-level global config.
pub fn bootstrap_fresh_install(home_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(home_dir.join("agents"))
        .with_context(|| format!("failed to create {}/agents/", home_dir.display()))?;
    bootstrap_agent(home_dir, crate::agent::DEFAULT_AGENT)
        .with_context(|| "failed to initialize default agent".to_string())?;
    write_active_agent(home_dir, crate::agent::DEFAULT_AGENT)?;
    write_default_if_missing(home_dir, "config.toml", DEFAULT_GLOBAL_CONFIG)?;
    Ok(())
}

/// Bootstrap a named agent under the multi-agent layout.
/// Validates the name, creates `{home_dir}/agents/{name}/`, and calls `bootstrap()`.
pub fn bootstrap_agent(home_dir: &Path, name: &str) -> Result<()> {
    crate::agent::validate_agent_name(name)?;
    let dir = crate::agent::agent_dir(home_dir, name);
    bootstrap(&dir)
}

/// Resolve the effective home directory for a named agent.
/// - Multi-agent layout: returns `{home_dir}/agents/{name}/`
/// - Legacy layout (no `agents/` dir): returns `home_dir` unchanged (backward compat)
pub fn resolve_agent_home(home_dir: &Path, agent_name: &str) -> PathBuf {
    if is_multi_agent_layout(home_dir) {
        crate::agent::agent_dir(home_dir, agent_name)
    } else {
        home_dir.to_path_buf()
    }
}

/// Migrate a legacy layout to multi-agent layout (idempotent).
///
/// If already multi-agent layout → no-op.
/// If legacy layout: creates `agents/main/`, moves data/, logs/, skills/, exports/,
/// config.toml, identity.toml, soul.md, heartbeat.md, user.md into it.
/// Writes `active_agent` file with "main".
/// Creates a root-level config.toml with shared settings.
pub fn migrate_to_multi_agent(home_dir: &Path) -> Result<()> {
    if is_multi_agent_layout(home_dir) {
        return Ok(()); // Already migrated
    }
    if !is_legacy_layout(home_dir) {
        return Ok(()); // Nothing to migrate (fresh install)
    }

    let agent = crate::agent::agent_dir(home_dir, crate::agent::DEFAULT_AGENT);
    std::fs::create_dir_all(&agent)
        .with_context(|| format!("failed to create {}", agent.display()))?;

    // Move directories (fault-tolerant: NotFound means another process already moved it)
    for dir_name in &["data", "logs", "skills", "exports"] {
        let src = home_dir.join(dir_name);
        if src.is_dir() {
            let dst = agent.join(dir_name);
            match std::fs::rename(&src, &dst) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // already moved
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("failed to move {} to {}", src.display(), dst.display())
                    });
                }
            }
        }
    }

    // Move files (fault-tolerant: NotFound means another process already moved it)
    for filename in &[
        "config.toml",
        "identity.toml",
        "soul.md",
        "heartbeat.md",
        "user.md",
    ] {
        let src = home_dir.join(filename);
        if src.is_file() {
            let dst = agent.join(filename);
            match std::fs::rename(&src, &dst) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // already moved
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("failed to move {} to {}", src.display(), dst.display())
                    });
                }
            }
        }
    }

    // Write active_agent file
    write_active_agent(home_dir, crate::agent::DEFAULT_AGENT)?;

    // Write root-level shared config
    write_default_if_missing(home_dir, "config.toml", DEFAULT_GLOBAL_CONFIG)?;

    Ok(())
}

/// Read the active agent name from `{home_dir}/active_agent`.
/// Returns DEFAULT_AGENT ("main") if file doesn't exist or is empty.
pub fn read_active_agent(home_dir: &Path) -> String {
    let path = home_dir.join("active_agent");
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::agent::DEFAULT_AGENT.to_string())
}

/// Write the active agent name to `{home_dir}/active_agent`.
pub fn write_active_agent(home_dir: &Path, name: &str) -> Result<()> {
    let path = home_dir.join("active_agent");
    std::fs::write(&path, name).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Default global config (shared settings at the root level).
pub const DEFAULT_GLOBAL_CONFIG: &str = r#"# Mika global configuration (shared across all agents).
# Override with MIKA_* environment variables (highest priority).
#
# Secrets MUST be set via environment variables, not in this file:
#   MIKA_ANTHROPIC_API_KEY — Anthropic API key
#   MIKA_OPENAI_API_KEY   — OpenAI API key (optional, for vector search)

log_level = "info"
"#;

/// Create the ~/.mika/ directory structure with default files.
/// Sets permissions to 0700 for directories, 0600 for files on Unix.
pub fn bootstrap(home_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(home_dir.join("data"))
        .with_context(|| format!("failed to create {}/data/", home_dir.display()))?;
    std::fs::create_dir_all(home_dir.join("logs"))
        .with_context(|| format!("failed to create {}/logs/", home_dir.display()))?;
    std::fs::create_dir_all(home_dir.join("skills"))
        .with_context(|| format!("failed to create {}/skills/", home_dir.display()))?;

    write_default_if_missing(home_dir, "config.toml", DEFAULT_CONFIG)?;
    write_default_if_missing(home_dir, "identity.toml", DEFAULT_IDENTITY)?;
    write_default_if_missing(home_dir, "soul.md", DEFAULT_SOUL)?;
    write_default_if_missing(home_dir, "heartbeat.md", DEFAULT_HEARTBEAT)?;
    write_default_if_missing(home_dir, "user.md", DEFAULT_USER)?;

    #[cfg(unix)]
    set_permissions(home_dir)?;

    Ok(())
}

pub fn write_default_if_missing(dir: &Path, filename: &str, content: &str) -> Result<()> {
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
    for dir_name in &["data", "logs", "skills"] {
        let dir_path = home_dir.join(dir_name);
        if dir_path.exists() {
            std::fs::set_permissions(&dir_path, std::fs::Permissions::from_mode(0o700))?;
        }
    }

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
#   MIKA_ANTHROPIC_API_KEY — Anthropic API key or OAuth token (sk-ant-oat01-...)

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
    use serial_test::serial;
    use std::fs;

    #[test]
    #[serial]
    fn test_resolve_home_dir_with_mika_home() {
        // Safety: test sets env var; no other thread reads this.
        unsafe { std::env::set_var("MIKA_HOME", "/tmp/test-mika-home") };
        let result = resolve_home_dir().unwrap();
        assert_eq!(result, PathBuf::from("/tmp/test-mika-home"));
        unsafe { std::env::remove_var("MIKA_HOME") };
    }

    #[test]
    #[serial]
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

    #[test]
    fn test_is_multi_agent_layout() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_multi_agent_layout(tmp.path()));

        fs::create_dir_all(tmp.path().join("agents")).unwrap();
        assert!(is_multi_agent_layout(tmp.path()));
    }

    #[test]
    fn test_is_legacy_layout() {
        let tmp = tempfile::tempdir().unwrap();
        // No DB at all
        assert!(!is_legacy_layout(tmp.path()));

        // Legacy: data/mika.db exists, no agents/ dir
        fs::create_dir_all(tmp.path().join("data")).unwrap();
        fs::write(tmp.path().join("data").join("mika.db"), "fake").unwrap();
        assert!(is_legacy_layout(tmp.path()));

        // Multi-agent: also has agents/ dir
        fs::create_dir_all(tmp.path().join("agents")).unwrap();
        assert!(!is_legacy_layout(tmp.path()));
    }

    #[test]
    fn test_bootstrap_agent() {
        let tmp = tempfile::tempdir().unwrap();
        bootstrap_agent(tmp.path(), "work").unwrap();

        let agent = tmp.path().join("agents").join("work");
        assert!(agent.join("data").is_dir());
        assert!(agent.join("logs").is_dir());
        assert!(agent.join("skills").is_dir());
        assert!(agent.join("config.toml").is_file());
        assert!(agent.join("soul.md").is_file());
        assert!(agent.join("identity.toml").is_file());
    }

    #[test]
    fn test_bootstrap_agent_rejects_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(bootstrap_agent(tmp.path(), "INVALID").is_err());
        assert!(bootstrap_agent(tmp.path(), "").is_err());
        assert!(bootstrap_agent(tmp.path(), "-bad").is_err());
    }

    #[test]
    fn test_resolve_agent_home_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        // No agents/ dir → legacy layout → returns home_dir
        let resolved = resolve_agent_home(tmp.path(), "main");
        assert_eq!(resolved, tmp.path());
    }

    #[test]
    fn test_resolve_agent_home_multi_agent() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("agents")).unwrap();
        let resolved = resolve_agent_home(tmp.path(), "work");
        assert_eq!(resolved, tmp.path().join("agents").join("work"));
    }

    #[test]
    fn test_is_initialized_multi_agent() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_initialized(tmp.path()));

        // Create a multi-agent layout with one bootstrapped agent
        let agent = tmp.path().join("agents").join("main");
        fs::create_dir_all(&agent).unwrap();
        fs::write(agent.join("config.toml"), "# config").unwrap();
        assert!(is_initialized(tmp.path()));
    }

    #[test]
    fn test_migrate_to_multi_agent_from_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        // Set up legacy layout
        bootstrap(home).unwrap();
        // Write a marker into the DB so we can verify it moved
        fs::write(home.join("data").join("mika.db"), "test-db-content").unwrap();
        fs::write(home.join("soul.md"), "custom soul").unwrap();

        assert!(is_legacy_layout(home));

        // Migrate
        migrate_to_multi_agent(home).unwrap();

        // Verify multi-agent layout
        assert!(is_multi_agent_layout(home));
        assert!(!is_legacy_layout(home));

        // Data moved to agents/main/
        let main_agent = home.join("agents").join("main");
        assert_eq!(
            fs::read_to_string(main_agent.join("data").join("mika.db")).unwrap(),
            "test-db-content"
        );
        assert_eq!(
            fs::read_to_string(main_agent.join("soul.md")).unwrap(),
            "custom soul"
        );
        assert!(main_agent.join("identity.toml").is_file());
        assert!(main_agent.join("config.toml").is_file());
        assert!(main_agent.join("skills").is_dir());

        // Root-level data/ should no longer exist
        assert!(!home.join("data").is_dir());

        // active_agent file should exist
        assert_eq!(read_active_agent(home), "main");

        // Root config.toml should be the global one
        let root_config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(root_config.contains("global"));
    }

    #[test]
    fn test_migrate_to_multi_agent_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        // Set up legacy layout and migrate
        bootstrap(home).unwrap();
        fs::write(home.join("data").join("mika.db"), "test-db").unwrap();
        migrate_to_multi_agent(home).unwrap();

        // Migrate again — should be no-op
        migrate_to_multi_agent(home).unwrap();

        // Still works
        assert!(is_multi_agent_layout(home));
        let main_agent = home.join("agents").join("main");
        assert_eq!(
            fs::read_to_string(main_agent.join("data").join("mika.db")).unwrap(),
            "test-db"
        );
    }

    #[test]
    fn test_migrate_to_multi_agent_noop_on_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        // No legacy layout, nothing to migrate
        migrate_to_multi_agent(tmp.path()).unwrap();
        // agents/ dir should not be created
        assert!(!is_multi_agent_layout(tmp.path()));
    }

    #[test]
    fn test_read_write_active_agent() {
        let tmp = tempfile::tempdir().unwrap();

        // Default when no file
        assert_eq!(read_active_agent(tmp.path()), "main");

        // Write and read back
        write_active_agent(tmp.path(), "work").unwrap();
        assert_eq!(read_active_agent(tmp.path()), "work");

        // Overwrite
        write_active_agent(tmp.path(), "code").unwrap();
        assert_eq!(read_active_agent(tmp.path()), "code");
    }

    #[test]
    fn test_read_active_agent_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("active_agent"), "  \n").unwrap();
        assert_eq!(read_active_agent(tmp.path()), "main");
    }

    #[test]
    fn test_bootstrap_fresh_install() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        bootstrap_fresh_install(home).unwrap();

        // Multi-agent layout created
        assert!(is_multi_agent_layout(home));

        // Default agent bootstrapped
        let main_agent = home.join("agents").join("main");
        assert!(main_agent.join("data").is_dir());
        assert!(main_agent.join("logs").is_dir());
        assert!(main_agent.join("skills").is_dir());
        assert!(main_agent.join("config.toml").is_file());
        assert!(main_agent.join("soul.md").is_file());

        // Active agent set to "main"
        assert_eq!(read_active_agent(home), "main");

        // Root-level global config written
        let root_config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(root_config.contains("global"));
    }
}
