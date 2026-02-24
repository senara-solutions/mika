use anyhow::{Context, Result};
use mika_agent::agent::{self, AgentParams};
use mika_agent::db::Database;
use mika_agent::tools;
use mika_common::claude::ClaudeClient;
use mika_common::config::Settings;
use mika_common::home;
use mika_common::logging;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use uuid::Uuid;

const CORE_MEMORY_DEFAULTS: &[(&str, &str)] = &[
    ("persona", "Mika -- personal AI executive assistant."),
    (
        "current_priorities",
        "Get to know the user and understand their needs.",
    ),
    ("key_people", "No one tracked yet."),
];

#[tokio::main]
async fn main() -> Result<()> {
    logging::init_pretty("info");

    // Resolve and bootstrap home directory
    let home_dir = home::resolve_home_dir()?;
    let is_first_run = !home::is_initialized(&home_dir);

    if is_first_run {
        home::bootstrap(&home_dir)
            .with_context(|| format!("failed to initialize {}", home_dir.display()))?;
        println!("\n  ✦ Mika initialized at {}\n", home_dir.display());
    }

    let settings = Settings::load(&home_dir)
        .context("Failed to load config. Set MIKA_ANTHROPIC_API_KEY env var.")?;

    let db_path = PathBuf::from(&settings.db_path);

    // Ensure parent directory exists for the database
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    let db = Database::open(&db_path).context("failed to open database")?;

    // Seed core memory if empty (first run or fresh database)
    if db.get_all_core_memory()?.is_empty() {
        // Load user.md content to seed user_summary if available
        let user_md_path = home_dir.join("user.md");
        let user_md_content = std::fs::read_to_string(&user_md_path).ok();
        let user_md_ref = user_md_content.as_deref().filter(|s| {
            // Only use user.md if the user actually edited it (not the default template)
            !s.starts_with("# Tell Mika about yourself")
        });
        db.seed_core_memory(user_md_ref)?;
        tracing::info!("seeded core memory for new database");
    }

    let claude = ClaudeClient::new(
        settings.anthropic_api_key.clone(),
        settings.claude_model.clone(),
        settings.claude_max_tokens,
    );

    let tool_registry = tools::default_tools();

    // Generate a session ID for this CLI session (used for audit logging)
    let session_id = Uuid::new_v4().to_string();
    tracing::info!(session_id = %session_id, "starting CLI session");

    // Detect onboarding: user_summary still equals the seed value
    let is_onboarding = db
        .get_core_memory("user_summary")?
        .map(|e| e.value == "New user. No information yet.")
        .unwrap_or(true);

    if is_onboarding {
        tracing::info!("onboarding mode: first conversation with user");
    }

    println!("Mika CLI — type a message and press Enter. Type /help for commands.\n");

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

        // Handle slash commands
        if input.starts_with('/') {
            handle_slash_command(input, &db, &home_dir)?;
            continue;
        }

        match agent::run_agent(&AgentParams {
            db: &db,
            claude: &claude,
            tools: &tool_registry,
            user_message: input,
            channel_type: "cli",
            session_id: &session_id,
            home_dir: &home_dir,
            is_onboarding,
        })
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

fn handle_slash_command(input: &str, db: &Database, home_dir: &std::path::Path) -> Result<()> {
    match input {
        "/help" => {
            println!("\nAvailable commands:");
            println!("  /memory          — Show all core memory blocks");
            println!("  /reset <block>   — Reset a core memory block to default");
            println!("  /help            — Show this help message");
            println!("  quit / exit      — Exit the CLI\n");
        }
        "/memory" => {
            let entries = db.get_all_core_memory()?;
            if entries.is_empty() {
                println!("\nNo core memory entries.\n");
            } else {
                println!("\n--- Core Memory ---");
                for entry in &entries {
                    println!(
                        "\n[{}] (~{} tokens)\n{}",
                        entry.key, entry.token_count, entry.value
                    );
                }
                println!("\n-------------------\n");
            }
        }
        s if s.starts_with("/reset ") => {
            let block = s.strip_prefix("/reset ").unwrap().trim();
            let allowed = [
                "persona",
                "user_summary",
                "current_priorities",
                "key_people",
            ];

            if !allowed.contains(&block) {
                println!(
                    "\nInvalid block '{block}'. Allowed: {}\n",
                    allowed.join(", ")
                );
            } else {
                // Determine reset value
                let reset_value = if block == "user_summary" {
                    // Re-seed from user.md if available
                    let user_md_path = home_dir.join("user.md");
                    let content = std::fs::read_to_string(&user_md_path).ok();
                    let from_file = content
                        .as_deref()
                        .filter(|s| !s.starts_with("# Tell Mika about yourself"));
                    from_file
                        .unwrap_or("New user. No information yet.")
                        .to_string()
                } else {
                    CORE_MEMORY_DEFAULTS
                        .iter()
                        .find(|(k, _)| *k == block)
                        .map(|(_, v)| v.to_string())
                        .unwrap_or_default()
                };

                db.set_core_memory(block, &reset_value)?;
                println!("\nReset [{block}] to default.\n");
            }
        }
        _ => {
            println!("\nUnknown command: {input}. Type /help for available commands.\n");
        }
    }
    Ok(())
}
