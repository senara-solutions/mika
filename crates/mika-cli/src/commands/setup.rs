use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context, Result, bail};
use dialoguer::{Confirm, Input, Password};
use mika_common::home;

use crate::cli::SetupMode;

pub async fn run(agent_name: &str, mode: SetupMode, api_key: Option<&str>) -> Result<()> {
    // Compose mode is a standalone flow — no ~/.mika bootstrapping
    if matches!(mode, SetupMode::Compose) {
        return run_compose_generation();
    }

    let home_dir = home::resolve_home_dir()?;
    let already_initialized = home::is_initialized(&home_dir);

    if already_initialized {
        println!("\n  Mika is already initialized. Checking configuration...");
    } else {
        home::bootstrap_fresh_install(&home_dir)?;

        // If a custom agent name was requested, bootstrap it and set as active
        if agent_name != mika_common::agent::DEFAULT_AGENT {
            home::bootstrap_agent(&home_dir, agent_name)
                .with_context(|| format!("failed to initialize agent '{agent_name}'"))?;
            home::write_active_agent(&home_dir, agent_name)?;
        }
    }

    // Non-interactive API key provisioning (for CI/automation)
    if let Some(key) = api_key {
        let key = key.trim();
        if !key.is_empty() {
            mika_common::dotenv::set_env_var(&home_dir, "MIKA_LLM_API_KEY", key)?;
            println!("  API key saved to {}", home_dir.join(".env").display());
        }
        // If non-interactive and --api-key provided, skip the interactive wizard
        if !std::io::stdin().is_terminal() {
            // Auto-generate internal token if missing
            if !secret_is_set("MIKA_INTERNAL_TOKEN") {
                let token = generate_token();
                mika_common::dotenv::set_env_var(&home_dir, "MIKA_INTERNAL_TOKEN", &token)?;
            }
            println!("  Setup complete (non-interactive).");
            return Ok(());
        }
    }

    // TTY guard: interactive prompts require a terminal (#607)
    let interactive = std::io::stdin().is_terminal();
    if !interactive && !already_initialized {
        bail!(
            "mika setup requires an interactive terminal for first-time configuration. \
             Use `mika setup --api-key <key>` for non-interactive setup, \
             or pre-set MIKA_LLM_API_KEY and other env vars."
        );
    }

    let mode_label = match mode {
        SetupMode::Cli => "CLI",
        SetupMode::Server => "Server",
        SetupMode::Compose => unreachable!(),
    };
    println!("\n  Mika Setup ({mode_label} Mode)\n");

    let mut env_written = false;
    let mut config_written = false;

    // --- CLI prompts (shared by CLI and Server modes) ---
    run_cli_prompts(
        &home_dir,
        interactive,
        &mut env_written,
        &mut config_written,
    )?;

    // --- Server-specific prompts ---
    if matches!(mode, SetupMode::Server) && interactive {
        run_server_prompts(&home_dir, &mut env_written, &mut config_written)?;
    }

    // Summary
    println!();
    if env_written {
        println!("  Secrets saved to {}", home_dir.join(".env").display());
    }
    if config_written {
        println!(
            "  Config saved to {}",
            home_dir.join("config.toml").display()
        );
    }

    let agent_home = home::resolve_agent_home(&home_dir, agent_name);
    if !already_initialized {
        println!("  \u{2726} Mika initialized at {}", agent_home.display());
    }

    if !env_written && !config_written && already_initialized {
        println!("  All configuration is already set. Nothing to do.");
    }

    println!();
    Ok(())
}

/// CLI mode prompts: API keys, telemetry, internal token.
fn run_cli_prompts(
    home_dir: &Path,
    interactive: bool,
    env_written: &mut bool,
    config_written: &mut bool,
) -> Result<()> {
    // 1. LLM API key
    if interactive && !secret_is_set("MIKA_LLM_API_KEY") {
        *env_written |= prompt_optional_secret(home_dir, "MIKA_LLM_API_KEY", "  LLM API key")?;
    }

    // 2. Brave Search API key (optional)
    if interactive && !secret_is_set("MIKA_BRAVE_API_KEY") {
        *env_written |= prompt_optional_secret(
            home_dir,
            "MIKA_BRAVE_API_KEY",
            "  Brave Search API key (optional, press Enter to skip)",
        )?;
    }

    // 3. GitHub integration (optional)
    if interactive && !secret_is_set("MIKA_INVESTIGATE_GITHUB_TOKEN") {
        *env_written |= prompt_optional_secret(
            home_dir,
            "MIKA_INVESTIGATE_GITHUB_TOKEN",
            "  GitHub token for dashboard issue creation (optional, press Enter to skip)",
        )?;
    }
    if interactive && !secret_is_set("MIKA_GITHUB_REPO") {
        *env_written |= prompt_optional_value(
            home_dir,
            "MIKA_GITHUB_REPO",
            "  GitHub repo for dashboard issues, e.g. owner/repo (optional, press Enter to skip)",
        )?;
    }

    // 4. Telemetry
    if interactive && !config_key_is_set(home_dir, "telemetry_enabled") {
        let enable = Confirm::new()
            .with_prompt("  Enable telemetry?")
            .default(false)
            .interact()?;

        if enable {
            set_config_toml_value(home_dir, "telemetry_enabled", toml::Value::Boolean(true))?;
            *config_written = true;

            let endpoint: String = Input::new()
                .with_prompt("  OTLP endpoint")
                .interact_text()?;
            let endpoint = endpoint.trim();
            if !endpoint.is_empty() {
                set_config_toml_value(
                    home_dir,
                    "otlp_endpoint",
                    toml::Value::String(endpoint.to_string()),
                )?;
            }

            if !secret_is_set("MIKA_OTLP_AUTH_HEADER") {
                *env_written |= prompt_optional_secret(
                    home_dir,
                    "MIKA_OTLP_AUTH_HEADER",
                    "  OTLP auth header (optional, press Enter to skip)",
                )?;
            }
        }
    }

    // 5. Internal token (auto-generate, no prompt)
    if !secret_is_set("MIKA_INTERNAL_TOKEN") {
        let token = generate_token();
        mika_common::dotenv::set_env_var(home_dir, "MIKA_INTERNAL_TOKEN", &token)?;
        *env_written = true;
        println!("\n  Generated internal token for server mode.");
    }

    Ok(())
}

/// Server mode prompts: routing URL, dashboard token, customer ID, server port.
fn run_server_prompts(
    home_dir: &Path,
    env_written: &mut bool,
    config_written: &mut bool,
) -> Result<()> {
    println!("\n  Server Configuration\n");

    // MIKA_ROUTING_URL (required for server)
    if !secret_is_set("MIKA_ROUTING_URL") {
        let url: String = Input::new()
            .with_prompt("  Gateway URL (MIKA_ROUTING_URL)")
            .interact_text()?;
        let url = url.trim();
        if !url.is_empty() {
            reqwest::Url::parse(url).map_err(|e| anyhow::anyhow!("Invalid URL: {e}"))?;
            mika_common::dotenv::set_env_var(home_dir, "MIKA_ROUTING_URL", url)?;
            *env_written = true;
        }
    }

    // MIKA_DASHBOARD_TOKEN (auto-generate)
    if !secret_is_set("MIKA_DASHBOARD_TOKEN") {
        let token = generate_token();
        mika_common::dotenv::set_env_var(home_dir, "MIKA_DASHBOARD_TOKEN", &token)?;
        *env_written = true;
        println!("\n  Generated dashboard token.");
    }

    // MIKA_SERVER_PORT (optional, default 8080) — non-secret, goes to config.toml
    if !config_key_is_set(home_dir, "server_port") {
        let port: String = Input::new()
            .with_prompt("  Server port (default: 8080)")
            .default("8080".to_string())
            .interact_text()?;
        let port = port.trim();
        if port != "8080" {
            let port_num: u16 = port
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid port number: {port}"))?;
            set_config_toml_value(
                home_dir,
                "server_port",
                toml::Value::Integer(i64::from(port_num)),
            )?;
            *config_written = true;
        }
    }

    Ok(())
}

/// Compose mode: generate a standalone .env file for docker-compose in CWD.
fn run_compose_generation() -> Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!("mika setup --mode compose requires an interactive terminal");
    }

    println!("\n  Docker Compose Setup\n");
    println!("  This will generate a .env file for docker-compose in the current directory.\n");

    // --- Required secrets ---
    let api_key = Password::new().with_prompt("  LLM API key").interact()?;
    let api_key = api_key.trim();
    if api_key.is_empty() {
        bail!("LLM API key is required");
    }

    let telegram_token = Password::new()
        .with_prompt("  Telegram Bot token")
        .interact()?;
    let telegram_token = telegram_token.trim();
    if telegram_token.is_empty() {
        bail!("Telegram Bot token is required");
    }

    let webhook_url: String = Input::new()
        .with_prompt("  Telegram webhook URL (public HTTPS)")
        .interact_text()?;
    let webhook_url = webhook_url.trim();
    if webhook_url.is_empty() {
        bail!("Telegram webhook URL is required");
    }
    reqwest::Url::parse(webhook_url).map_err(|e| anyhow::anyhow!("Invalid webhook URL: {e}"))?;

    // --- Optional secrets ---
    let brave_key = Password::new()
        .with_prompt("  Brave Search API key (optional, press Enter to skip)")
        .allow_empty_password(true)
        .interact()?;
    let brave_key = brave_key.trim();

    let openai_key = Password::new()
        .with_prompt("  OpenAI API key for embeddings (optional, press Enter to skip)")
        .allow_empty_password(true)
        .interact()?;
    let openai_key = openai_key.trim();

    let github_token = Password::new()
        .with_prompt("  GitHub token for dashboard issue creation (optional, press Enter to skip)")
        .allow_empty_password(true)
        .interact()?;
    let github_token = github_token.trim();

    let github_repo: String = Input::new()
        .with_prompt(
            "  GitHub repo for dashboard issues, e.g. owner/repo (optional, press Enter to skip)",
        )
        .allow_empty(true)
        .interact_text()?;
    let github_repo = github_repo.trim().to_string();

    // --- Auto-generate tokens ---
    let internal_token = generate_token();
    let webhook_secret = generate_token();
    let dashboard_token = generate_token();

    // --- Build .env content ---
    let mut lines: Vec<String> = Vec::new();
    lines.push("# Generated by: mika setup --mode compose".to_string());
    lines.push(String::new());

    // Shared
    lines.push("# ── Shared ──".to_string());
    lines.push(format!("MIKA_INTERNAL_TOKEN={internal_token}"));
    lines.push(format!("MIKA_LLM_API_KEY={api_key}"));
    if !brave_key.is_empty() {
        lines.push(format!("MIKA_BRAVE_API_KEY={brave_key}"));
    }
    if !openai_key.is_empty() {
        lines.push(format!("MIKA_OPENAI_API_KEY={openai_key}"));
    }
    if !github_token.is_empty() {
        lines.push(format!("MIKA_INVESTIGATE_GITHUB_TOKEN={github_token}"));
    }
    if !github_repo.is_empty() {
        lines.push(format!("MIKA_GITHUB_REPO={github_repo}"));
    }
    lines.push(String::new());

    // Agent / Server
    lines.push("# ── Agent (mika-server) ──".to_string());
    lines.push("MIKA_ROUTING_URL=http://gateway:8080".to_string());
    lines.push(format!("MIKA_DASHBOARD_TOKEN={dashboard_token}"));
    lines.push(String::new());

    // Gateway
    lines.push("# ── Gateway (mika-gateway) ──".to_string());
    lines.push("MIKA_DATABASE_URL=postgres://mika:mika@postgres:5432/mika".to_string());
    lines.push(format!("MIKA_TELEGRAM_BOT_TOKEN={telegram_token}"));
    lines.push(format!("MIKA_TELEGRAM_WEBHOOK_SECRET={webhook_secret}"));
    lines.push(format!("MIKA_TELEGRAM_WEBHOOK_URL={webhook_url}"));
    lines.push(String::new());

    let content = lines.join("\n");

    // Write to CWD/.env
    let env_path = std::env::current_dir()?.join(".env");
    if env_path.exists() {
        let overwrite = Confirm::new()
            .with_prompt(format!(
                "  {} already exists. Overwrite?",
                env_path.display()
            ))
            .default(false)
            .interact()?;
        if !overwrite {
            println!("\n  Aborted.");
            return Ok(());
        }
    }

    let tmp_path = env_path.with_file_name(".env.tmp");
    std::fs::write(&tmp_path, &content)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
        {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e).context("failed to set permissions on .env");
        }
    }

    std::fs::rename(&tmp_path, &env_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            env_path.display()
        )
    })?;

    println!("\n  .env written to {}", env_path.display());
    println!("  Contains all variables for server, gateway, and postgres.");
    println!(
        "\n  Next: place a docker-compose.yml alongside this .env and run `docker compose up`."
    );
    println!();
    Ok(())
}

/// Prompt for an optional secret with masked input. Returns true if a value was written.
fn prompt_optional_secret(home_dir: &Path, env_key: &str, prompt: &str) -> Result<bool> {
    let value = Password::new()
        .with_prompt(prompt)
        .allow_empty_password(true)
        .interact()?;
    let value = value.trim();
    if value.is_empty() {
        return Ok(false);
    }
    mika_common::dotenv::set_env_var(home_dir, env_key, value)?;
    Ok(true)
}

/// Prompt for an optional non-secret value with visible input. Returns true if a value was written.
fn prompt_optional_value(home_dir: &Path, env_key: &str, prompt: &str) -> Result<bool> {
    let value: String = Input::new()
        .with_prompt(prompt)
        .allow_empty(true)
        .interact_text()?;
    let value = value.trim();
    if value.is_empty() {
        return Ok(false);
    }
    mika_common::dotenv::set_env_var(home_dir, env_key, value)?;
    Ok(true)
}

/// Check if a secret is already available in the process environment.
/// Since `load_dotenv()` runs before setup, `.env` values are already loaded.
fn secret_is_set(key: &str) -> bool {
    std::env::var(key).ok().filter(|v| !v.is_empty()).is_some()
}

/// Check if a config key is already set in config.toml.
fn config_key_is_set(home_dir: &Path, key: &str) -> bool {
    let config_path = home_dir.join("config.toml");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return false;
    };
    let table: toml::Table = content.parse().unwrap_or_default();
    table.contains_key(key)
}

/// Generate 64-char hex token (32 random bytes).
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

/// Set a key=value in `{home_dir}/config.toml` using proper TOML serialization.
/// Creates the file if it doesn't exist. Uses atomic write with 0600 permissions.
fn set_config_toml_value(home_dir: &Path, key: &str, value: toml::Value) -> Result<()> {
    let config_path = home_dir.join("config.toml");
    let mut table: toml::Table = match std::fs::read_to_string(&config_path) {
        Ok(content) => content.parse().unwrap_or_default(),
        Err(_) => toml::Table::new(),
    };
    table.insert(key.to_string(), value);

    let content = toml::to_string_pretty(&table)
        .with_context(|| format!("failed to serialize config for {}", config_path.display()))?;

    // Atomic write: write to temp file, set permissions, then rename
    let tmp_path = config_path.with_file_name("config.toml.tmp");
    std::fs::write(&tmp_path, &content)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
        {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e.into());
        }
    }

    std::fs::rename(&tmp_path, &config_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            config_path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token_length_and_hex() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_token_uniqueness() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_set_config_toml_value_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        set_config_toml_value(tmp.path(), "telemetry_enabled", toml::Value::Boolean(true)).unwrap();

        let content = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(content.contains("telemetry_enabled = true"));
    }

    #[test]
    fn test_set_config_toml_value_updates_existing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "telemetry_enabled = false\nlog_level = \"info\"\n",
        )
        .unwrap();

        set_config_toml_value(tmp.path(), "telemetry_enabled", toml::Value::Boolean(true)).unwrap();

        let content = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(content.contains("telemetry_enabled = true"));
        assert!(content.contains("log_level = \"info\""));
        assert_eq!(content.matches("telemetry_enabled").count(), 1);
    }

    #[test]
    fn test_set_config_toml_value_handles_special_chars() {
        let tmp = tempfile::tempdir().unwrap();
        // Values with quotes and special chars should be properly escaped by the toml crate
        set_config_toml_value(
            tmp.path(),
            "otlp_endpoint",
            toml::Value::String("https://example.com/path?a=1&b=\"2\"".to_string()),
        )
        .unwrap();

        let content = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        // Verify it roundtrips through TOML parsing
        let table: toml::Table = content.parse().unwrap();
        assert_eq!(
            table.get("otlp_endpoint").unwrap().as_str().unwrap(),
            "https://example.com/path?a=1&b=\"2\""
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_set_config_toml_value_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        set_config_toml_value(tmp.path(), "key", toml::Value::Boolean(true)).unwrap();

        let perms = std::fs::metadata(tmp.path().join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(perms, 0o600);
    }

    #[test]
    fn test_config_key_is_set() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!config_key_is_set(tmp.path(), "telemetry_enabled"));

        std::fs::write(tmp.path().join("config.toml"), "telemetry_enabled = true\n").unwrap();
        assert!(config_key_is_set(tmp.path(), "telemetry_enabled"));
    }

    #[test]
    fn test_secret_is_set_checks_env() {
        let key = "MIKA_TEST_SECRET_IS_SET_CHECK";
        // Ensure clean state
        unsafe { std::env::remove_var(key) };
        assert!(!secret_is_set(key));

        unsafe { std::env::set_var(key, "val") };
        assert!(secret_is_set(key));

        // Empty value should not count as set
        unsafe { std::env::set_var(key, "") };
        assert!(!secret_is_set(key));

        // Cleanup
        unsafe { std::env::remove_var(key) };
    }
}
