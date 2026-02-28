use anyhow::Result;
use mika_agent::mcp::config::{McpConfig, McpServerConfig, McpTransport};
use mika_common::home;

use crate::cli::{McpArgs, McpCommand};

pub async fn run(args: McpArgs, agent_name: &str) -> Result<()> {
    let global_home = home::resolve_home_dir()?;
    home::migrate_to_multi_agent(&global_home)?;
    let agent_home = home::resolve_agent_home(&global_home, agent_name);

    match args.command {
        None | Some(McpCommand::List) => list_servers(&agent_home),
        Some(McpCommand::Add {
            name,
            transport,
            command,
            args: cmd_args,
            url,
        }) => add_server(&agent_home, &name, &transport, command, cmd_args, url),
        Some(McpCommand::Remove { name }) => remove_server(&agent_home, &name),
    }
}

fn list_servers(agent_home: &std::path::Path) -> Result<()> {
    let config = McpConfig::load(agent_home)?;

    if config.mcp_servers.is_empty() {
        println!("\n  No MCP servers configured.\n");
        println!("  Add one with: mika mcp add <name> --transport stdio --command <cmd>");
        println!("  Config file:  {}/mcp.json", agent_home.display());
        return Ok(());
    }

    println!("\n  MCP Servers ({}):", config.mcp_servers.len());
    let mut names: Vec<_> = config.mcp_servers.keys().collect();
    names.sort();
    for name in names {
        let cfg = &config.mcp_servers[name];
        let transport = match cfg.transport {
            McpTransport::Stdio => {
                format!("stdio ({})", cfg.command.as_deref().unwrap_or("?"))
            }
            McpTransport::Http => {
                format!("http ({})", cfg.url.as_deref().unwrap_or("?"))
            }
        };
        let status = if cfg.enabled { "enabled" } else { "disabled" };
        println!("    {name}: {transport} [{status}]");
    }
    println!();
    Ok(())
}

fn add_server(
    agent_home: &std::path::Path,
    name: &str,
    transport: &str,
    command: Option<String>,
    args: Option<Vec<String>>,
    url: Option<String>,
) -> Result<()> {
    let mut config = McpConfig::load(agent_home)?;

    if config.mcp_servers.contains_key(name) {
        anyhow::bail!("MCP server '{name}' already exists. Remove it first with: mika mcp remove {name}");
    }

    let transport_type = match transport {
        "stdio" => {
            if command.is_none() {
                anyhow::bail!("--command is required for stdio transport");
            }
            McpTransport::Stdio
        }
        "http" => {
            if url.is_none() {
                anyhow::bail!("--url is required for http transport");
            }
            McpTransport::Http
        }
        other => anyhow::bail!("Unknown transport type: {other}. Use 'stdio' or 'http'."),
    };

    let server_config = McpServerConfig {
        transport: transport_type,
        command,
        args,
        env: None,
        url,
        enabled: true,
    };

    server_config.validate(name)?;
    config.mcp_servers.insert(name.to_string(), server_config);
    config.save(agent_home)?;

    println!("Added MCP server '{name}'. Restart Mika to connect.");
    Ok(())
}

fn remove_server(agent_home: &std::path::Path, name: &str) -> Result<()> {
    let mut config = McpConfig::load(agent_home)?;

    if config.mcp_servers.remove(name).is_none() {
        anyhow::bail!("MCP server '{name}' not found.");
    }

    config.save(agent_home)?;
    println!("Removed MCP server '{name}'.");
    Ok(())
}
