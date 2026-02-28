//! MCP server configuration.
//!
//! MCP servers are configured via `~/.mika/mcp.json` (or `{agent_home}/mcp.json`
//! in multi-agent mode). The format matches the Claude Desktop convention:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "filesystem": {
//!       "transport": "stdio",
//!       "command": "npx",
//!       "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"],
//!       "env": {},
//!       "enabled": true
//!     }
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Top-level MCP configuration, loaded from `mcp.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub transport: McpTransport,
    /// Command to run for stdio transport.
    #[serde(default)]
    pub command: Option<String>,
    /// Arguments for the command.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Environment variables to set for the child process.
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    /// URL for HTTP transport.
    #[serde(default)]
    pub url: Option<String>,
    /// Whether this server is enabled (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// MCP transport type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
}

fn default_true() -> bool {
    true
}

impl McpConfig {
    /// Load MCP configuration from `{home_dir}/mcp.json`.
    ///
    /// Returns an empty config if the file does not exist.
    /// Returns an error if the file exists but is malformed.
    pub fn load(home_dir: &Path) -> anyhow::Result<Self> {
        let path = home_dir.join("mcp.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: McpConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// Save MCP configuration to `{home_dir}/mcp.json`.
    pub fn save(&self, home_dir: &Path) -> anyhow::Result<()> {
        let path = home_dir.join("mcp.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Return only enabled servers.
    pub fn enabled_servers(&self) -> impl Iterator<Item = (&str, &McpServerConfig)> {
        self.mcp_servers
            .iter()
            .filter(|(_, cfg)| cfg.enabled)
            .map(|(name, cfg)| (name.as_str(), cfg))
    }
}

impl McpServerConfig {
    /// Validate that the config has the required fields for its transport type.
    pub fn validate(&self, name: &str) -> anyhow::Result<()> {
        match self.transport {
            McpTransport::Stdio => {
                if self.command.as_ref().map_or(true, |c| c.trim().is_empty()) {
                    anyhow::bail!("MCP server '{name}': stdio transport requires 'command'");
                }
            }
            McpTransport::Http => {
                if self.url.as_ref().map_or(true, |u| u.trim().is_empty()) {
                    anyhow::bail!("MCP server '{name}': http transport requires 'url'");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let json = r#"{
            "mcpServers": {
                "filesystem": {
                    "transport": "stdio",
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem"],
                    "enabled": true
                },
                "remote": {
                    "transport": "http",
                    "url": "http://localhost:8000/mcp",
                    "enabled": false
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.mcp_servers.len(), 2);

        let fs = &config.mcp_servers["filesystem"];
        assert_eq!(fs.transport, McpTransport::Stdio);
        assert_eq!(fs.command.as_deref(), Some("npx"));
        assert!(fs.enabled);

        let remote = &config.mcp_servers["remote"];
        assert_eq!(remote.transport, McpTransport::Http);
        assert_eq!(remote.url.as_deref(), Some("http://localhost:8000/mcp"));
        assert!(!remote.enabled);
    }

    #[test]
    fn test_parse_empty_config() {
        let json = "{}";
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_enabled_default_true() {
        let json = r#"{
            "mcpServers": {
                "test": {
                    "transport": "stdio",
                    "command": "echo"
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert!(config.mcp_servers["test"].enabled);
    }

    #[test]
    fn test_enabled_servers_filter() {
        let json = r#"{
            "mcpServers": {
                "a": { "transport": "stdio", "command": "a", "enabled": true },
                "b": { "transport": "stdio", "command": "b", "enabled": false },
                "c": { "transport": "stdio", "command": "c", "enabled": true }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        let enabled: Vec<&str> = config.enabled_servers().map(|(n, _)| n).collect();
        assert_eq!(enabled.len(), 2);
        assert!(enabled.contains(&"a"));
        assert!(enabled.contains(&"c"));
    }

    #[test]
    fn test_validate_stdio_requires_command() {
        let cfg = McpServerConfig {
            transport: McpTransport::Stdio,
            command: None,
            args: None,
            env: None,
            url: None,
            enabled: true,
        };
        assert!(cfg.validate("test").is_err());
    }

    #[test]
    fn test_validate_http_requires_url() {
        let cfg = McpServerConfig {
            transport: McpTransport::Http,
            command: None,
            args: None,
            env: None,
            url: None,
            enabled: true,
        };
        assert!(cfg.validate("test").is_err());
    }

    #[test]
    fn test_validate_valid_stdio() {
        let cfg = McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("npx".to_string()),
            args: Some(vec!["-y".to_string(), "server".to_string()]),
            env: None,
            url: None,
            enabled: true,
        };
        assert!(cfg.validate("test").is_ok());
    }

    #[test]
    fn test_validate_valid_http() {
        let cfg = McpServerConfig {
            transport: McpTransport::Http,
            command: None,
            args: None,
            env: None,
            url: Some("http://localhost:8000/mcp".to_string()),
            enabled: true,
        };
        assert!(cfg.validate("test").is_ok());
    }

    #[test]
    fn test_load_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config = McpConfig::load(tmp.path()).unwrap();
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = McpConfig::default();
        config.mcp_servers.insert(
            "test".to_string(),
            McpServerConfig {
                transport: McpTransport::Stdio,
                command: Some("echo".to_string()),
                args: None,
                env: None,
                url: None,
                enabled: true,
            },
        );
        config.save(tmp.path()).unwrap();

        let loaded = McpConfig::load(tmp.path()).unwrap();
        assert_eq!(loaded.mcp_servers.len(), 1);
        assert!(loaded.mcp_servers.contains_key("test"));
    }
}
