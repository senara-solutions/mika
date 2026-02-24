use anyhow::Result;
use std::process::Command;

use crate::cli::{ConfigArgs, ConfigCommand};
use crate::init;

pub async fn run(args: ConfigArgs) -> Result<()> {
    let ctx = init::init_db_only()?;

    match args.command {
        None => {
            println!();
            println!("  Mika Configuration");
            println!("  Home:       {}", ctx.home_dir.display());
            println!("  Model:      {}", ctx.settings.claude_model);
            println!("  Max tokens: {}", ctx.settings.claude_max_tokens);
            println!("  Log level:  {}", ctx.settings.log_level);
            println!("  DB path:    {}", ctx.settings.db_path.display());
            println!("  API key:    [REDACTED]");
            println!();
        }
        Some(ConfigCommand::Edit) => {
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let identity_path = ctx.home_dir.join("identity.toml");
            let status = Command::new(&editor).arg(&identity_path).status()?;
            if !status.success() {
                anyhow::bail!("{editor} exited with {status}");
            }
        }
        Some(ConfigCommand::Soul) => {
            let soul_path = ctx.home_dir.join("soul.md");
            match std::fs::read_to_string(&soul_path) {
                Ok(content) => print!("{content}"),
                Err(_) => println!("No soul.md found at {}", soul_path.display()),
            }
        }
    }

    ctx.async_db.shutdown();
    Ok(())
}
