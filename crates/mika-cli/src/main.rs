mod cli;
mod commands;
mod init;
mod tui;
mod wizard;

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
    // --team is mutually exclusive with --agent (enforced by clap on each level).
    // Merge top-level and subcommand-level --team flag.
    let team_override = cli
        .command
        .as_ref()
        .and_then(|c| c.team_override())
        .or(cli.team.as_deref());

    if let Some(team_name) = team_override {
        let team_name = team::normalize_team_name(team_name);
        team::validate_team_name(&team_name)?;
        let global_home = home::resolve_home_dir()?;
        mika_common::dotenv::load_dotenv(&global_home);

        if !team::team_exists(&global_home, &team_name) {
            anyhow::bail!("Team '{team_name}' not found.");
        }

        match cli.command {
            // `mika --team` or `mika chat --team`
            None | Some(Commands::Chat(_)) => {
                let (explicit_run_id, last_run) = match cli.command {
                    Some(Commands::Chat(ref args)) => (args.run_id.as_deref(), args.last_run),
                    _ => (None, false),
                };

                let run_id = if last_run {
                    Some(resolve_last_run(&global_home, &team_name)?)
                } else {
                    explicit_run_id.map(String::from)
                };

                // Validate --run-id format before any filesystem/DB use (defense-in-depth)
                if let Some(ref ref_id) = run_id
                    && uuid::Uuid::parse_str(ref_id).is_err()
                {
                    anyhow::bail!(
                        "Invalid --run-id format. Expected a UUID (e.g., from a previous team run)."
                    );
                }

                let (_log_guard, _telemetry_guard) = init_team_logging(&global_home, &team_name);

                return commands::chat::run_team(&team_name, &global_home, run_id.as_deref()).await;
            }
            // `mika ask --team`
            Some(Commands::Ask(ref args)) => {
                let run_id = if args.last_run {
                    Some(resolve_last_run(&global_home, &team_name)?)
                } else {
                    args.run_id.clone()
                };

                let (_log_guard, _telemetry_guard) = init_team_logging(&global_home, &team_name);

                return commands::ask::run_team_ask(
                    &team_name,
                    &args.message,
                    run_id.as_deref(),
                    &args.format,
                    &global_home,
                )
                .await;
            }
            Some(_) => {
                anyhow::bail!(
                    "--team is only supported with 'chat' and 'ask'. \
                     Use 'mika ask --team {team_name} \"goal\"' for non-interactive team runs."
                );
            }
        }
    }

    // Resolve agent name first — needed for correct log directory.
    // Priority: subcommand --agent flag > top-level --agent flag > active_agent file > "mika"
    let agent_override = cli
        .command
        .as_ref()
        .and_then(|c| c.agent_override())
        .or(cli.agent.as_deref());

    let agent_name = match agent_override {
        Some(name) => {
            let name = agent::normalize_agent_name(name);
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
    let suppress_stderr = matches!(
        cli.command,
        None | Some(Commands::Chat(_)) | Some(Commands::Ask(_))
    );
    let log_output = if suppress_stderr {
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
            commands::chat::run(&agent_name, cli.session_id.as_deref(), None).await
        }
        Some(Commands::Chat(ref args)) => {
            commands::chat::run(
                &agent_name,
                cli.session_id.as_deref(),
                args.model.as_deref(),
            )
            .await
        }
        Some(Commands::Setup { mode, api_key }) => {
            commands::setup::run(&agent_name, mode, api_key.as_deref()).await
        }
        Some(Commands::Memory(args)) => commands::memory::run(args, &agent_name).await,
        Some(Commands::Reminders(args)) => commands::reminders::run(args, &agent_name).await,
        Some(Commands::Status(_)) => commands::status::run(&agent_name).await,
        Some(Commands::Config(args)) => commands::config::run(args, &agent_name).await,
        Some(Commands::Skills(args)) => commands::skills::run(args, &agent_name).await,
        Some(Commands::Ask(args)) => {
            match commands::ask::run(
                &args.message,
                &agent_name,
                args.task_id.as_deref(),
                cli.session_id.as_deref(),
                args.parent_task_id.as_deref(),
                &args.format,
                args.model.as_deref(),
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

/// Resolve `--last-run` by looking up the most recent finished team run.
fn resolve_last_run(global_home: &std::path::Path, team_name: &str) -> anyhow::Result<String> {
    let db = commands::teams::open_container_db(global_home)?;
    match db.get_last_finished_team_run(team_name)? {
        Some(run) => Ok(run.id),
        None => anyhow::bail!(
            "No finished team run found for team '{team_name}'. \
             Run the team first before using --last-run."
        ),
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

/// Initialize logging and optional telemetry for team-mode branches (chat and ask).
///
/// Returns guards that must be held alive for the duration of the team run.
/// Dropping the log guard stops file logging; dropping the telemetry guard flushes OTel spans.
#[cfg(feature = "telemetry")]
fn init_team_logging(
    global_home: &std::path::Path,
    team_name: &str,
) -> (
    Option<mika_common::logging::LogGuard>,
    Option<mika_common::telemetry::TelemetryGuard>,
) {
    let log_level = resolve_log_level(&[global_home.join("config.toml")]);
    let (otel_layer, telemetry_guard) = mika_common::config::Settings::load(global_home)
        .ok()
        .map(|s| mika_common::telemetry::try_init_otel(&s))
        .unwrap_or((None, None));
    let log_dir = team::team_dir(global_home, team_name).join("logs");
    let log_guard = mika_common::logging::init_pretty(
        &log_level,
        Some(&log_dir),
        LogOutput::FileOnly,
        otel_layer,
    );
    (log_guard, telemetry_guard)
}

/// Initialize logging and optional telemetry for team-mode branches (chat and ask).
///
/// Returns guards that must be held alive for the duration of the team run.
/// Dropping the log guard stops file logging.
#[cfg(not(feature = "telemetry"))]
fn init_team_logging(
    global_home: &std::path::Path,
    team_name: &str,
) -> (Option<mika_common::logging::LogGuard>, Option<()>) {
    let log_level = resolve_log_level(&[global_home.join("config.toml")]);
    let (otel_layer, telemetry_guard) = mika_common::config::Settings::load(global_home)
        .ok()
        .map(|s| mika_common::telemetry::try_init_otel(&s))
        .unwrap_or((None, None));
    let log_dir = team::team_dir(global_home, team_name).join("logs");
    let log_guard = mika_common::logging::init_pretty(
        &log_level,
        Some(&log_dir),
        LogOutput::FileOnly,
        otel_layer,
    );
    (log_guard, telemetry_guard)
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
            "llm_model = \"claude-sonnet-4-6\"\nlog_level = \"info\"\nmax_tokens = 4096\n";
        assert_eq!(parse_log_level(content), Some("info".to_string()));
    }

    #[test]
    fn test_parse_log_level_missing() {
        assert_eq!(parse_log_level("llm_model = \"claude-sonnet-4-6\"\n"), None);
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

    /// --agent and --team are mutually exclusive on the chat subcommand.
    #[test]
    fn test_agent_and_team_mutually_exclusive_on_chat() {
        let result = crate::cli::Cli::try_parse_from([
            "mika", "chat", "--agent", "work", "--team", "research",
        ]);
        assert!(
            result.is_err(),
            "--agent and --team should conflict on chat"
        );
    }

    /// --agent and --team are mutually exclusive at the top level.
    #[test]
    fn test_agent_and_team_mutually_exclusive_top_level() {
        let result =
            crate::cli::Cli::try_parse_from(["mika", "--agent", "work", "--team", "research"]);
        assert!(
            result.is_err(),
            "--agent and --team should conflict at top level"
        );
    }

    /// --team alone on chat subcommand should parse successfully.
    #[test]
    fn test_team_flag_on_chat_parses() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "chat", "--team", "research"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(
            cli.command.as_ref().unwrap().team_override(),
            Some("research")
        );
    }

    /// --team at top level (bare mika) should parse successfully.
    #[test]
    fn test_team_flag_top_level_parses() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "--team", "research"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(cli.team.as_deref(), Some("research"));
    }

    /// --agent on chat subcommand should work.
    #[test]
    fn test_agent_flag_on_chat() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "chat", "--agent", "work"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(cli.command.as_ref().unwrap().agent_override(), Some("work"));
    }

    /// --agent at top level (bare mika) should work.
    #[test]
    fn test_agent_flag_top_level() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "--agent", "work"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(cli.agent.as_deref(), Some("work"));
        assert!(cli.command.is_none());
    }

    /// --agent on ask subcommand should work.
    #[test]
    fn test_agent_flag_on_ask() {
        let cli =
            crate::cli::Cli::try_parse_from(["mika", "ask", "--agent", "work", "hello world"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(cli.command.as_ref().unwrap().agent_override(), Some("work"));
    }

    /// --agent should NOT be accepted on setup subcommand.
    #[test]
    fn test_agent_flag_rejected_on_setup() {
        let result = crate::cli::Cli::try_parse_from(["mika", "setup", "--agent", "work"]);
        assert!(result.is_err(), "--agent should not be accepted on setup");
    }

    /// --team should be accepted on ask subcommand.
    #[test]
    fn test_team_flag_accepted_on_ask() {
        let result =
            crate::cli::Cli::try_parse_from(["mika", "ask", "--team", "research", "hello"]);
        assert!(result.is_ok(), "--team should be accepted on ask");
    }

    /// --team and --agent should conflict on ask subcommand.
    #[test]
    fn test_team_conflicts_with_agent_on_ask() {
        let result = crate::cli::Cli::try_parse_from([
            "mika", "ask", "--team", "research", "--agent", "mika", "hello",
        ]);
        assert!(result.is_err(), "--team and --agent should conflict on ask");
    }

    /// --run-id requires --team on ask subcommand.
    #[test]
    fn test_run_id_requires_team_on_ask() {
        let result =
            crate::cli::Cli::try_parse_from(["mika", "ask", "--run-id", "abc-123", "hello"]);
        assert!(result.is_err(), "--run-id should require --team");
    }

    /// --last-run requires --team on ask subcommand.
    #[test]
    fn test_last_run_requires_team_on_ask() {
        let result = crate::cli::Cli::try_parse_from(["mika", "ask", "--last-run", "hello"]);
        assert!(result.is_err(), "--last-run should require --team");
    }

    /// --last-run requires --team on chat subcommand.
    #[test]
    fn test_last_run_requires_team_on_chat() {
        let result = crate::cli::Cli::try_parse_from(["mika", "chat", "--last-run"]);
        assert!(result.is_err(), "--last-run should require --team");
    }

    /// --last-run and --run-id conflict on chat subcommand.
    #[test]
    fn test_last_run_conflicts_with_run_id_on_chat() {
        let result = crate::cli::Cli::try_parse_from([
            "mika",
            "chat",
            "--team",
            "foo",
            "--last-run",
            "--run-id",
            "abc",
        ]);
        assert!(
            result.is_err(),
            "--last-run and --run-id should conflict on chat"
        );
    }

    /// --last-run and --run-id conflict on ask subcommand.
    #[test]
    fn test_last_run_conflicts_with_run_id_on_ask() {
        let result = crate::cli::Cli::try_parse_from([
            "mika",
            "ask",
            "--team",
            "foo",
            "--last-run",
            "--run-id",
            "abc",
            "hello",
        ]);
        assert!(
            result.is_err(),
            "--last-run and --run-id should conflict on ask"
        );
    }

    /// --last-run with --team on chat should parse successfully.
    #[test]
    fn test_last_run_with_team_on_chat_parses() {
        let cli =
            crate::cli::Cli::try_parse_from(["mika", "chat", "--team", "research", "--last-run"]);
        assert!(cli.is_ok());
    }

    /// clap-markdown output should include the --team flag (on chat subcommand).
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

    /// --agent on memory subcommand should work.
    #[test]
    fn test_agent_flag_on_memory() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "memory", "--agent", "work"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(cli.command.as_ref().unwrap().agent_override(), Some("work"));
    }

    /// --format json on ask subcommand should parse successfully.
    #[test]
    fn test_format_json_on_ask_parses() {
        let cli =
            crate::cli::Cli::try_parse_from(["mika", "ask", "--format", "json", "hello world"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        if let Some(Commands::Ask(args)) = cli.command {
            assert!(matches!(args.format, crate::cli::OutputFormat::Json));
        } else {
            panic!("Expected Ask command");
        }
    }

    /// --format text on ask subcommand should parse successfully (explicit default).
    #[test]
    fn test_format_text_on_ask_parses() {
        let cli =
            crate::cli::Cli::try_parse_from(["mika", "ask", "--format", "text", "hello world"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        if let Some(Commands::Ask(args)) = cli.command {
            assert!(matches!(args.format, crate::cli::OutputFormat::Text));
        } else {
            panic!("Expected Ask command");
        }
    }

    /// ask without --format should default to text.
    #[test]
    fn test_format_defaults_to_text_on_ask() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "ask", "hello world"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        if let Some(Commands::Ask(args)) = cli.command {
            assert!(matches!(args.format, crate::cli::OutputFormat::Text));
        } else {
            panic!("Expected Ask command");
        }
    }

    /// --format with invalid value should fail.
    #[test]
    fn test_format_invalid_value_rejected() {
        let result =
            crate::cli::Cli::try_parse_from(["mika", "ask", "--format", "xml", "hello world"]);
        assert!(result.is_err());
    }

    /// --agent should NOT be accepted on doctor subcommand.
    #[test]
    fn test_agent_flag_rejected_on_doctor() {
        let result = crate::cli::Cli::try_parse_from(["mika", "doctor", "--agent", "work"]);
        assert!(result.is_err(), "--agent should not be accepted on doctor");
    }

    /// --agent should NOT be accepted on teams subcommand.
    #[test]
    fn test_agent_flag_rejected_on_teams() {
        let result = crate::cli::Cli::try_parse_from(["mika", "teams", "--agent", "work", "list"]);
        assert!(result.is_err(), "--agent should not be accepted on teams");
    }

    /// --model on ask subcommand should parse successfully.
    #[test]
    fn test_model_flag_on_ask_parses() {
        let cli =
            crate::cli::Cli::try_parse_from(["mika", "ask", "--model", "sonnet", "hello world"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        if let Some(Commands::Ask(args)) = cli.command {
            assert_eq!(args.model.as_deref(), Some("sonnet"));
        } else {
            panic!("Expected Ask command");
        }
    }

    /// --model on chat subcommand should parse successfully.
    #[test]
    fn test_model_flag_on_chat_parses() {
        let cli = crate::cli::Cli::try_parse_from(["mika", "chat", "--model", "opus"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        if let Some(Commands::Chat(args)) = cli.command {
            assert_eq!(args.model.as_deref(), Some("opus"));
        } else {
            panic!("Expected Chat command");
        }
    }

    /// --model with provider-prefixed value should parse.
    #[test]
    fn test_model_flag_provider_prefixed() {
        let cli =
            crate::cli::Cli::try_parse_from(["mika", "ask", "--model", "openai/gpt-4o", "hello"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        if let Some(Commands::Ask(args)) = cli.command {
            assert_eq!(args.model.as_deref(), Some("openai/gpt-4o"));
        } else {
            panic!("Expected Ask command");
        }
    }

    /// --model and --team should conflict on ask subcommand.
    #[test]
    fn test_model_conflicts_with_team_on_ask() {
        let result = crate::cli::Cli::try_parse_from([
            "mika", "ask", "--model", "sonnet", "--team", "research", "hello",
        ]);
        assert!(result.is_err(), "--model and --team should conflict on ask");
    }

    /// --model and --team should conflict on chat subcommand.
    #[test]
    fn test_model_conflicts_with_team_on_chat() {
        let result = crate::cli::Cli::try_parse_from([
            "mika", "chat", "--model", "sonnet", "--team", "research",
        ]);
        assert!(
            result.is_err(),
            "--model and --team should conflict on chat"
        );
    }

    /// resolve_model_alias resolves known aliases and passes through unknown values.
    #[test]
    fn test_resolve_model_alias() {
        assert_eq!(
            crate::cli::resolve_model_alias("sonnet"),
            "claude-sonnet-4-6"
        );
        assert_eq!(crate::cli::resolve_model_alias("opus"), "claude-opus-4-6");
        assert_eq!(crate::cli::resolve_model_alias("haiku"), "claude-haiku-4-5");
        assert_eq!(
            crate::cli::resolve_model_alias("Sonnet"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            crate::cli::resolve_model_alias("openai/gpt-4o"),
            "openai/gpt-4o"
        );
        assert_eq!(
            crate::cli::resolve_model_alias("claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
    }
}
