use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Local;

use crate::cli::OutputFormat;

/// Print the resolved log file paths for the given agent.
///
/// Two sinks exist:
/// - **Server log:** `MIKA_SERVER_LOG_FILE` (or `/var/log/mika/server.log` fallback) — contains
///   all runtime events from the long-running mika-server daemon, filterable by `agent_id`.
/// - **Per-agent CLI log:** `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` — contains events
///   from discrete CLI invocations (`mika ask`, `mika chat`, etc.) for that agent only.
pub fn run(agent_name: &str, agent_home: &Path, format: &OutputFormat) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();

    // Resolve server log path: env var > fallback
    let server_log = std::env::var("MIKA_SERVER_LOG_FILE")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/log/mika/server.log"));

    // Per-agent CLI log path (daily rotation)
    let cli_log = agent_home.join("logs").join(format!("mika.log.{today}"));

    let server_log_exists = server_log.exists();
    let cli_log_exists = cli_log.exists();

    let server_log_size = if server_log_exists {
        std::fs::metadata(&server_log).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    let cli_log_size = if cli_log_exists {
        std::fs::metadata(&cli_log).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    match format {
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "agent": agent_name,
                "server_log": {
                    "path": server_log.to_string_lossy(),
                    "exists": server_log_exists,
                    "size_bytes": server_log_size,
                    "filter_command": format!("jq 'select(.agent_id == \"{agent_name}\")' {}", server_log.display()),
                },
                "cli_log": {
                    "path": cli_log.to_string_lossy(),
                    "exists": cli_log_exists,
                    "size_bytes": cli_log_size,
                    "date": today,
                },
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        OutputFormat::Text => {
            println!();
            println!("  \u{2726} Log Sinks for agent: {agent_name}");
            println!();
            println!("  Server log (mika-server runtime events):");
            println!("    Path:   {}", server_log.display());
            println!(
                "    Status: {}",
                if server_log_exists {
                    format!("exists ({})", format_size(server_log_size))
                } else {
                    "not found".to_string()
                }
            );
            println!(
                "    Filter: jq 'select(.agent_id == \"{agent_name}\")' {}",
                server_log.display()
            );
            println!();
            println!("  Per-agent CLI log (mika ask/chat invocations):");
            println!("    Path:   {}", cli_log.display());
            println!(
                "    Status: {}",
                if cli_log_exists {
                    format!("exists ({})", format_size(cli_log_size))
                } else {
                    "not found (no CLI invocations today)".to_string()
                }
            );
            println!();
            println!("  Tip: For server-mode runtime events (skill execution, task engine,");
            println!("  callback lifecycle), always use the server log filtered by agent_id.");
            println!("  The per-agent CLI log only contains events from discrete CLI calls.");
            println!();
        }
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes > 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes > 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes > 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}
