mod cli;
mod commands;
mod init;
mod tui;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use mika_common::agent;
use mika_common::home;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing so structured logs are not silently dropped.
    // All CLI modes use pretty (human-readable) output at warn level.
    // TUI commands write to stderr which ratatui's alternate screen handles.
    mika_common::logging::init_pretty("warn");

    // Resolve agent name: --agent flag > active_agent file > "main"
    let agent_name = match cli.agent {
        Some(name) => {
            let name = agent::normalize_agent_name(&name);
            agent::validate_agent_name(&name)?;
            name
        }
        None => init::resolve_active_agent()?,
    };

    match cli.command {
        // Bare `mika` with no subcommand: auto-setup if needed, then chat
        None => {
            let home_dir = home::resolve_home_dir()?;
            if !home::is_initialized(&home_dir) {
                commands::setup::run(&agent_name).await?;
            }
            commands::chat::run(&agent_name).await
        }
        Some(Commands::Chat) => commands::chat::run(&agent_name).await,
        Some(Commands::Setup) => commands::setup::run(&agent_name).await,
        Some(Commands::Memory(args)) => commands::memory::run(args, &agent_name).await,
        Some(Commands::Reminders(args)) => commands::reminders::run(args, &agent_name).await,
        Some(Commands::Status) => commands::status::run(&agent_name).await,
        Some(Commands::Config(args)) => commands::config::run(args, &agent_name).await,
        Some(Commands::Skills { name }) => commands::skills::run(name, &agent_name).await,
        Some(Commands::Ask { message }) => {
            match commands::ask::run(&message, &agent_name).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Agents(args)) => commands::agents::run(args).await,
        Some(Commands::Teams(args)) => commands::teams::run(args).await,
    }
}
