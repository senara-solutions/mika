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
use rmcp::model::{CallToolRequestParams, ContentBlock};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::TokioChildProcess;
use tracing::{info, warn};

use crate::skills::manifest::DataGrade;
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
    /// Grade par nom d'outil namespacé, dérivé du `data_grade` de son serveur
    /// (mika#1959). Consommé par la garde L4 de `tool_execution::dispatch` via
    /// l'ensemencement de `skill_data_grades` dans `agent_loop` — pas par
    /// `call_tool`, qui ne refuse rien.
    tool_data_grades: HashMap<String, DataGrade>,
}

/// Joint le `data_grade` de chaque serveur à chacun de ses outils namespacés.
///
/// Libre et pure pour être testable : `connect_all` ouvre de vraies connexions,
/// aucun test unitaire ne l'atteint (décision D4 du plan mika#1959). Un outil
/// dont le serveur a disparu de la config retombe sur le défaut de l'enum —
/// cas inatteignable depuis `connect_all` (le routage est bâti depuis cette
/// même config), couvert parce que la signature le permet.
pub(crate) fn mcp_tool_data_grades(
    config: &McpConfig,
    tool_routing: &HashMap<String, (String, String)>,
) -> HashMap<String, DataGrade> {
    tool_routing
        .iter()
        .map(|(namespaced, (server, _tool))| {
            let grade = config
                .mcp_servers
                .get(server)
                .map(|cfg| cfg.data_grade)
                .unwrap_or_default();
            (namespaced.clone(), grade)
        })
        .collect()
}

/// Le segment serveur d'un nom d'outil namespacé, pour la journalisation.
///
/// Dérivé du **nom**, jamais d'un second parcours de `config.mcp_servers` : le
/// nom porte déjà le serveur (`mcp__{serveur}__{outil}`), et deux lectures du
/// même fait peuvent divergir. `split_once` plutôt qu'un découpage d'octets —
/// `scripts/check-byte-slices.sh` refuse le second, et un nom de serveur peut
/// porter des caractères multi-octets.
///
/// `pub(crate)` depuis mika#2814 vol 4 : `agent_loop` en a besoin pour le champ
/// `server` du `WARN` de désarmement, et pour la même raison qu'ici — dériver le
/// serveur du nom plutôt que d'ouvrir un second parcours de la config.
pub(crate) fn serveur_du_nom_namespace(namespaced: &str) -> Option<&str> {
    let (serveur, _outil) = namespaced.strip_prefix(MCP_PREFIX)?.split_once(MCP_SEP)?;
    (!serveur.is_empty()).then_some(serveur)
}

/// Retire du lot **publié au modèle** tout outil dont le serveur déclare
/// `data_grade = "testimony"` (mika#2814) — l'équivalent L2 de
/// `SkillRegistry::apply_testimony_grade_ban` du côté MCP.
///
/// La doctrine non-transit (mika#1798) interdit de *proposer* autant que
/// d'*accéder* : mika#1959 a mis l'appel sous garde L4, mais l'outil restait
/// dans `tool_definitions()`, donc dans le lot envoyé au modèle, qui pouvait
/// encore l'offrir. Cette fonction ferme cette moitié.
///
/// Trois propriétés que la signature porte à dessein :
///
/// - **Seules les définitions sont amputées.** `tool_routing` et
///   `tool_data_grades` gardent l'inventaire complet, pour qu'un appel
///   halluciné sur un nom non publié soit refusé comme
///   `TOOL_TESTIMONY_GRADE_FORBIDDEN` et non comme « outil inconnu ». Le refus
///   L4 reste donc la dernière porte, pas la seule.
/// - **Une définition absente de la map est publiée.** Le défaut est de
///   publier, pas de censurer : un trou dans la map est une erreur de
///   construction, et une censure muette serait le pire rendu possible.
/// - **Libre et pure**, parce que `connect_all` ouvre de vraies connexions
///   qu'aucun test unitaire n'atteint (même raison que `mcp_tool_data_grades`).
///
/// Journalise une ligne par outil retiré : sans elle, « ce serveur n'est pas
/// connecté » et « ses outils sont bannis » rendent les mêmes octets
/// (classe mika#2205).
pub(crate) fn retain_publiables(
    tool_definitions: Vec<ToolDefinition>,
    tool_data_grades: &HashMap<String, DataGrade>,
) -> Vec<ToolDefinition> {
    tool_definitions
        .into_iter()
        .filter(|def| {
            if tool_data_grades.get(&def.name) != Some(&DataGrade::Testimony) {
                return true;
            }
            warn!(
                event = "mcp_tool_testimony_unpublished",
                tool = %def.name,
                server = serveur_du_nom_namespace(&def.name).unwrap_or("<non namespacé>"),
                doctrine = "mika#1798",
                "outil MCP dont le serveur déclare data_grade = \"testimony\" : \
                 retiré du lot publié au modèle, qui ne peut donc plus le proposer. \
                 L'appel reste refusé par la garde L4 (mika#1959)."
            );
            false
        })
        .collect()
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

        let tool_data_grades = mcp_tool_data_grades(config, &tool_routing);

        // mika#2814 — la non-publication des outils testimony, consommant la
        // même map que la garde L4, jamais une seconde lecture de la config.
        let discovered = tool_definitions.len();
        let tool_definitions = retain_publiables(tool_definitions, &tool_data_grades);
        let published = tool_definitions.len();

        if !connections.is_empty() {
            info!(
                servers = connections.len(),
                discovered,
                published,
                // Écrit même à zéro : absent ne vaut pas zéro (mika#2331), et
                // l'absence de la ligne signifie un binaire antérieur à
                // mika#2814 (mika#2340), jamais « rien n'a été banni ».
                unpublished = discovered - published,
                "MCP manager initialized"
            );
        }

        Self {
            connections,
            tool_definitions,
            tool_routing,
            tool_data_grades,
        }
    }

    /// Get tool definitions for all connected MCP servers.
    pub fn tool_definitions(&self) -> &[ToolDefinition] {
        &self.tool_definitions
    }

    /// Grade par outil namespacé, pour l'ensemencement de la map L4.
    pub fn tool_data_grades(&self) -> &HashMap<String, DataGrade> {
        &self.tool_data_grades
    }

    /// Manager sans aucune connexion, routage et grades fournis (mika#1959).
    ///
    /// Pour les tests d'autres modules du crate : `connect_all` ouvre de vraies
    /// connexions, qu'aucun test unitaire ne peut obtenir. `connections` reste
    /// vide, donc `is_mcp_tool` répond **vrai** et `call_tool` rend
    /// « MCP server '<serveur>' is not connected. ».
    ///
    /// C'est précisément ce qui rend le test de refus L4 décisif plutôt que
    /// tautologique : cette phrase est la preuve que `call_tool` a été atteint,
    /// donc son absence est la preuve qu'il ne l'a pas été. Un manager à `None`
    /// ne prouverait rien — le troisième étage du dispatch serait sauté de
    /// toute façon.
    #[cfg(test)]
    pub(crate) fn sans_connexion_pour_tests(
        tool_routing: HashMap<String, (String, String)>,
        tool_data_grades: HashMap<String, DataGrade>,
    ) -> Self {
        Self {
            connections: HashMap::new(),
            tool_definitions: Vec::new(),
            tool_routing,
            tool_data_grades,
        }
    }

    /// Check if a tool name belongs to an MCP server.
    pub fn is_mcp_tool(&self, name: &str) -> bool {
        self.tool_routing.contains_key(name)
    }

    /// Cet outil MCP est-il banni par la doctrine non-transit — c'est-à-dire son
    /// serveur déclare-t-il `data_grade = "testimony"` (mika#2814 vol 4) ?
    ///
    /// **Un outil absent de la map n'est pas banni.** `None ⇒ false` : le défaut
    /// est de publier, pas de censurer, exactement ce que
    /// [`retain_publiables`] écrit pour la même map. Un trou dans la map est
    /// une erreur de construction, et une censure muette en serait le pire
    /// rendu.
    ///
    /// **Le prédicat vit sur le manager, et pas au site de lecture.** La map
    /// est déjà publique ([`McpManager::tool_data_grades`]), donc un appelant
    /// pourrait écrire la comparaison lui-même. Deux raisons de ne pas le
    /// laisser faire : « cet outil est-il banni ? » se pose alors dans le même
    /// vocabulaire que [`retain_publiables`], et une comparaison de
    /// [`DataGrade`] recopiée ailleurs serait un second endroit à mettre à jour
    /// le jour où un troisième grade apparaît.
    pub fn is_testimony_tool(&self, name: &str) -> bool {
        self.tool_data_grades.get(name) == Some(&DataGrade::Testimony)
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

        // rmcp 2.0: CallToolRequestParams is #[non_exhaustive] — construct via
        // the name constructor + builder instead of a struct literal.
        let mut params = CallToolRequestParams::new(tool_name.clone());
        if let Some(arguments) = input.as_object().cloned() {
            params = params.with_arguments(arguments);
        }

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
    // environment. This prevents leaking MIKA_ANTHROPIC_API_KEY and other secrets
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
        match content {
            ContentBlock::Text(t) => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t.text);
            }
            ContentBlock::Image(img) => {
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

    // Truncate text output to a byte budget, floored to a char boundary
    // (mika#2103 — one canonical implementation, not a local re-derivation).
    if text.len() > MAX_OUTPUT_LEN {
        text = format!(
            "{}\n... (truncated at {MAX_OUTPUT_LEN} bytes)",
            mika_common::text::safe_truncate(&text, MAX_OUTPUT_LEN)
        );
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
    use rmcp::model::{CallToolResult, ContentBlock};

    /// Create an empty manager for testing.
    fn empty_manager() -> McpManager {
        McpManager {
            connections: HashMap::new(),
            tool_definitions: Vec::new(),
            tool_routing: HashMap::new(),
            tool_data_grades: HashMap::new(),
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
        let result = CallToolResult::success(vec![ContentBlock::text("Hello from MCP")]);
        let output = convert_mcp_result(&result, "greet");
        assert!(!output.is_error);
        assert_eq!(output.content, "Hello from MCP");
        assert!(output.images.is_empty());
    }

    #[test]
    fn test_convert_mcp_result_error() {
        let result = CallToolResult::error(vec![ContentBlock::text("Something went wrong")]);
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
        let result = CallToolResult::success(vec![ContentBlock::text(long_text)]);
        let output = convert_mcp_result(&result, "verbose");
        assert!(output.content.contains("truncated"));
        assert!(output.content.len() < MAX_OUTPUT_LEN + 100);
    }

    #[test]
    fn test_convert_mcp_result_multiple_text_blocks() {
        let result = CallToolResult::success(vec![
            ContentBlock::text("Line 1"),
            ContentBlock::text("Line 2"),
            ContentBlock::text("Line 3"),
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
                data_grade: DataGrade::default(),
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
                data_grade: DataGrade::default(),
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

    // -- mika#1959 : la population des grades par outil ---------------------

    /// Un routage à la main, de la forme que `connect_all` produit.
    fn routage(entrees: &[(&str, &str)]) -> HashMap<String, (String, String)> {
        entrees
            .iter()
            .map(|(serveur, outil)| {
                (
                    mcp_tool_name(serveur, outil),
                    (serveur.to_string(), outil.to_string()),
                )
            })
            .collect()
    }

    /// AC2 — chaque outil porte le grade **déclaré** par son serveur.
    ///
    /// La logique sous assertion est la fonction pure `mcp_tool_data_grades`
    /// (décision D4) : `connect_all` ouvre de vraies connexions stdio/HTTP,
    /// donc un test qui remplirait la map à la main n'assurerait rien de la
    /// jointure config × routage qui est tout le travail de ce ticket.
    #[test]
    fn mika1959_chaque_outil_porte_le_grade_de_son_serveur() {
        let json = r#"{
            "mcpServers": {
                "journal-perso": {
                    "transport": "stdio",
                    "command": "npx",
                    "data_grade": "testimony"
                },
                "wikipedia": {
                    "transport": "http",
                    "url": "https://mcp.wikipedia.org/",
                    "data_grade": "operational"
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        let routing = routage(&[
            ("journal-perso", "read_entry"),
            ("journal-perso", "search"),
            ("wikipedia", "lookup"),
        ]);

        let grades = mcp_tool_data_grades(&config, &routing);

        assert_eq!(
            grades.len(),
            3,
            "chaque outil du routage doit avoir une entrée : la map L4 est \
             indexée par nom d'outil, un outil sans entrée est un outil hors garde"
        );
        assert_eq!(
            grades["mcp__journal-perso__read_entry"],
            DataGrade::Testimony,
            "le grade doit venir du serveur qui route l'outil, pas d'une constante"
        );
        assert_eq!(
            grades["mcp__journal-perso__search"],
            DataGrade::Testimony,
            "deux outils d'un même serveur testimony portent tous deux son grade"
        );
        assert_eq!(
            grades["mcp__wikipedia__lookup"],
            DataGrade::Operational,
            "un serveur operational du même manifeste ne doit pas être contaminé"
        );
    }

    /// AC2 — un serveur sans champ donne `Operational`, et un serveur absent de
    /// la config retombe sur le même défaut.
    ///
    /// Le second cas est inatteignable depuis `connect_all` (le routage y est
    /// bâti depuis cette même config) ; il est couvert parce que la signature
    /// de la fonction pure le permet, et parce que le `unwrap_or_default()` qui
    /// le traite est exactement la ligne qu'un futur `unwrap()` remplacerait.
    #[test]
    fn mika1959_un_serveur_sans_champ_ou_absent_vaut_operational() {
        let json = r#"{
            "mcpServers": {
                "serveur-quelconque": {
                    "transport": "stdio",
                    "command": "some-mcp"
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        let routing = routage(&[("serveur-quelconque", "do_thing"), ("fantome", "do_thing")]);

        let grades = mcp_tool_data_grades(&config, &routing);

        assert_eq!(
            grades["mcp__serveur-quelconque__do_thing"],
            DataGrade::Operational,
            "champ absent ⇒ défaut sûr : le serveur tourne, L4 ne tire pas"
        );
        assert_eq!(
            grades["mcp__fantome__do_thing"],
            DataGrade::Operational,
            "serveur hors config ⇒ défaut de l'enum, jamais un panic"
        );
    }

    // -- mika#2814 : la non-publication des outils testimony ----------------

    /// Des définitions d'outils de la forme que `connect_all` produit.
    fn definitions(entrees: &[(&str, &str)]) -> Vec<ToolDefinition> {
        entrees
            .iter()
            .map(|(serveur, outil)| ToolDefinition {
                name: mcp_tool_name(serveur, outil),
                description: format!("[MCP: {serveur}] {outil}"),
                input_schema: serde_json::json!({ "type": "object" }),
            })
            .collect()
    }

    /// Les noms publiés, dans l'ordre où le filtre les rend.
    fn noms(defs: &[ToolDefinition]) -> Vec<&str> {
        defs.iter().map(|d| d.name.as_str()).collect()
    }

    /// AC1 — un serveur testimony et un serveur operational dans la **même**
    /// table : seuls les outils du premier disparaissent.
    ///
    /// Les deux grades dans un seul appel, parce qu'un filtre qui vide tout et
    /// un filtre qui lit le grade rendent le même résultat sur une table
    /// mono-grade. C'est la cohabitation qui distingue les deux.
    #[test]
    fn mika2814_seuls_les_outils_testimony_sont_depublies() {
        let defs = definitions(&[
            ("journal-perso", "read_entry"),
            ("journal-perso", "search"),
            ("wikipedia", "lookup"),
        ]);
        let grades = HashMap::from([
            (
                mcp_tool_name("journal-perso", "read_entry"),
                DataGrade::Testimony,
            ),
            (
                mcp_tool_name("journal-perso", "search"),
                DataGrade::Testimony,
            ),
            (mcp_tool_name("wikipedia", "lookup"), DataGrade::Operational),
        ]);

        let publies = retain_publiables(defs, &grades);

        assert_eq!(
            noms(&publies),
            vec!["mcp__wikipedia__lookup"],
            "les deux outils du serveur testimony doivent être retirés et celui \
             du serveur operational conservé — un filtre aveugle au grade rendrait \
             soit les trois, soit aucun"
        );
    }

    /// AC1 — une définition **absente** de la map des grades est **publiée**.
    ///
    /// Le défaut est de publier, pas de censurer : un trou dans la map est une
    /// erreur de construction, et la censurer en silence serait le pire rendu
    /// possible — un outil disparu sans qu'aucune ligne ne le dise. Ce cas est
    /// inatteignable depuis `connect_all` (définitions et grades y sont bâtis
    /// depuis le même routage) ; il est couvert parce que la signature le
    /// permet, et parce que le `!= Some(&Testimony)` qui le traite est
    /// exactement la ligne qu'un futur `== Some(&Operational)` inverserait.
    #[test]
    fn mika2814_une_definition_hors_map_est_publiee() {
        let defs = definitions(&[("fantome", "do_thing"), ("journal-perso", "read_entry")]);
        let grades = HashMap::from([(
            mcp_tool_name("journal-perso", "read_entry"),
            DataGrade::Testimony,
        )]);

        let publies = retain_publiables(defs, &grades);

        assert_eq!(
            noms(&publies),
            vec!["mcp__fantome__do_thing"],
            "un outil sans entrée dans la map doit rester publié : le défaut de \
             ce filtre est de publier"
        );
    }

    /// AC3 — le filtre n'ampute **que** les définitions : le routage et la map
    /// des grades gardent l'inventaire complet.
    ///
    /// C'est ce qui rend le refus L4 de mika#1959 décisif plutôt qu'un hasard :
    /// un appel halluciné sur un nom non publié est refusé comme
    /// `TOOL_TESTIMONY_GRADE_FORBIDDEN`, et non comme « outil inconnu ». Les
    /// deux rendus ne se valent pas — le second ferait croire à une faute de
    /// frappe là où il y a une doctrine.
    #[test]
    fn mika2814_le_routage_et_les_grades_ne_sont_pas_amputes() {
        let outil = mcp_tool_name("journal-perso", "read_entry");
        let grades = HashMap::from([(outil.clone(), DataGrade::Testimony)]);

        let publies = retain_publiables(definitions(&[("journal-perso", "read_entry")]), &grades);
        assert!(
            publies.is_empty(),
            "l'outil testimony doit être dépublié, or : {:?}",
            noms(&publies)
        );

        let manager = McpManager::sans_connexion_pour_tests(
            routage(&[("journal-perso", "read_entry")]),
            grades,
        );
        assert!(
            manager.is_mcp_tool(&outil),
            "{outil} doit rester routable : sans lui, le dispatch rendrait \
             « outil inconnu » au lieu du refus L4"
        );
        assert_eq!(
            manager.tool_data_grades().get(&outil),
            Some(&DataGrade::Testimony),
            "{outil} doit garder son grade dans la map que L4 consomme"
        );
    }

    /// Contrôle de bonne foi — une table sans aucun testimony ressort
    /// **inchangée**, dans le même ordre.
    ///
    /// Sans lui, un filtre dégénéré rendant `vec![]` passerait les trois
    /// assertions ci-dessus : chacune mesure une absence, et l'absence est
    /// aussi ce que rend un instrument qui ne mesure rien.
    #[test]
    fn mika2814_controle_une_table_sans_testimony_ressort_intacte() {
        let entrees = [("wikipedia", "lookup"), ("github", "list_issues")];
        let grades: HashMap<String, DataGrade> = entrees
            .iter()
            .map(|(s, o)| (mcp_tool_name(s, o), DataGrade::Operational))
            .collect();
        let avant = definitions(&entrees);
        let noms_avant: Vec<String> = avant.iter().map(|d| d.name.clone()).collect();

        let publies = retain_publiables(avant, &grades);

        assert_eq!(
            noms(&publies),
            noms_avant.iter().map(String::as_str).collect::<Vec<_>>(),
            "aucun grade testimony ⇒ aucune dépublication, et l'ordre préservé \
             (c'est l'ordre dans lequel le modèle reçoit les outils)"
        );
    }
}
