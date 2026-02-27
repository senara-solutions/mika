use anyhow::Result;
use std::process::Command;

use crate::cli::{ConfigArgs, ConfigCommand};
use crate::init;

pub async fn run(args: ConfigArgs, agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;

    match args.command {
        None => {
            println!();
            println!("  Mika Configuration");
            println!("  Home:       {}", ctx.home_dir.display());
            println!("  Model:      {}", ctx.settings.claude_model);
            println!("  Max tokens: {}", ctx.settings.claude_max_tokens);
            println!("  Log level:  {}", ctx.settings.log_level);
            println!("  DB path:    {}", ctx.settings.db_path.display());
            let auth_display = match &ctx.settings.anthropic_api_key {
                Some(key) if key.trim_start().starts_with("sk-ant-oat") => {
                    "OAuth token [REDACTED]"
                }
                Some(_) => "API key [REDACTED]",
                None => "[NOT SET]",
            };
            println!("  Auth:       {}", auth_display);
            println!();
        }
        Some(ConfigCommand::Edit) => {
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let parts: Vec<&str> = editor.split_whitespace().collect();
            if parts.is_empty() {
                anyhow::bail!("$EDITOR is empty or whitespace-only");
            }
            let identity_path = ctx.home_dir.join("identity.toml");
            let status = Command::new(parts[0])
                .args(&parts[1..])
                .arg(&identity_path)
                .status()?;
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

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}
