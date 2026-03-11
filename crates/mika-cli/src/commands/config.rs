//! `mika config` — view and manage configuration.

use anyhow::Result;
use mika_common::claude::is_oauth_token;
use mika_common::config::{CONFIG_KEYS, ConfigBackend, get_effective_value, lookup_config_key};
use std::path::Path;
use std::process::Command;

use crate::cli::{ConfigArgs, ConfigCommand};
use crate::init;

pub async fn run(args: ConfigArgs, agent_name: &str) -> Result<()> {
    match args.command {
        None => run_summary(agent_name).await,
        Some(ConfigCommand::Edit) => run_edit(agent_name),
        Some(ConfigCommand::Soul) => run_soul(agent_name),
        Some(ConfigCommand::Get { key, verbose }) => run_get(agent_name, &key, verbose).await,
        Some(ConfigCommand::Set { key, value }) => run_set(agent_name, &key, value).await,
        Some(ConfigCommand::List { verbose }) => run_list(agent_name, verbose).await,
    }
}

/// Original `mika config` (no subcommand) behavior — print summary.
async fn run_summary(agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;

    println!();
    println!("  Mika Configuration");
    println!("  Home:       {}", ctx.home_dir.display());
    println!("  Model:      {}", ctx.settings.claude_model);
    println!("  Max tokens: {}", ctx.settings.claude_max_tokens);
    println!("  Log level:  {}", ctx.settings.log_level);
    println!("  DB path:    {}", ctx.settings.db_path.display());
    let auth_display = match &ctx.settings.anthropic_api_key {
        Some(key) => {
            if is_oauth_token(key.trim()) {
                "OAuth token [REDACTED]"
            } else {
                "API key [REDACTED]"
            }
        }
        None => "[NOT SET]",
    };
    println!("  Auth:       {}", auth_display);
    println!();

    Ok(())
}

fn run_edit(agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let parts: Vec<&str> = editor.split_whitespace().collect();
    if parts.is_empty() {
        anyhow::bail!("$EDITOR is empty or whitespace-only");
    }
    let identity_path = ctx.home_dir.join("identity.toml");
    let status = Command::new(parts[0])
        .args(&parts[1..])
        .arg(&identity_path)
        .status()?;
    if !status.success() {
        anyhow::bail!("{editor} exited with {status}");
    }
    Ok(())
}

fn run_soul(agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let soul_path = ctx.home_dir.join("soul.md");
    match std::fs::read_to_string(&soul_path) {
        Ok(content) => print!("{content}"),
        Err(_) => println!("No soul.md found at {}", soul_path.display()),
    }
    Ok(())
}

async fn run_get(agent_name: &str, key: &str, verbose: bool) -> Result<()> {
    let info = lookup_config_key(key).ok_or_else(|| {
        anyhow::anyhow!("Unknown config key: {key}\nRun `mika config list` to see all keys")
    })?;

    let ctx = init::init_db_only_for_agent(agent_name)?;

    let value = if info.backend == ConfigBackend::Database {
        // DB keys need async lookup
        ctx.async_db.get_customer_config(key).await?
    } else {
        get_effective_value(key, &ctx.settings)
    };

    let display_value = format_display_value(&value, info.secret);

    if verbose {
        let source = resolve_source(key, info, &ctx.home_dir, &ctx.global_home);
        println!(
            "{key} = {display_value} (source: {source}, backend: {:?})",
            info.backend
        );
    } else {
        println!("{display_value}");
    }

    Ok(())
}

async fn run_set(agent_name: &str, key: &str, value: Option<String>) -> Result<()> {
    let info = lookup_config_key(key).ok_or_else(|| {
        anyhow::anyhow!("Unknown config key: {key}\nRun `mika config list` to see all keys")
    })?;

    if info.backend == ConfigBackend::ReadOnly {
        anyhow::bail!("{key} is read-only and cannot be changed");
    }

    let ctx = init::init_db_only_for_agent(agent_name)?;

    match info.backend {
        ConfigBackend::File => {
            let val =
                value.ok_or_else(|| anyhow::anyhow!("Usage: mika config set {key} <value>"))?;
            mika_common::validation::validate_file_key(key, &val)
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            // Write to agent config.toml (per-agent override)
            let config_path = ctx.home_dir.join("config.toml");
            write_config_toml(&config_path, key, &val)?;

            // Warn if env var overrides
            if let Some(env_var) = info.env_var
                && std::env::var(env_var).is_ok()
            {
                eprintln!(
                    "Note: {env_var} environment variable is set and takes priority over config.toml"
                );
            }

            println!("Set {key} = {val}");
        }
        ConfigBackend::Env => {
            // Secret keys: never accept value from CLI args
            if value.is_some() {
                eprintln!("Warning: secret keys should not be passed as CLI arguments.");
                eprintln!("The value argument will be ignored. You will be prompted securely.");
            }

            // Require TTY for interactive prompt
            if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                anyhow::bail!(
                    "Secret keys require an interactive terminal.\n\
                     Set {} as an environment variable instead.",
                    info.env_var.unwrap_or("the corresponding MIKA_* var")
                );
            }

            let secret = dialoguer::Password::new()
                .with_prompt(format!("Enter value for {key}"))
                .interact()?;

            if secret.trim().is_empty() {
                anyhow::bail!("Value cannot be empty");
            }

            let env_key = info
                .env_var
                .ok_or_else(|| anyhow::anyhow!("No env var mapping for {key}"))?;
            mika_common::dotenv::set_env_var(&ctx.global_home, env_key, &secret)?;
            println!("Set {key} in ~/.mika/.env");
        }
        ConfigBackend::Database => {
            let val =
                value.ok_or_else(|| anyhow::anyhow!("Usage: mika config set {key} <value>"))?;
            mika_agent::config_keys::validate_config_value(key, &val)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            ctx.async_db.set_customer_config(key, &val).await?;
            println!("Set {key} = {val}");
        }
        ConfigBackend::ReadOnly => unreachable!(),
    }

    Ok(())
}

async fn run_list(agent_name: &str, verbose: bool) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;

    // Pre-fetch all DB config
    let db_configs = ctx.async_db.list_customer_config().await?;

    println!();
    println!("  Mika Configuration Keys");
    println!();

    for info in CONFIG_KEYS {
        let value = if info.backend == ConfigBackend::Database {
            db_configs
                .iter()
                .find(|(k, _)| k == info.key)
                .map(|(_, v)| v.clone())
        } else {
            get_effective_value(info.key, &ctx.settings)
        };

        let display_value = format_display_value(&value, info.secret);

        if verbose {
            let source = resolve_source(info.key, info, &ctx.home_dir, &ctx.global_home);
            println!(
                "  {:<24} {:<30} ({}, {:?})",
                info.key, display_value, source, info.backend
            );
        } else {
            println!("  {:<24} {}", info.key, display_value);
        }
    }
    println!();

    Ok(())
}

/// Format a config value for display, redacting secrets.
fn format_display_value(value: &Option<String>, secret: bool) -> String {
    match value {
        Some(v) if secret => {
            if v.is_empty() {
                "[NOT SET]".to_string()
            } else {
                "[REDACTED]".to_string()
            }
        }
        Some(v) => v.clone(),
        None => "[NOT SET]".to_string(),
    }
}

/// Determine where the current value is coming from.
fn resolve_source(
    key: &str,
    info: &mika_common::config::ConfigKeyInfo,
    agent_home: &Path,
    global_home: &Path,
) -> String {
    // Check env var override (highest priority for File/Env backend keys)
    if let Some(env_var) = info.env_var
        && std::env::var(env_var).is_ok()
    {
        return format!("env var ({env_var})");
    }

    match info.backend {
        ConfigBackend::File => {
            // Check agent config.toml
            let agent_config = agent_home.join("config.toml");
            if toml_key_exists(&agent_config, key) {
                return "agent config.toml".to_string();
            }
            // Check global config.toml
            let global_config = global_home.join("config.toml");
            if toml_key_exists(&global_config, key) {
                return "global config.toml".to_string();
            }
            "default".to_string()
        }
        ConfigBackend::Env => {
            // Check .env file
            let env_path = global_home.join(".env");
            if let Some(env_var) = info.env_var
                && env_key_exists(&env_path, env_var)
            {
                return ".env file".to_string();
            }
            "not set".to_string()
        }
        ConfigBackend::Database => "database".to_string(),
        ConfigBackend::ReadOnly => "runtime".to_string(),
    }
}

/// Check if a key exists in a TOML file.
fn toml_key_exists(path: &Path, key: &str) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.parse::<toml::Table>().ok())
        .is_some_and(|t| t.contains_key(key))
}

/// Check if a key exists in a .env file.
fn env_key_exists(path: &Path, key: &str) -> bool {
    std::fs::read_to_string(path).ok().is_some_and(|content| {
        content.lines().any(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with('#')
                && !trimmed.is_empty()
                && trimmed
                    .split_once('=')
                    .is_some_and(|(k, _)| k.trim() == key)
        })
    })
}

/// Write a key-value pair to a TOML config file using toml_edit to preserve comments.
/// Creates the file if it doesn't exist. Uses atomic write (temp + rename).
fn write_config_toml(path: &Path, key: &str, value: &str) -> Result<()> {
    let content = if path.exists() {
        std::fs::read_to_string(path)?
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse {}: {e}", path.display()))?;

    // Set the value with appropriate type
    let toml_value = match key {
        "claude_max_tokens" | "server_port" | "embedding_dimensions" => {
            let n: i64 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("{key} must be an integer"))?;
            toml_edit::value(n)
        }
        _ => toml_edit::value(value),
    };
    doc[key] = toml_value;

    // Atomic write
    let tmp_path = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("config.toml")
    ));
    std::fs::write(&tmp_path, doc.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600));
    }

    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_config_toml_creates_new() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        write_config_toml(&path, "claude_model", "claude-opus-4-6").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("claude_model = \"claude-opus-4-6\""));
    }

    #[test]
    fn test_write_config_toml_preserves_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "# My comment\nlog_level = \"info\"\n").unwrap();

        write_config_toml(&path, "claude_model", "claude-opus-4-6").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# My comment"));
        assert!(content.contains("log_level = \"info\""));
        assert!(content.contains("claude_model = \"claude-opus-4-6\""));
    }

    #[test]
    fn test_write_config_toml_updates_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "claude_model = \"old-model\"\n").unwrap();

        write_config_toml(&path, "claude_model", "new-model").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("claude_model = \"new-model\""));
        assert!(!content.contains("old-model"));
    }

    #[test]
    fn test_write_config_toml_integer_value() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        write_config_toml(&path, "claude_max_tokens", "8192").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("claude_max_tokens = 8192"));
    }

    #[test]
    fn test_toml_key_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "claude_model = \"test\"\n").unwrap();

        assert!(toml_key_exists(&path, "claude_model"));
        assert!(!toml_key_exists(&path, "log_level"));
    }

    #[test]
    fn test_env_key_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".env");
        std::fs::write(&path, "# comment\nMIKA_ANTHROPIC_API_KEY=\"test\"\n").unwrap();

        assert!(env_key_exists(&path, "MIKA_ANTHROPIC_API_KEY"));
        assert!(!env_key_exists(&path, "MIKA_OPENAI_API_KEY"));
    }

    #[test]
    fn test_env_key_exists_ignores_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".env");
        std::fs::write(&path, "# MIKA_KEY=value\n").unwrap();

        assert!(!env_key_exists(&path, "MIKA_KEY"));
    }

    #[cfg(unix)]
    #[test]
    fn test_write_config_toml_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        write_config_toml(&path, "log_level", "debug").unwrap();

        let perms = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(perms, 0o600);
    }

    #[test]
    fn test_format_display_value() {
        assert_eq!(format_display_value(&Some("hello".into()), false), "hello");
        assert_eq!(
            format_display_value(&Some("secret".into()), true),
            "[REDACTED]"
        );
        assert_eq!(format_display_value(&Some("".into()), true), "[NOT SET]");
        assert_eq!(format_display_value(&None, false), "[NOT SET]");
        assert_eq!(format_display_value(&None, true), "[NOT SET]");
    }

    #[test]
    fn test_get_effective_value_covers_all_non_db_non_env_keys() {
        use mika_common::config::{CONFIG_KEYS, ConfigBackend};

        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // Create minimal config.toml so Settings::load doesn't fail
        std::fs::write(home.join("config.toml"), "").unwrap();
        let settings = mika_common::config::Settings::load(home).unwrap();

        for info in CONFIG_KEYS {
            match info.backend {
                ConfigBackend::File | ConfigBackend::ReadOnly => {
                    // Should not panic — every File/ReadOnly key must have a branch
                    let _ = get_effective_value(info.key, &settings);
                }
                ConfigBackend::Env | ConfigBackend::Database => {
                    // Env/DB keys are resolved through other paths, skip
                }
            }
        }
    }
}
