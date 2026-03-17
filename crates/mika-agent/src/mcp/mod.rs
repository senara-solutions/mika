//! MCP (Model Context Protocol) client integration.
//!
//! Connects to external MCP servers at startup, discovers their tools, and
//! dispatches tool calls from the agent loop. MCP tools are namespaced as
//! `mcp__{server_name}__{tool_name}` to prevent collisions with builtins
//! and skill tools.

pub mod config;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use mika_common::claude::ToolDefinition;
use rmcp::model::{CallToolRequestParams, RawContent};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::TokioChildProcess;
use tracing::{info, warn};

use crate::tools::{ImageData, ToolOutput};
use config::{McpConfig, McpServerConfig, McpTransport};

/// Maximum output length for MCP tool results (matches executor::MAX_OUTPUT_LEN).
const MAX_OUTPUT_LEN: usize = 10_000;

/// Prefix for namespaced MCP tool names.
const MCP_PREFIX: &str = "mcp__";
/// Separator between server name and tool name in namespaced names.
const MCP_SEP: &str = "__";

/// A connected MCP server with its discovered tools.
struct McpConnection {
    service: RunningService<RoleClient, ()>,
    tools: Vec<rmcp::model::Tool>,
}

/// Maximum number of images allowed in a single MCP tool result.
const MAX_MCP_IMAGES: usize = 5;
/// Maximum base64 data size for a single MCP image (5 MB).
const MAX_MCP_IMAGE_SIZE: usize = 5 * 1024 * 1024;

/// Manages connections to all configured MCP servers.
///
/// MCP tool calls are dispatched through this manager.
pub struct McpManager {
    connections: HashMap<String, McpConnection>,
    tool_definitions: Vec<ToolDefinition>,
    /// Maps namespaced tool name -> (server_name, original_tool_name).
    tool_routing: HashMap<String, (String, String)>,
}

impl McpManager {
    /// Connect to all enabled MCP servers from config.
    ///
    /// Connections are established in parallel for fast startup.
    /// Failures are logged and skipped -- never blocks startup.
    /// Returns an empty manager if no servers are configured or all fail.
    pub async fn connect_all(config: &McpConfig) -> Self {
        let mut connections = HashMap::new();
        let mut tool_definitions = Vec::new();
        let mut tool_routing = HashMap::new();

        // Spawn all connections in parallel
        let mut join_set = tokio::task::JoinSet::new();
        for (name, server_config) in config.enabled_servers() {
            if let Err(e) = server_config.validate(name) {
                warn!(server = name, error = %e, "skipping MCP server: invalid config");
                continue;
            }
            let name = name.to_string();
            let server_config = server_config.clone();
            join_set.spawn(async move {
                let result = connect_server(&name, &server_config).await;
                (name, result)
            });
        }

        // Collect results
        while let Some(join_result) = join_set.join_next().await {
            let (name, connect_result) = match join_result {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "MCP server connection task panicked");
                    continue;
                }
            };
            match connect_result {
                Ok(conn) => {
                    info!(
                        server = %name,
                        tools = conn.tools.len(),
                        "connected to MCP server"
                    );

                    for tool in &conn.tools {
                        let tool_name_str: &str = &tool.name;
                        let namespaced = format!("{MCP_PREFIX}{name}{MCP_SEP}{tool_name_str}");

                        if tool_routing.contains_key(&namespaced) {
                            warn!(
                                tool = %namespaced,
                                "duplicate MCP tool name, skipping"
                            );
                            continue;
                        }

                        let description = format!(
                            "[MCP: {name}] {}",
                            tool.description.as_deref().unwrap_or("")
                        );
                        let input_schema =
                            serde_json::Value::Object(tool.input_schema.as_ref().clone());

                        tool_definitions.push(ToolDefinition {
                            name: namespaced.clone(),
                            description,
                            input_schema,
                        });
                        tool_routing
                            .insert(namespaced, (name.to_string(), tool_name_str.to_string()));
                    }

                    connections.insert(name, conn);
                }
                Err(e) => {
                    warn!(server = %name, error = %e, "failed to connect to MCP server");
                }
            }
        }

        if !connections.is_empty() {
            info!(
                servers = connections.len(),
                tools = tool_definitions.len(),
                "MCP manager initialized"
            );
        }

        Self {
            connections,
            tool_definitions,
            tool_routing,
        }
    }

    /// Get tool definitions for all connected MCP servers.
    pub fn tool_definitions(&self) -> &[ToolDefinition] {
        &self.tool_definitions
    }

    /// Check if a tool name belongs to an MCP server.
    pub fn is_mcp_tool(&self, name: &str) -> bool {
        self.tool_routing.contains_key(name)
    }

    /// Check if any MCP servers are connected.
    pub fn has_connections(&self) -> bool {
        !self.connections.is_empty()
    }

    /// Execute an MCP tool call by namespaced name.
    ///
    /// Routes to the correct MCP server, strips the namespace prefix,
    /// and converts the result to `ToolOutput`.
    pub async fn call_tool(&self, namespaced_name: &str, input: serde_json::Value) -> ToolOutput {
        let (server_name, tool_name) = match self.tool_routing.get(namespaced_name) {
            Some(route) => route,
            None => {
                return ToolOutput::error(format!("Unknown MCP tool: {namespaced_name}"));
            }
        };

        let conn = match self.connections.get(server_name) {
            Some(c) => c,
            None => {
                return ToolOutput::error(format!("MCP server '{server_name}' is not connected."));
            }
        };

        let arguments = input.as_object().cloned();

        let params = CallToolRequestParams {
            meta: None,
            name: tool_name.clone().into(),
            arguments,
            task: None,
        };

        let result = match conn.service.call_tool(params).await {
            Ok(r) => r,
            Err(e) => {
                warn!(server = %server_name, tool = %tool_name, error = %e, "MCP tool call failed");
                return ToolOutput::error(format!("MCP tool call failed: {e}"));
            }
        };

        convert_mcp_result(&result, tool_name)
    }

    /// Gracefully shut down all MCP server connections.
    pub async fn shutdown(self) {
        for (name, conn) in self.connections {
            if let Err(e) = conn.service.cancel().await {
                warn!(server = %name, error = %e, "error shutting down MCP server");
            } else {
                info!(server = %name, "MCP server disconnected");
            }
        }
    }
}

/// Connect to a single MCP server.
async fn connect_server(name: &str, config: &McpServerConfig) -> Result<McpConnection> {
    match config.transport {
        McpTransport::Stdio => connect_stdio(name, config).await,
        McpTransport::Http => connect_http(name, config).await,
    }
}

/// Connect to an MCP server via stdio (child process).
async fn connect_stdio(name: &str, config: &McpServerConfig) -> Result<McpConnection> {
    let command_str = config.command.as_deref().unwrap_or_default();

    let mut cmd = tokio::process::Command::new(command_str);

    if let Some(args) = &config.args {
        cmd.args(args);
    }

    // Only pass explicitly configured env vars -- do NOT inherit the full process
    // environment. This prevents leaking MIKA_LLM_API_KEY and other secrets
    // to MCP server child processes.
    cmd.env_clear();

    // Inherit essential env vars for process functionality
    for key in &[
        "PATH",
        "HOME",
        "USER",
        "LANG",
        "TERM",
        "TMPDIR",
        "XDG_RUNTIME_DIR",
    ] {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }

    // Add server-specific env vars (reject overrides of essential vars and MIKA_ secrets)
    if let Some(env) = &config.env {
        for (key, value) in env {
            if key.starts_with("MIKA_") {
                warn!(server = name, key = %key, "ignoring MIKA_ env var override in MCP config");
                continue;
            }
            cmd.env(key, value);
        }
    }

    // Kill the child process when the transport is dropped
    cmd.kill_on_drop(true);

    let transport = TokioChildProcess::new(cmd)?;

    let service: RunningService<RoleClient, ()> = tokio::time::timeout(
        Duration::from_secs(30),
        <() as ServiceExt<RoleClient>>::serve((), transport),
    )
    .await
    .map_err(|_| anyhow::anyhow!("MCP server '{name}' handshake timed out (30s)"))?
    .map_err(|e| anyhow::anyhow!("MCP server '{name}' handshake failed: {e}"))?;

    let tools_result = service
        .list_tools(Default::default())
        .await
        .map_err(|e| anyhow::anyhow!("MCP server '{name}' tools/list failed: {e}"))?;

    Ok(McpConnection {
        service,
        tools: tools_result.tools,
    })
}

/// Connect to an MCP server via Streamable HTTP.
async fn connect_http(name: &str, config: &McpServerConfig) -> Result<McpConnection> {
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };

    let url = config.url.as_deref().unwrap_or_default();
    let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url);

    // Apply configured HTTP headers
    if let Some(headers) = &config.headers {
        // Authorization goes through rmcp's built-in auth_header support (case-insensitive)
        if let Some(auth) = headers
            .iter()
            .find_map(|(k, v)| k.eq_ignore_ascii_case("authorization").then_some(v))
        {
            if http::HeaderValue::from_str(auth).is_ok() {
                transport_config = transport_config.auth_header(auth.clone());
            } else {
                warn!(
                    server = name,
                    "invalid Authorization header value, skipping"
                );
            }
        }

        // Other headers use the custom_headers API
        let mut custom = HashMap::new();
        for (key, value) in headers {
            if key.eq_ignore_ascii_case("authorization") {
                continue; // Already handled above
            }
            match (
                http::HeaderName::from_bytes(key.as_bytes()),
                http::HeaderValue::from_str(value),
            ) {
                (Ok(hname), Ok(hval)) => {
                    custom.insert(hname, hval);
                }
                _ => {
                    warn!(server = name, header = %key, "invalid HTTP header, skipping");
                }
            }
        }
        if !custom.is_empty() {
            transport_config = transport_config.custom_headers(custom);
        }
    }

    let transport = StreamableHttpClientTransport::from_config(transport_config);

    let service: RunningService<RoleClient, ()> = tokio::time::timeout(
        Duration::from_secs(30),
        <() as ServiceExt<RoleClient>>::serve((), transport),
    )
    .await
    .map_err(|_| anyhow::anyhow!("MCP server '{name}' HTTP handshake timed out (30s)"))?
    .map_err(|e| anyhow::anyhow!("MCP server '{name}' HTTP handshake failed: {e}"))?;

    let tools_result = service
        .list_tools(Default::default())
        .await
        .map_err(|e| anyhow::anyhow!("MCP server '{name}' tools/list failed: {e}"))?;

    Ok(McpConnection {
        service,
        tools: tools_result.tools,
    })
}

/// Convert an MCP CallToolResult to a ToolOutput.
fn convert_mcp_result(result: &rmcp::model::CallToolResult, tool_name: &str) -> ToolOutput {
    let is_error = result.is_error.unwrap_or(false);
    let mut text = String::new();
    let mut images = Vec::new();

    for content in &result.content {
        match &**content {
            RawContent::Text(t) => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t.text);
            }
            RawContent::Image(img) => {
                if images.len() >= MAX_MCP_IMAGES {
                    warn!(
                        tool = tool_name,
                        "MCP tool returned too many images, truncating"
                    );
                    break;
                }
                if img.data.len() > MAX_MCP_IMAGE_SIZE {
                    warn!(
                        tool = tool_name,
                        size = img.data.len(),
                        "MCP image too large, skipping"
                    );
                    continue;
                }
                images.push(ImageData {
                    media_type: img.mime_type.clone(),
                    data: img.data.clone(),
                });
            }
            _ => {
                warn!(tool = tool_name, "unsupported MCP content type, skipping");
            }
        }
    }

    // Truncate text output (char-boundary-safe)
    if text.len() > MAX_OUTPUT_LEN {
        let mut boundary = MAX_OUTPUT_LEN;
        while boundary > 0 && !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
        text.push_str("\n... (truncated at 10000 chars)");
    }

    if text.is_empty() && images.is_empty() {
        text = if is_error {
            format!("MCP tool '{tool_name}' returned an error with no details.")
        } else {
            format!("MCP tool '{tool_name}' completed with no output.")
        };
    }

    if is_error {
        ToolOutput::error(text)
    } else if images.is_empty() {
        ToolOutput::success(text)
    } else {
        ToolOutput::success_with_images(text, images)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{CallToolResult, Content};

    /// Create an empty manager for testing.
    fn empty_manager() -> McpManager {
        McpManager {
            connections: HashMap::new(),
            tool_definitions: Vec::new(),
            tool_routing: HashMap::new(),
        }
    }

    /// Parse a namespaced MCP tool name into (server_name, tool_name).
    fn parse_mcp_tool_name(namespaced: &str) -> Option<(&str, &str)> {
        let rest = namespaced.strip_prefix(MCP_PREFIX)?;
        let sep_pos = rest.find(MCP_SEP)?;
        let server = &rest[..sep_pos];
        let tool = &rest[sep_pos + MCP_SEP.len()..];
        if server.is_empty() || tool.is_empty() {
            return None;
        }
        Some((server, tool))
    }

    /// Build a namespaced MCP tool name.
    fn mcp_tool_name(server_name: &str, tool_name: &str) -> String {
        format!("{MCP_PREFIX}{server_name}{MCP_SEP}{tool_name}")
    }

    #[test]
    fn test_parse_mcp_tool_name() {
        let (server, tool) = parse_mcp_tool_name("mcp__filesystem__read_file").unwrap();
        assert_eq!(server, "filesystem");
        assert_eq!(tool, "read_file");
    }

    #[test]
    fn test_parse_mcp_tool_name_with_underscores() {
        let (server, tool) = parse_mcp_tool_name("mcp__my_server__my_tool_name").unwrap();
        assert_eq!(server, "my_server");
        assert_eq!(tool, "my_tool_name");
    }

    #[test]
    fn test_parse_mcp_tool_name_invalid() {
        assert!(parse_mcp_tool_name("not_mcp").is_none());
        assert!(parse_mcp_tool_name("mcp__").is_none());
        assert!(parse_mcp_tool_name("mcp____").is_none());
    }

    #[test]
    fn test_mcp_tool_name_roundtrip() {
        let name = mcp_tool_name("github", "create_issue");
        assert_eq!(name, "mcp__github__create_issue");
        let (server, tool) = parse_mcp_tool_name(&name).unwrap();
        assert_eq!(server, "github");
        assert_eq!(tool, "create_issue");
    }

    #[test]
    fn test_is_mcp_tool_empty_manager() {
        let manager = empty_manager();
        assert!(!manager.is_mcp_tool("mcp__test__tool"));
        assert!(!manager.has_connections());
    }

    #[test]
    fn test_convert_mcp_result_text_only() {
        let result = CallToolResult::success(vec![Content::text("Hello from MCP")]);
        let output = convert_mcp_result(&result, "greet");
        assert!(!output.is_error);
        assert_eq!(output.content, "Hello from MCP");
        assert!(output.images.is_empty());
    }

    #[test]
    fn test_convert_mcp_result_error() {
        let result = CallToolResult::error(vec![Content::text("Something went wrong")]);
        let output = convert_mcp_result(&result, "fail");
        assert!(output.is_error);
        assert_eq!(output.content, "Something went wrong");
    }

    #[test]
    fn test_convert_mcp_result_empty() {
        let result = CallToolResult::success(vec![]);
        let output = convert_mcp_result(&result, "empty_tool");
        assert!(!output.is_error);
        assert!(output.content.contains("no output"));
    }

    #[test]
    fn test_convert_mcp_result_truncation() {
        let long_text = "x".repeat(MAX_OUTPUT_LEN + 500);
        let result = CallToolResult::success(vec![Content::text(long_text)]);
        let output = convert_mcp_result(&result, "verbose");
        assert!(output.content.contains("truncated"));
        assert!(output.content.len() < MAX_OUTPUT_LEN + 100);
    }

    #[test]
    fn test_convert_mcp_result_multiple_text_blocks() {
        let result = CallToolResult::success(vec![
            Content::text("Line 1"),
            Content::text("Line 2"),
            Content::text("Line 3"),
        ]);
        let output = convert_mcp_result(&result, "multi");
        assert!(!output.is_error);
        assert_eq!(output.content, "Line 1\nLine 2\nLine 3");
    }

    #[tokio::test]
    async fn test_connect_all_empty_config() {
        let config = McpConfig::default();
        let manager = McpManager::connect_all(&config).await;
        assert!(!manager.has_connections());
        assert!(manager.tool_definitions().is_empty());
    }

    #[tokio::test]
    async fn test_connect_all_disabled_servers_skipped() {
        let mut config = McpConfig::default();
        config.mcp_servers.insert(
            "disabled".to_string(),
            McpServerConfig {
                transport: McpTransport::Stdio,
                command: Some("nonexistent_command_xyz".to_string()),
                args: None,
                env: None,
                url: None,
                headers: None,
                enabled: false,
            },
        );
        let manager = McpManager::connect_all(&config).await;
        assert!(!manager.has_connections());
    }

    #[tokio::test]
    async fn test_connect_all_invalid_config_skipped() {
        let mut config = McpConfig::default();
        // stdio without command -- invalid
        config.mcp_servers.insert(
            "bad".to_string(),
            McpServerConfig {
                transport: McpTransport::Stdio,
                command: None,
                args: None,
                env: None,
                url: None,
                headers: None,
                enabled: true,
            },
        );
        let manager = McpManager::connect_all(&config).await;
        assert!(!manager.has_connections());
    }

    #[tokio::test]
    async fn test_call_tool_unknown() {
        let manager = empty_manager();
        let output = manager
            .call_tool("mcp__test__nonexistent", serde_json::json!({}))
            .await;
        assert!(output.is_error);
        assert!(output.content.contains("Unknown MCP tool"));
    }
}
