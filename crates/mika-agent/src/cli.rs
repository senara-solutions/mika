use anyhow::{Context, Result};
use mika_agent::agent;
use mika_agent::db::Database;
use mika_agent::tools;
use mika_common::claude::ClaudeClient;
use mika_common::config::Settings;
use mika_common::crypto::EncryptionKey;
use mika_common::logging;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    logging::init_pretty("info");

    let settings = Settings::load().context(
        "Failed to load config. Set MIKA_ANTHROPIC_API_KEY and MIKA_ENCRYPTION_KEY env vars.",
    )?;

    let key = EncryptionKey::from_hex(&settings.encryption_key)
        .context("invalid MIKA_ENCRYPTION_KEY (need 64 hex chars)")?;

    let db_path = PathBuf::from(&settings.db_path);
    let db = Database::open(&db_path, key).context("failed to open database")?;

    // Seed core memory if empty
    if db.get_all_core_memory()?.is_empty() {
        db.seed_core_memory()?;
        tracing::info!("seeded core memory for new database");
    }

    let claude = ClaudeClient::new(
        settings.anthropic_api_key.clone(),
        settings.claude_model.clone(),
        settings.claude_max_tokens,
    );

    let tool_registry = tools::default_tools();
    let customer_id = settings.customer_id.as_deref().unwrap_or("cli-user");

    println!("Mika CLI — type a message and press Enter. Type 'quit' to exit.\n");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("You: ");
        stdout.flush()?;

        let mut input = String::new();
        stdin.lock().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }
        if input == "quit" || input == "exit" {
            println!("Goodbye!");
            break;
        }

        match agent::run_agent(
            &db,
            &claude,
            &tool_registry,
            customer_id,
            settings.routing_url.as_deref(),
            input,
            "cli",
        )
        .await
        {
            Ok(response) => {
                println!("\nMika: {response}\n");
            }
            Err(e) => {
                eprintln!("\nError: {e:#}\n");
            }
        }
    }

    Ok(())
}
