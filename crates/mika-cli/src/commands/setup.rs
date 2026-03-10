use std::io::Write;

use anyhow::{Context, Result};
use mika_common::home;

pub async fn run(agent_name: &str) -> Result<()> {
    let home_dir = home::resolve_home_dir()?;

    if home::is_initialized(&home_dir) {
        println!(
            "\n  Mika is already initialized at {}\n",
            home_dir.display()
        );
        return Ok(());
    }

    home::bootstrap_fresh_install(&home_dir)?;

    // If a custom agent name was requested, bootstrap it and set as active
    if agent_name != mika_common::agent::DEFAULT_AGENT {
        home::bootstrap_agent(&home_dir, agent_name)
            .with_context(|| format!("failed to initialize agent '{agent_name}'"))?;
        home::write_active_agent(&home_dir, agent_name)?;
    }

    let agent_home = home::resolve_agent_home(&home_dir, agent_name);

    // Prompt for API key if not already set via env var or .env
    if std::env::var("MIKA_ANTHROPIC_API_KEY")
        .ok()
        .filter(|v| !v.is_empty())
        .is_none()
        && mika_common::dotenv::get_env_var(&home_dir, "MIKA_ANTHROPIC_API_KEY").is_none()
    {
        println!("\n  Enter your Anthropic API key (or press Enter to skip):");
        print!("  > ");
        std::io::stdout().flush()?;

        let mut key = String::new();
        std::io::stdin().read_line(&mut key)?;
        let key = key.trim();
        if !key.is_empty() {
            mika_common::dotenv::set_env_var(&home_dir, "MIKA_ANTHROPIC_API_KEY", key)?;
            println!("  Saved to {}", home_dir.join(".env").display());
        }
    }

    println!(
        "\n  \u{2726} Mika initialized at {}\n",
        agent_home.display()
    );

    Ok(())
}
