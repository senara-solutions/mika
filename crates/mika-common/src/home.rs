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

/// Path to the canonical bundled-skill library: `{home_dir}/skills/`.
///
/// All bundled skills are extracted to this single shared location at agent
/// startup; per-agent `{home_dir}/agents/<name>/skills/<skill>` entries are
/// symlinks into this library. See `bundled_skills::seed_bundled_skill_library`.
pub fn library_skills_dir(home_dir: &Path) -> PathBuf {
    home_dir.join("skills")
}

/// Check if Mika has been initialized (container DB or agents exist).
pub fn is_initialized(home_dir: &Path) -> bool {
    // Container DB exists (normal + legacy layout)
    if container_db_path(home_dir).exists() {
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
/// In the unified model, `data/mika.db` at root is the normal container DB.
/// Legacy means it exists but there's no `agents/` directory yet.
pub fn is_legacy_layout(home_dir: &Path) -> bool {
    container_db_path(home_dir).exists() && !is_multi_agent_layout(home_dir)
}

/// Bootstrap a fresh installation with multi-agent layout.
///
/// Creates the `agents/` directory, initializes the default agent,
/// sets it as active, and writes the root-level global config.
pub fn bootstrap_fresh_install(home_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(home_dir.join("agents"))
        .with_context(|| format!("failed to create {}/agents/", home_dir.display()))?;
    // Create the container-level data directory for the shared database
    std::fs::create_dir_all(home_dir.join("data"))
        .with_context(|| format!("failed to create {}/data/", home_dir.display()))?;
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
/// If legacy layout: creates `agents/mika/`, moves data/, logs/, skills/, exports/,
/// config.toml, identity.toml, soul.md, heartbeat.md, user.md into it.
/// Writes `active_agent` file with "mika".
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
    // NOTE: "data" stays at root (it's the container DB), only logs/skills/exports move
    for dir_name in &["logs", "skills", "exports"] {
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
/// Returns DEFAULT_AGENT ("mika") if file doesn't exist or is empty.
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
# Per-provider API keys go in ~/.mika/.env (auto-loaded, 0600 permissions):
#   MIKA_ANTHROPIC_API_KEY — Anthropic API key
#   MIKA_OPENAI_API_KEY    — OpenAI API key (LLM + optional vector search)
#   MIKA_OPENROUTER_API_KEY — OpenRouter API key
#   MIKA_GROQ_API_KEY      — Groq API key
#   MIKA_MISTRAL_API_KEY   — Mistral API key
#   MIKA_GOOGLE_API_KEY    — Google AI API key
#   MIKA_DEEPSEEK_API_KEY  — DeepSeek API key
#   MIKA_BRAVE_API_KEY     — Brave Search API key (optional, for web search)

log_level = "info"
"#;

/// Create the ~/.mika/ directory structure with default files.
/// Sets permissions to 0700 for directories, 0600 for files on Unix.
pub fn bootstrap(home_dir: &Path) -> Result<()> {
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
        ".env",
    ] {
        let path = home_dir.join(filename);
        if path.exists() {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

pub const DEFAULT_CONFIG: &str = r#"# Mika configuration (per-agent overrides).
# Override with MIKA_* environment variables (highest priority).
#
# Secrets go in ~/.mika/.env (auto-loaded, 0600 permissions).
# Run `mika setup` to configure your API key.

# Active LLM provider — one of: anthropic, openai, openrouter, groq, ollama, mistral, google, deepseek, mikamodel
llm_provider = "anthropic"
llm_max_tokens = 4096
log_level = "info"

# Per-provider model (optional — defaults to provider's recommended model)
# anthropic_model = "claude-sonnet-4-6"
# openai_model = "gpt-4o"
# openrouter_model = "anthropic/claude-sonnet-4"
# groq_model = "llama-3.3-70b-versatile"
# ollama_model = "llama3"
# mistral_model = "mistral-large-latest"
# google_model = "gemini-2.5-flash"
# deepseek_model = "deepseek-chat"
# mikamodel_model = "mika"
"#;

/// Narrow operator-assistant skill allowlist for the default (personal/customer) agent.
///
/// A missing or empty `[skills].allowlist` is treated as *default-permissive* — every
/// bundled skill loads, including the engineering skills (`dev-pilot`, `dev-groom`,
/// `mika-arch-*`, `qa-review*`, `self-dev*`) whose prompts carry the internal
/// `Disposition: READY|ITERATE|ESCALATE` contract. The model then mimics that contract and
/// appends `Disposition:` lines to user-facing replies (mika#1596). Shipping this narrow list
/// in `DEFAULT_IDENTITY` closes that gap; an explicit `[skills]` block in a provisioned
/// `identity.toml` still overrides it (`bootstrap`/`write_default_if_missing` never overwrite
/// an existing file).
///
/// Maintenance: this is the personal/customer-agent counterpart to the well-known-agent
/// allowlists in `crates/mika-agent/src/well_known_agents.rs`. New user-facing skills a
/// personal agent should reach must be added here too. The allowlist↔required_tools coherence
/// guard (mika#1595) runs at agent-load on the resolved surface, so keep this list to skills
/// with self-contained or operator-granted tools. The TOML array in `DEFAULT_IDENTITY` below
/// MUST stay in sync with this constant — `home.rs` tests assert they match.
pub const DEFAULT_AGENT_SKILL_ALLOWLIST: &[&str] = &[
    "calendar",
    "google-workspace",
    "browser-control",
    "desktop",
    "file-reader",
    "mcp",
    "web-search",
    "self-knowledge",
    "shell-exec",
    "tmux",
    "git-ops",
    "gh-read-only",
    // Orchestrator surface (mika#1641): full read/write GitHub via the `github`
    // skill's `run_gh` handler (issue edit/create, pr merge, gh api) on top of the
    // read-only `gh-read-only`. Mika (the executive assistant) assumes the daily-
    // orchestration seat and needs write GitHub reach. `git-ops`, `shell-exec`,
    // `tmux`, and `file-reader` above already cover the rest of the orchestrator
    // tool surface. See docs/operator/mika-orchestrator-handbook.md.
    "github",
];

pub const DEFAULT_IDENTITY: &str = r#"name = "Mika"
emoji = "✦"

[skills]
# Narrow operator-assistant allowlist. A missing/empty allowlist is default-permissive
# (loads every bundled engineering skill) and leaks internal "Disposition:" contracts into
# user-facing replies — see mika#1596. Provisioning an explicit identity.toml overrides this.
# Keep in sync with DEFAULT_AGENT_SKILL_ALLOWLIST in crates/mika-common/src/home.rs and the
# well-known-agent allowlists in crates/mika-agent/src/well_known_agents.rs.
allowlist = [
    "calendar",
    "google-workspace",
    "browser-control",
    "desktop",
    "file-reader",
    "mcp",
    "web-search",
    "self-knowledge",
    "shell-exec",
    "tmux",
    "git-ops",
    "gh-read-only",
    # Orchestrator surface (mika#1641): full read/write GitHub. Keep in sync with
    # DEFAULT_AGENT_SKILL_ALLOWLIST above (home.rs tests assert they match).
    "github",
]

# [kg]
# enabled = true                    # default: true — set false to skip KG for this agent
# docs_root = "/path/to/docs"       # optional; falls back to MIKA_KG_DOCS_ROOT / kg_docs_root / CWD/docs/solutions
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
- When you adapt, replace, or rename something, you own the full outcome — not just the artifact. Trace all references and update them. The job isn't done until the system works end-to-end.
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

        assert!(home.join("logs").is_dir());
        assert!(home.join("config.toml").is_file());
        assert!(home.join("identity.toml").is_file());
        assert!(home.join("soul.md").is_file());
        assert!(home.join("heartbeat.md").is_file());
        assert!(home.join("user.md").is_file());

        // Verify content
        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("llm_provider"));
        assert!(config.contains("mika setup"));

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
        let resolved = resolve_agent_home(tmp.path(), "mika");
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
        let agent = tmp.path().join("agents").join("mika");
        fs::create_dir_all(&agent).unwrap();
        fs::write(agent.join("config.toml"), "# config").unwrap();
        assert!(is_initialized(tmp.path()));
    }

    #[test]
    fn test_migrate_to_multi_agent_from_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        // Set up legacy layout (bootstrap no longer creates data/, so create it
        // manually to simulate the legacy layout that had a per-root data/ dir)
        bootstrap(home).unwrap();
        fs::create_dir_all(home.join("data")).unwrap();
        // Write a marker into the DB so we can verify it stays at root
        fs::write(home.join("data").join("mika.db"), "test-db-content").unwrap();
        fs::write(home.join("soul.md"), "custom soul").unwrap();

        assert!(is_legacy_layout(home));

        // Migrate
        migrate_to_multi_agent(home).unwrap();

        // Verify multi-agent layout
        assert!(is_multi_agent_layout(home));

        // data/ stays at root (container DB)
        assert!(home.join("data").is_dir());
        assert_eq!(
            fs::read_to_string(home.join("data").join("mika.db")).unwrap(),
            "test-db-content"
        );

        // Agent files moved to agents/mika/
        let mika_agent = home.join("agents").join("mika");
        assert_eq!(
            fs::read_to_string(mika_agent.join("soul.md")).unwrap(),
            "custom soul"
        );
        assert!(mika_agent.join("identity.toml").is_file());
        assert!(mika_agent.join("config.toml").is_file());
        assert!(mika_agent.join("skills").is_dir());

        // active_agent file should exist
        assert_eq!(read_active_agent(home), "mika");

        // Root config.toml should be the global one
        let root_config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(root_config.contains("global"));
    }

    #[test]
    fn test_migrate_to_multi_agent_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        // Set up legacy layout and migrate (bootstrap no longer creates data/,
        // so create it manually to simulate the legacy layout)
        bootstrap(home).unwrap();
        fs::create_dir_all(home.join("data")).unwrap();
        fs::write(home.join("data").join("mika.db"), "test-db").unwrap();
        migrate_to_multi_agent(home).unwrap();

        // Migrate again — should be no-op
        migrate_to_multi_agent(home).unwrap();

        // Still works — container DB at root
        assert!(is_multi_agent_layout(home));
        assert_eq!(
            fs::read_to_string(home.join("data").join("mika.db")).unwrap(),
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
        assert_eq!(read_active_agent(tmp.path()), "mika");

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
        assert_eq!(read_active_agent(tmp.path()), "mika");
    }

    #[test]
    fn test_bootstrap_fresh_install() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        bootstrap_fresh_install(home).unwrap();

        // Multi-agent layout created
        assert!(is_multi_agent_layout(home));

        // Container-level data dir created
        assert!(home.join("data").is_dir());

        // Default agent bootstrapped
        let mika_agent = home.join("agents").join("mika");
        assert!(mika_agent.join("logs").is_dir());
        assert!(mika_agent.join("skills").is_dir());
        assert!(mika_agent.join("config.toml").is_file());
        assert!(mika_agent.join("soul.md").is_file());

        // Active agent set to "mika"
        assert_eq!(read_active_agent(home), "mika");

        // Root-level global config written
        let root_config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(root_config.contains("global"));
    }

    /// AC4 (mika#1596): the default identity written by `bootstrap_fresh_install` carries a
    /// narrow `[skills].allowlist` that excludes engineering/verdict-carrying skills and
    /// includes the operator-essential ones — so a fresh personal/customer agent does not
    /// load the architect/dev skills that leak `Disposition:` lines into user-facing replies.
    #[test]
    fn test_bootstrap_fresh_install_writes_narrow_skill_allowlist() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        bootstrap_fresh_install(home).unwrap();

        let identity_path = home.join("agents").join("mika").join("identity.toml");
        let raw = fs::read_to_string(&identity_path).unwrap();
        let parsed: toml::Value = toml::from_str(&raw).unwrap();

        let allowlist: Vec<String> = parsed
            .get("skills")
            .and_then(|s| s.get("allowlist"))
            .and_then(|a| a.as_array())
            .expect("default identity must have an active [skills].allowlist")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        // Non-empty — proves the `apply_identity_allowlist` no-op-on-empty path can't trigger.
        assert!(!allowlist.is_empty());

        // Single source of truth: the written allowlist matches the constant.
        assert_eq!(allowlist, DEFAULT_AGENT_SKILL_ALLOWLIST);

        // AC4 exclusions: no engineering / verdict-carrying skills.
        let excluded = [
            "dev-pilot",
            "dev-groom",
            "mika-arch-groom-ticket",
            "mika-arch-groom-milestone",
            "mika-arch-second-review",
            "qa-review",
            "qa-review-build-callback",
        ];
        for name in excluded {
            assert!(
                !allowlist.iter().any(|s| s == name),
                "default allowlist must exclude engineering skill `{name}`"
            );
        }
        // Prefix guard: nothing self-dev* or mika-arch*.
        for s in &allowlist {
            assert!(
                !s.starts_with("self-dev") && !s.starts_with("mika-arch"),
                "default allowlist must not contain engineering skill `{s}`"
            );
        }

        // AC2 inclusions: operator-essential skills present.
        for name in [
            "calendar",
            "google-workspace",
            "browser-control",
            "desktop",
            "file-reader",
            "web-search",
            "mcp",
        ] {
            assert!(
                allowlist.iter().any(|s| s == name),
                "default allowlist must include operator-essential skill `{name}`"
            );
        }
    }
}
