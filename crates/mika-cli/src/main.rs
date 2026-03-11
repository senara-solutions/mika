mod cli;
mod commands;
mod init;
mod tui;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};
use mika_common::agent;
use mika_common::home;
use mika_common::logging::LogOutput;
use mika_common::team;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Team mode: branch early, before agent resolution.
    // --team is mutually exclusive with --agent (enforced by clap).
    if let Some(ref team_name) = cli.team {
        let team_name = team::normalize_team_name(team_name);
        team::validate_team_name(&team_name)?;
        let global_home = home::resolve_home_dir()?;
        mika_common::dotenv::load_dotenv(&global_home);

        if !team::team_exists(&global_home, &team_name) {
            anyhow::bail!("Team '{team_name}' not found.");
        }

        // Only allow bare `mika --team` or `mika --team chat`
        match cli.command {
            None | Some(Commands::Chat) => {
                let log_level = resolve_log_level(&[global_home.join("config.toml")]);

                // Build optional OTel export layer (feature-gated, graceful degradation)
                let (otel_layer, _telemetry_guard) =
                    mika_common::config::Settings::load(&global_home)
                        .ok()
                        .map(|s| mika_common::telemetry::try_init_otel(&s))
                        .unwrap_or((None, None));

                let log_dir = team::team_dir(&global_home, &team_name).join("logs");
                let _log_guard = mika_common::logging::init_pretty(
                    &log_level,
                    Some(&log_dir),
                    LogOutput::FileOnly,
                    otel_layer,
                );

                return commands::chat::run_team(&team_name, &global_home).await;
            }
            Some(_) => {
                anyhow::bail!(
                    "--team cannot be used with subcommands other than 'chat'. \
                     Use 'mika teams run {team_name} \"goal\"' for non-interactive team runs."
                );
            }
        }
    }

    // Resolve agent name first — needed for correct log directory.
    // Priority: --agent flag > active_agent file > "mika"
    let agent_name = match cli.agent {
        Some(name) => {
            let name = agent::normalize_agent_name(&name);
            agent::validate_agent_name(&name)?;
            name
        }
        None => init::resolve_active_agent()?,
    };

    // Load .env from ~/.mika/.env (secrets, does not override shell env vars)
    let global_home = home::resolve_home_dir().ok();
    if let Some(ref h) = global_home {
        mika_common::dotenv::load_dotenv(h);
    }

    // Resolve log directory: ~/.mika/agents/{name}/logs/
    // Uses agent-specific home so logs land in the correct agent directory.
    let agent_home = global_home
        .as_ref()
        .map(|h| home::resolve_agent_home(h, &agent_name));
    let log_dir = agent_home.as_ref().map(|h| h.join("logs"));

    // Generate CLI reference markdown in the agent home directory.
    // Used by the self-knowledge skill so the agent can discover its own commands.
    // Only writes when content changes to avoid unnecessary fs writes.
    if let Some(ref home) = agent_home {
        let cmd = Cli::command();
        let markdown = clap_markdown::help_markdown_command(&cmd);
        let reference_path = home.join("cli-reference.md");
        let should_write =
            std::fs::read_to_string(&reference_path).ok().as_deref() != Some(&markdown);
        if should_write {
            let _ = std::fs::write(&reference_path, &markdown);
        }
    }

    // Read log_level from agent config, falling back to global config.
    let mut config_paths = Vec::new();
    if let Some(ref h) = agent_home {
        config_paths.push(h.join("config.toml"));
    }
    if let Some(ref h) = global_home {
        config_paths.push(h.join("config.toml"));
    }
    let log_level = resolve_log_level(&config_paths);

    // Build optional OTel export layer (feature-gated, graceful degradation)
    let (otel_layer, _telemetry_guard) = global_home
        .as_ref()
        .and_then(|h| mika_common::config::Settings::load(h).ok())
        .map(|s| mika_common::telemetry::try_init_otel(&s))
        .unwrap_or((None, None));

    // Initialize tracing with correct agent-specific directory and configured level.
    // Use FileOnly in TUI mode — ratatui's EnterAlternateScreen only covers stdout,
    // so stderr output would corrupt the TUI display.
    // The _log_guard MUST stay alive until the end of main — dropping it stops file logging.
    let is_tui = matches!(cli.command, None | Some(Commands::Chat));
    let log_output = if is_tui {
        LogOutput::FileOnly
    } else {
        LogOutput::PrettyAndFile
    };
    let _log_guard =
        mika_common::logging::init_pretty(&log_level, log_dir.as_deref(), log_output, otel_layer);

    match cli.command {
        // Bare `mika` with no subcommand: auto-setup if needed, then chat
        None => {
            let home_dir = home::resolve_home_dir()?;
            if !home::is_initialized(&home_dir) {
                commands::setup::run(&agent_name, cli::SetupMode::Cli, None).await?;
            }
            commands::chat::run(&agent_name, cli.session.as_deref()).await
        }
        Some(Commands::Chat) => commands::chat::run(&agent_name, cli.session.as_deref()).await,
        Some(Commands::Setup { mode, api_key }) => {
            commands::setup::run(&agent_name, mode, api_key.as_deref()).await
        }
        Some(Commands::Memory(args)) => commands::memory::run(args, &agent_name).await,
        Some(Commands::Reminders(args)) => commands::reminders::run(args, &agent_name).await,
        Some(Commands::Status) => commands::status::run(&agent_name).await,
        Some(Commands::Config(args)) => commands::config::run(args, &agent_name).await,
        Some(Commands::Skills(args)) => commands::skills::run(args, &agent_name).await,
        Some(Commands::Ask {
            message,
            task_id,
            parent_task,
        }) => {
            match commands::ask::run(
                &message,
                &agent_name,
                task_id.as_deref(),
                cli.session.as_deref(),
                parent_task.as_deref(),
            )
            .await
            {
                Ok(()) => Ok(()),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Agents(args)) => commands::agents::run(args).await,
        Some(Commands::Teams(args)) => commands::teams::run(args).await,
        Some(Commands::Mcp(args)) => commands::mcp::run(args, &agent_name).await,
        Some(Commands::Tasks(args)) => commands::tasks::run(args, &agent_name).await,
        Some(Commands::Doctor(args)) => commands::doctor::run(args, &agent_name).await,
    }
}

/// Resolve log level from env var or config files (first match wins).
fn resolve_log_level(config_paths: &[std::path::PathBuf]) -> String {
    std::env::var("MIKA_LOG_LEVEL")
        .ok()
        .filter(|s| {
            matches!(
                s.as_str(),
                "trace" | "debug" | "info" | "warn" | "error" | "off"
            )
        })
        .or_else(|| {
            config_paths.iter().find_map(|path| {
                std::fs::read_to_string(path)
                    .ok()
                    .and_then(|content| parse_log_level(&content))
            })
        })
        .unwrap_or_else(|| "warn".to_string())
}

/// Extract `log_level` value from a TOML config string.
/// Uses the `toml` crate (already a dependency) for correct parsing —
/// handles comments, sections, and avoids prefix-matching false positives.
fn parse_log_level(content: &str) -> Option<String> {
    let table: toml::Table = content.parse().ok()?;
    let level = table.get("log_level")?.as_str().filter(|s| !s.is_empty())?;
    // Only accept standard tracing levels to prevent filter directive injection
    match level {
        "trace" | "debug" | "info" | "warn" | "error" | "off" => Some(level.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mika_common::home;

    #[test]
    fn test_parse_log_level_quoted() {
        assert_eq!(
            parse_log_level("log_level = \"debug\"\n"),
            Some("debug".to_string())
        );
    }

    #[test]
    fn test_parse_log_level_with_other_fields() {
        let content =
            "claude_model = \"claude-sonnet-4-6\"\nlog_level = \"info\"\nmax_tokens = 4096\n";
        assert_eq!(parse_log_level(content), Some("info".to_string()));
    }

    #[test]
    fn test_parse_log_level_missing() {
        assert_eq!(
            parse_log_level("claude_model = \"claude-sonnet-4-6\"\n"),
            None
        );
    }

    #[test]
    fn test_parse_log_level_empty_value() {
        assert_eq!(parse_log_level("log_level = \"\"\n"), None);
    }

    #[test]
    fn test_parse_log_level_rejects_filter_directive() {
        // Complex tracing filter directives should be rejected — only simple levels allowed
        assert_eq!(
            parse_log_level("log_level = \"mika_agent::server=trace\"\n"),
            None
        );
    }

    #[test]
    fn test_log_dir_multi_agent_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join("agents")).unwrap();

        let log_dir = home::resolve_agent_home(home, "work").join("logs");
        assert_eq!(log_dir, home.join("agents").join("work").join("logs"));
    }

    #[test]
    fn test_log_dir_legacy_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // No agents/ dir → legacy layout → resolve_agent_home returns home

        let log_dir = home::resolve_agent_home(home, "mika").join("logs");
        assert_eq!(log_dir, home.join("logs"));
    }

    #[test]
    fn test_env_var_log_level_allowlist() {
        // Validates the same allowlist used for MIKA_LOG_LEVEL env var
        let valid = ["trace", "debug", "info", "warn", "error", "off"];
        for level in valid {
            assert!(
                matches!(level, "trace" | "debug" | "info" | "warn" | "error" | "off"),
                "Expected {level} to match allowlist"
            );
        }
        assert!(!matches!(
            "mika_agent=trace",
            "trace" | "debug" | "info" | "warn" | "error" | "off"
        ));
    }

    /// Verify clap-markdown output contains all top-level command names.
    /// If you add a new top-level command to the `Commands` enum in cli.rs,
    /// this test will catch it.
    #[test]
    fn test_clap_markdown_contains_all_commands() {
        use clap::CommandFactory;
        let cmd = crate::cli::Cli::command();
        let markdown = clap_markdown::help_markdown_command(&cmd);
        for name in [
            "chat",
            "setup",
            "memory",
            "reminders",
            "status",
            "config",
            "skills",
            "ask",
            "agents",
            "teams",
            "doctor",
        ] {
            assert!(
                markdown.contains(name),
                "clap-markdown output missing command: {name}"
            );
        }
    }

    /// --agent and --team are mutually exclusive.
    #[test]
    fn test_agent_and_team_mutually_exclusive() {
        let result =
            crate::cli::Cli::try_parse_from(["mika", "--agent", "work", "--team", "research"]);
        assert!(result.is_err(), "--agent and --team should conflict");
    }

    /// --team alone should parse successfully.
    #[test]
    fn test_team_flag_parses() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "--team", "research"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(cli.team.as_deref(), Some("research"));
        assert!(cli.agent.is_none());
    }

    /// --agent alone should still work.
    #[test]
    fn test_agent_flag_still_works() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "--agent", "work"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(cli.agent.as_deref(), Some("work"));
        assert!(cli.team.is_none());
    }

    /// --team with a subcommand should parse (validation happens in main, not clap).
    #[test]
    fn test_team_with_subcommand_parses() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "--team", "dev", "chat"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(cli.team.as_deref(), Some("dev"));
    }

    /// clap-markdown output should include the --team flag.
    #[test]
    fn test_clap_markdown_contains_team_flag() {
        use clap::CommandFactory;
        let cmd = crate::cli::Cli::command();
        let markdown = clap_markdown::help_markdown_command(&cmd);
        assert!(
            markdown.contains("--team"),
            "clap-markdown output missing --team flag"
        );
    }

    /// `mika setup --api-key <key>` should parse successfully.
    #[test]
    fn test_setup_accepts_api_key_flag() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "setup", "--api-key", "sk-test"]);
        assert!(cli.is_ok());
    }
}
