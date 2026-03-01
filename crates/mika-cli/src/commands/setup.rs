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
    println!(
        "\n  \u{2726} Mika initialized at {}\n",
        agent_home.display()
    );

    Ok(())
}
