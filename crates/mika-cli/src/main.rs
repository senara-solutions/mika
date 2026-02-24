mod cli;
mod commands;
mod init;
mod tui;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use mika_common::home;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // Bare `mika` with no subcommand: auto-setup if needed, then chat
        None => {
            let home_dir = home::resolve_home_dir()?;
            if !home::is_initialized(&home_dir) {
                commands::setup::run().await?;
            }
            commands::chat::run().await
        }
        Some(Commands::Chat) => commands::chat::run().await,
        Some(Commands::Setup) => commands::setup::run().await,
        Some(Commands::Memory(args)) => commands::memory::run(args).await,
        Some(Commands::Reminders(args)) => commands::reminders::run(args).await,
        Some(Commands::Status) => commands::status::run().await,
        Some(Commands::Config(args)) => commands::config::run(args).await,
    }
}
