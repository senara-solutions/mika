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
#[derive(Clone, Serialize, Deserialize)]
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
    /// HTTP headers for Streamable HTTP transport (e.g. Authorization, API keys).
    /// Ignored for stdio transport.
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    /// Whether this server is enabled (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl std::fmt::Debug for McpServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("McpServerConfig");
        s.field("transport", &self.transport);
        s.field("command", &self.command);
        s.field("args", &self.args);
        // Redact env values (may contain secrets like GITHUB_TOKEN)
        if let Some(env) = &self.env {
            let redacted: HashMap<&String, &str> = env.keys().map(|k| (k, "[REDACTED]")).collect();
            s.field("env", &redacted);
        } else {
            s.field("env", &self.env);
        }
        s.field("url", &self.url);
        // Redact header values (may contain bearer tokens / API keys)
        if let Some(headers) = &self.headers {
            let redacted: HashMap<&String, &str> =
                headers.keys().map(|k| (k, "[REDACTED]")).collect();
            s.field("headers", &redacted);
        } else {
            s.field("headers", &self.headers);
        }
        s.field("enabled", &self.enabled);
        s.finish()
    }
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
    ///
    /// Sets file permissions to `0600` on Unix (may contain secrets in headers/env).
    pub fn save(&self, home_dir: &Path) -> anyhow::Result<()> {
        let path = home_dir.join("mcp.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Return only enabled servers.
    pub fn enabled_servers(&self) -> impl Iterator<Item = (&str, &McpServerConfig)> {
        self.mcp_servers
            .iter()
            .filter(|(_, cfg)| cfg.enabled)
            .map(|(name, cfg)| (name.as_str(), cfg))
    }

    /// Load MCP configuration from the operator-shell path resolved by
    /// [`mika_common::mcp_config_path::resolve_operator_mcp_config_path`]
    /// (mika#1737 AC3, AC4). Returns an empty config if the file does
    /// not exist. Returns an error only when the file exists but is
    /// malformed. The resolved source tier is logged so operators can
    /// verify which resolution rung fired.
    pub fn load_operator_shell() -> anyhow::Result<Self> {
        let (path, source) = mika_common::mcp_config_path::resolve_operator_mcp_config_path();
        if matches!(
            source,
            mika_common::mcp_config_path::McpConfigPathSource::CwdFallback
        ) {
            tracing::warn!(
                path = %path.display(),
                "MCP config path fell back to CWD; set MIKA_MCP_CONFIG, XDG_CONFIG_HOME, or HOME"
            );
        } else {
            tracing::debug!(
                path = %path.display(),
                ?source,
                "resolved operator-shell MCP config path"
            );
        }
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: McpConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// Save MCP configuration to the operator-shell path (mika#1737
    /// AC1). Creates parent directories as needed; sets `0600` on Unix
    /// because headers/env may contain secrets.
    pub fn save_operator_shell(&self) -> anyhow::Result<()> {
        let (path, _source) = mika_common::mcp_config_path::resolve_operator_mcp_config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// One-shot AC5 migration: copy an existing per-agent
    /// `{agent_home}/mcp.json` to the operator-shell path if the
    /// operator-shell path does not yet exist. Idempotent — subsequent
    /// invocations skip the copy once the operator-shell path exists.
    ///
    /// Returns `Ok(true)` if a copy happened, `Ok(false)` if not, or
    /// `Err` on IO failure. IO errors are non-fatal to the caller (the
    /// operator can hand-copy) so callers should log-and-continue.
    ///
    /// Multi-agent semantics: this migration is deliberately per-invocation
    /// / per-agent. If several agents each have a distinct `mcp.json`, the
    /// FIRST invocation across all agents wins and populates the shared
    /// operator-shell config. Later invocations see the operator-shell
    /// path already exists and become no-ops. Operators with divergent
    /// per-agent configs must hand-merge — the ratified disposition
    /// explicitly makes MCP config operator-shell scoped, so divergent
    /// per-agent state was already an anti-pattern.
    pub fn migrate_from_agent_home_if_needed(agent_home: &Path) -> anyhow::Result<bool> {
        let (target, _source) = mika_common::mcp_config_path::resolve_operator_mcp_config_path();
        if target.exists() {
            return Ok(false);
        }
        let source_path = mika_common::mcp_config_path::legacy_per_agent_mcp_path(agent_home);
        if !source_path.exists() {
            return Ok(false);
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Copy contents (do NOT symlink) so a subsequent delete of the
        // per-agent file does not lose the config.
        let content = std::fs::read_to_string(&source_path)?;
        std::fs::write(&target, &content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))?;
        }
        tracing::info!(
            from = %source_path.display(),
            to = %target.display(),
            "migrated per-agent MCP config to operator-shell path (mika#1737 AC5)"
        );
        Ok(true)
    }
}

impl McpServerConfig {
    /// Validate that the config has the required fields for its transport type.
    pub fn validate(&self, name: &str) -> anyhow::Result<()> {
        // Server names must be lowercase alphanumeric with single hyphens/underscores.
        // Double underscores (__) are the namespacing separator and must not appear in names.
        if name.is_empty()
            || name.contains("__")
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            anyhow::bail!(
                "MCP server name '{name}' is invalid: use lowercase alphanumeric, hyphens, \
                 or single underscores (no '__')"
            );
        }

        match self.transport {
            McpTransport::Stdio => {
                if self.command.as_ref().is_none_or(|c| c.trim().is_empty()) {
                    anyhow::bail!("MCP server '{name}': stdio transport requires 'command'");
                }
                if self.headers.as_ref().is_some_and(|h| !h.is_empty()) {
                    tracing::warn!(
                        server = name,
                        "MCP server '{name}': headers are ignored for stdio transport"
                    );
                }
            }
            McpTransport::Http => {
                let url = match self.url.as_deref() {
                    Some(u) if !u.trim().is_empty() => u.trim(),
                    _ => {
                        anyhow::bail!("MCP server '{name}': http transport requires 'url'");
                    }
                };
                // Validate URL scheme
                match url.split_once("://") {
                    Some(("http" | "https", _)) => {}
                    _ => {
                        anyhow::bail!("MCP server '{name}': url must use http or https scheme");
                    }
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
            headers: None,
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
            headers: None,
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
            headers: None,
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
            headers: None,
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
                headers: None,
                enabled: true,
            },
        );
        config.save(tmp.path()).unwrap();

        let loaded = McpConfig::load(tmp.path()).unwrap();
        assert_eq!(loaded.mcp_servers.len(), 1);
        assert!(loaded.mcp_servers.contains_key("test"));
    }

    #[test]
    fn test_validate_rejects_double_underscore_name() {
        let cfg = McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("echo".to_string()),
            args: None,
            env: None,
            url: None,
            headers: None,
            enabled: true,
        };
        assert!(cfg.validate("my__server").is_err());
    }

    #[test]
    fn test_validate_rejects_uppercase_name() {
        let cfg = McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("echo".to_string()),
            args: None,
            env: None,
            url: None,
            headers: None,
            enabled: true,
        };
        assert!(cfg.validate("MyServer").is_err());
    }

    #[test]
    fn test_validate_accepts_hyphen_underscore_name() {
        let cfg = McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("echo".to_string()),
            args: None,
            env: None,
            url: None,
            headers: None,
            enabled: true,
        };
        assert!(cfg.validate("my-server_1").is_ok());
    }

    #[test]
    fn test_validate_http_rejects_ftp_scheme() {
        let cfg = McpServerConfig {
            transport: McpTransport::Http,
            command: None,
            args: None,
            env: None,
            url: Some("ftp://evil.com/mcp".to_string()),
            headers: None,
            enabled: true,
        };
        assert!(cfg.validate("test").is_err());
    }

    #[test]
    fn test_validate_http_accepts_https() {
        let cfg = McpServerConfig {
            transport: McpTransport::Http,
            command: None,
            args: None,
            env: None,
            url: Some("https://mcp.example.com/v1".to_string()),
            headers: None,
            enabled: true,
        };
        assert!(cfg.validate("test").is_ok());
    }

    #[test]
    fn test_parse_config_with_headers() {
        let json = r#"{
            "mcpServers": {
                "remote": {
                    "transport": "http",
                    "url": "https://mcp.example.com/v1",
                    "headers": {
                        "Authorization": "Bearer sk-test-123",
                        "X-API-Key": "abc"
                    }
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        let remote = &config.mcp_servers["remote"];
        let headers = remote.headers.as_ref().unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers["Authorization"], "Bearer sk-test-123");
        assert_eq!(headers["X-API-Key"], "abc");
    }

    #[test]
    fn test_parse_config_without_headers_backwards_compatible() {
        let json = r#"{
            "mcpServers": {
                "old": {
                    "transport": "http",
                    "url": "http://localhost:8000/mcp"
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert!(config.mcp_servers["old"].headers.is_none());
    }

    #[test]
    fn test_parse_config_empty_headers() {
        let json = r#"{
            "mcpServers": {
                "test": {
                    "transport": "http",
                    "url": "http://localhost:8000/mcp",
                    "headers": {}
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        let headers = config.mcp_servers["test"].headers.as_ref().unwrap();
        assert!(headers.is_empty());
    }

    #[test]
    fn test_debug_redacts_header_and_env_values() {
        let cfg = McpServerConfig {
            transport: McpTransport::Http,
            command: None,
            args: None,
            env: Some(HashMap::from([(
                "GITHUB_TOKEN".to_string(),
                "ghp_secret123".to_string(),
            )])),
            url: Some("http://localhost/mcp".to_string()),
            headers: Some(HashMap::from([(
                "Authorization".to_string(),
                "Bearer secret-token".to_string(),
            )])),
            enabled: true,
        };
        let debug_output = format!("{:?}", cfg);
        assert!(debug_output.contains("[REDACTED]"));
        // Header values redacted
        assert!(!debug_output.contains("secret-token"));
        assert!(debug_output.contains("Authorization"));
        // Env values redacted
        assert!(!debug_output.contains("ghp_secret123"));
        assert!(debug_output.contains("GITHUB_TOKEN"));
    }

    #[test]
    fn test_headers_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = McpConfig::default();
        config.mcp_servers.insert(
            "api".to_string(),
            McpServerConfig {
                transport: McpTransport::Http,
                command: None,
                args: None,
                env: None,
                url: Some("https://api.example.com/mcp".to_string()),
                headers: Some(HashMap::from([(
                    "Authorization".to_string(),
                    "Bearer tok".to_string(),
                )])),
                enabled: true,
            },
        );
        config.save(tmp.path()).unwrap();

        let loaded = McpConfig::load(tmp.path()).unwrap();
        let headers = loaded.mcp_servers["api"].headers.as_ref().unwrap();
        assert_eq!(headers["Authorization"], "Bearer tok");
    }
}
