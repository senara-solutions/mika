use anyhow::{Context, Result};
use mika_common::home;

pub async fn run() -> Result<()> {
    let home_dir = home::resolve_home_dir()?;

    if home::is_initialized(&home_dir) {
        println!(
            "\n  Mika is already initialized at {}\n",
            home_dir.display()
        );
        return Ok(());
    }

    home::bootstrap(&home_dir)
        .with_context(|| format!("failed to initialize {}", home_dir.display()))?;
    println!(
        "\n  {} Mika initialized at {}\n",
        '\u{2726}',
        home_dir.display()
    );

    Ok(())
}
