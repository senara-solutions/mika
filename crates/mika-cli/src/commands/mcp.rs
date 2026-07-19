use std::collections::HashMap;

use anyhow::Result;
use mika_agent::mcp::config::{McpConfig, McpServerConfig, McpTransport};
use mika_common::home;

use crate::cli::{McpArgs, McpCommand, OutputFormat};
use crate::commands::format_helper::print_structured;

pub async fn run(args: McpArgs, agent_name: &str) -> Result<()> {
    // AC5 one-shot migration: if operator-shell path is unpopulated and a
    // legacy per-agent `{agent_home}/mcp.json` exists, copy it. Idempotent
    // no-op on subsequent invocations. `agent_name` is still resolved so
    // the migration knows where to source from.
    let global_home = home::resolve_home_dir()?;
    home::migrate_to_multi_agent(&global_home)?;
    let agent_home = home::resolve_agent_home(&global_home, agent_name);
    if let Err(e) = McpConfig::migrate_from_agent_home_if_needed(&agent_home) {
        tracing::warn!(error = %e, "mika#1737 AC5 MCP migration failed; hand-copy may be needed");
    }

    match args.command {
        None => list_servers(&OutputFormat::Text),
        Some(McpCommand::List { format }) => list_servers(&format),
        Some(McpCommand::Add {
            name,
            transport,
            command,
            args: cmd_args,
            url,
            headers,
        }) => add_server(&name, &transport, command, cmd_args, url, headers),
        Some(McpCommand::Remove { name }) => remove_server(&name),
        Some(McpCommand::Enable { name }) => toggle_server(&name, true),
        Some(McpCommand::Disable { name }) => toggle_server(&name, false),
    }
}

fn list_servers(format: &OutputFormat) -> Result<()> {
    let config = McpConfig::load_operator_shell()?;
    let (path, _source) = mika_common::mcp_config_path::resolve_operator_mcp_config_path();

    if config.mcp_servers.is_empty() {
        println!("\n  No MCP servers configured.\n");
        println!("  Add one with: mika mcp add <name> --transport stdio --command <cmd>");
        println!("  Config file:  {}", path.display());
        return Ok(());
    }

    // Build a display-safe representation (header values redacted)
    let mut display_servers: Vec<serde_json::Value> = Vec::new();
    let mut names: Vec<_> = config.mcp_servers.keys().collect();
    names.sort();
    for name in &names {
        let cfg = &config.mcp_servers[*name];
        let transport_str = match cfg.transport {
            McpTransport::Stdio => "stdio",
            McpTransport::Http => "http",
        };
        let header_keys: Option<Vec<String>> = cfg.headers.as_ref().map(|h| {
            let mut keys: Vec<_> = h.keys().cloned().collect();
            keys.sort();
            keys
        });
        display_servers.push(serde_json::json!({
            "name": name,
            "transport": transport_str,
            "command": cfg.command,
            "url": cfg.url,
            "enabled": cfg.enabled,
            "header_keys": header_keys,
        }));
    }

    if print_structured(format, &display_servers)? {
        return Ok(());
    }

    println!("\n  MCP Servers ({}):", config.mcp_servers.len());
    for name in &names {
        let cfg = &config.mcp_servers[*name];
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
        // Show header keys (values redacted) for HTTP servers
        if cfg.transport == McpTransport::Http
            && let Some(headers) = &cfg.headers
            && !headers.is_empty()
        {
            let mut keys: Vec<_> = headers.keys().cloned().collect();
            keys.sort();
            println!("      headers: {}", keys.join(", "));
        }
    }
    println!();
    Ok(())
}

fn add_server(
    name: &str,
    transport: &str,
    command: Option<String>,
    args: Option<Vec<String>>,
    url: Option<String>,
    raw_headers: Option<Vec<String>>,
) -> Result<()> {
    let mut config = McpConfig::load_operator_shell()?;

    if config.mcp_servers.contains_key(name) {
        anyhow::bail!(
            "MCP server '{name}' already exists. Remove it first with: mika mcp remove {name}"
        );
    }

    let transport_type = match transport {
        "stdio" => {
            if command.is_none() {
                anyhow::bail!("--command is required for stdio transport");
            }
            if raw_headers.as_ref().is_some_and(|h| !h.is_empty()) {
                anyhow::bail!("--header is only supported for http transport");
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

    let headers = parse_headers(raw_headers)?;

    let server_config = McpServerConfig {
        transport: transport_type,
        command,
        args,
        env: None,
        url,
        headers,
        enabled: true,
    };

    server_config.validate(name)?;
    config.mcp_servers.insert(name.to_string(), server_config);
    config.save_operator_shell()?;

    println!("Added MCP server '{name}'. Restart Mika to connect.");
    Ok(())
}

fn toggle_server(name: &str, enabled: bool) -> Result<()> {
    let mut config = McpConfig::load_operator_shell()?;

    let server = config
        .mcp_servers
        .get_mut(name)
        .ok_or_else(|| anyhow::anyhow!("MCP server '{name}' not found."))?;

    if server.enabled == enabled {
        let state = if enabled { "enabled" } else { "disabled" };
        println!("MCP server '{name}' is already {state}.");
        return Ok(());
    }

    server.enabled = enabled;
    config.save_operator_shell()?;

    let action = if enabled { "Enabled" } else { "Disabled" };
    println!("{action} MCP server '{name}'. Restart Mika to apply.");
    Ok(())
}

fn parse_headers(raw: Option<Vec<String>>) -> Result<Option<HashMap<String, String>>> {
    let raw = match raw {
        Some(h) if !h.is_empty() => h,
        _ => return Ok(None),
    };

    let mut map = HashMap::new();
    for entry in &raw {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Invalid header format: '{entry}'. Use KEY=VALUE."))?;
        if key.is_empty() {
            anyhow::bail!("Empty header name in: '{entry}'");
        }
        map.insert(key.to_string(), value.to_string());
    }
    Ok(Some(map))
}

fn remove_server(name: &str) -> Result<()> {
    let mut config = McpConfig::load_operator_shell()?;

    if config.mcp_servers.remove(name).is_none() {
        anyhow::bail!("MCP server '{name}' not found.");
    }

    config.save_operator_shell()?;
    println!("Removed MCP server '{name}'.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_headers_normal_key_value() {
        let input = Some(vec!["Content-Type=application/json".to_string()]);
        let result = parse_headers(input).unwrap();
        let map = result.expect("should return Some");
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("Content-Type").unwrap(), "application/json");
    }

    #[test]
    fn parse_headers_value_containing_equals() {
        let input = Some(vec!["Authorization=Bearer abc==".to_string()]);
        let result = parse_headers(input).unwrap();
        let map = result.expect("should return Some");
        assert_eq!(map.get("Authorization").unwrap(), "Bearer abc==");
    }

    #[test]
    fn parse_headers_missing_equals_returns_error() {
        let input = Some(vec!["InvalidHeader".to_string()]);
        let result = parse_headers(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Invalid header format"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_headers_empty_key_returns_error() {
        let input = Some(vec!["=some-value".to_string()]);
        let result = parse_headers(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Empty header name"), "unexpected error: {err}");
    }

    #[test]
    fn parse_headers_none_returns_ok_none() {
        let result = parse_headers(None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_headers_empty_vec_returns_ok_none() {
        let result = parse_headers(Some(vec![])).unwrap();
        assert!(result.is_none());
    }
}
