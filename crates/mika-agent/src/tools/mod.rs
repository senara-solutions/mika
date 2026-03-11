mod cancel_reminder;
mod cancel_task;
mod complete_task;
mod create_agent;
mod create_reminder;
pub mod create_skill;
mod create_task;
mod create_team;
mod create_work_item;
mod delegate_task;
mod delete_skill;
mod delete_team;
mod get_config;
mod get_session_messages;
mod get_task;
mod get_team_history;
mod get_team_status;
mod list_agents;
mod list_audit_events;
mod list_home_files;
mod list_reminders;
mod list_skills;
mod list_tasks;
mod list_teams;
mod list_work_items;
mod list_workspace;
mod query_timeline;
mod read_home_file;
mod read_workspace;
mod run_team;
mod search_memory;
mod send_message;
mod set_config;
mod store_fact;
mod toggle_skill;
mod update_core_memory;
mod update_fact;
mod update_skill;
mod update_team;
mod update_work_item_status;
mod write_file;
mod write_workspace;

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use mika_common::config::Settings;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32};

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use mika_common::agent::DEFAULT_AGENT;
use mika_common::embedding::EmbeddingClient;
use mika_common::team;

/// Maximum length (in characters) allowed for any single string input to a tool.
pub const MAX_INPUT_LEN: usize = 10_000;

/// Context available to every tool during execution.
pub struct ToolContext<'a> {
    pub db: &'a AsyncDatabase,
    pub session_id: &'a str,
    pub trace_id: &'a str,
    pub home_dir: &'a Path,
    pub core_memory_edit_count: &'a AtomicU32,
    pub is_onboarding: bool,
    pub message_sender: Option<Arc<dyn MessageSender>>,
    pub embedding_client: Option<&'a EmbeddingClient>,
    pub brave_api_key: Option<&'a str>,
    /// Shared flag: set to `true` by skill-modifying tools after successful writes.
    /// The agent loop coordinator checks this before each turn and rebuilds the
    /// SkillRegistry if set, enabling hot-reload without restart.
    pub skills_dirty: &'a AtomicBool,
    /// True when running in reflection mode (daily memory review).
    /// Memory tools require an `evidence` field and use a higher edit cap.
    pub is_reflection: bool,
    /// True when running within a task context (callback, delegation, team agent).
    /// Blocks top-level work item creation (Guard 1).
    pub is_task_context: bool,
    /// True when running in a callback turn (Guard 3 — blocks ALL work item creation).
    pub is_callback_turn: bool,
}

/// A tool that the agent can invoke via Claude's tool_use.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique tool name (must match what Claude sees in the tool definition).
    fn name(&self) -> &str;

    /// Tool definition sent to Claude in the request.
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with the given JSON input.
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput>;

    /// Optional per-tool timeout override (in seconds).
    /// Returns `None` to use the default agent tool timeout.
    fn timeout_secs(&self) -> Option<u64> {
        None
    }
}

/// Image data produced by a tool, ready for inclusion in a Claude API tool_result.
#[derive(Debug, Clone)]
pub struct ImageData {
    /// MIME type: "image/jpeg", "image/png", "image/gif", or "image/webp".
    pub media_type: String,
    /// Base64-encoded image bytes.
    pub data: String,
}

/// Result of a tool execution.
#[derive(Debug)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
    /// Images to include alongside text in the tool result.
    /// When non-empty, the tool result is sent as a multi-block content array.
    pub images: Vec<ImageData>,
}

impl ToolOutput {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            images: vec![],
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
            images: vec![],
        }
    }

    pub fn success_with_images(content: impl Into<String>, images: Vec<ImageData>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            images,
        }
    }
}

/// Index a fact into the search system (FTS5 + optional vector embedding).
///
/// Best-effort: logs warnings on failure but never propagates errors,
/// since search indexing should not block tool responses.
pub(crate) async fn index_fact(
    ctx: &ToolContext<'_>,
    source_type: &str,
    source_id: i64,
    content: &str,
) {
    // Delete any existing index entry for this source (handles upserts)
    let _ = ctx.db.delete_search_content(source_type, source_id).await;

    // Index into FTS5
    let content_id = match ctx
        .db
        .index_content(source_type, Some(source_id), content)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!(source_type, source_id, error = %e, "failed to index content for search");
            return;
        }
    };

    // Generate and store embedding if client is available
    if let Some(client) = ctx.embedding_client {
        match client.embed(content).await {
            Ok(embedding) => {
                if let Err(e) = ctx.db.index_embedding(content_id, embedding).await {
                    tracing::warn!(source_type, source_id, error = %e, "failed to index embedding");
                }
            }
            Err(e) => {
                tracing::warn!(source_type, source_id, error = %e, "failed to generate embedding");
            }
        }
    }
}

/// Check that the `evidence` field is present and non-empty when running in reflection mode.
/// Returns `Some(ToolOutput::error(...))` if evidence is missing, `None` if valid.
pub(crate) fn check_reflection_evidence(
    ctx: &ToolContext<'_>,
    input: &serde_json::Value,
) -> Option<ToolOutput> {
    if ctx.is_reflection {
        let evidence = input["evidence"].as_str().unwrap_or("").trim();
        if evidence.is_empty() {
            return Some(ToolOutput::error(
                "Reflection mode requires an evidence field citing specific conversation content.",
            ));
        }
    }
    None
}

/// Validate that a `work_item_id` references an active manual work item.
/// Returns `Some(error_message)` if validation fails, `None` if valid.
pub(crate) async fn validate_work_item(
    db: &crate::async_db::AsyncDatabase,
    work_item_id: &str,
) -> Option<String> {
    if work_item_id.is_empty() {
        return Some(
            "You must create a work item first using create_work_item, then pass its ID here. \
             No delegation without tracking."
                .to_string(),
        );
    }
    match db.get_task(work_item_id).await {
        Ok(Some(ref wi))
            if wi.trigger_type == "manual"
                && matches!(wi.status.as_str(), "pending" | "in_progress" | "blocked") =>
        {
            None
        }
        Ok(Some(_)) => Some(format!(
            "Work item '{work_item_id}' is not an active work item. \
             It must be a manual work item with status pending, in_progress, or blocked."
        )),
        Ok(None) => Some(format!(
            "Work item '{work_item_id}' not found. \
             Create one first using create_work_item."
        )),
        Err(e) => Some(format!("Failed to validate work item: {e}")),
    }
}

/// Check if the given agent is an orchestrator (default agent or listed as orchestrator in any team).
pub(crate) fn is_orchestrator(home_dir: &Path, agent_id: &str) -> bool {
    if agent_id == DEFAULT_AGENT {
        return true;
    }
    for team_name in team::list_teams(home_dir) {
        if let Ok(def) = team::load_team(home_dir, &team_name)
            && def.team.orchestrator == agent_id
        {
            return true;
        }
    }
    false
}

/// Validate a relative path and resolve it to a full path within `base_dir`.
///
/// Performs the following security checks:
/// 1. Non-empty path
/// 2. Path length within `MAX_INPUT_LEN`
/// 3. Absolute paths rejected
/// 4. Path traversal components (`..`, root, prefix) rejected
/// 5. Parent directories created only when `create_parents` is `true`
/// 6. Parent directory symlink check (when parent exists)
/// 7. Canonicalize containment check (resolved parent must be within `base_dir`, when parent exists)
///
/// `create_parents` should be `true` for write operations and `false` for read-only operations
/// to avoid creating directories as a side effect of reading.
///
/// Returns `Ok(full_path)` on success or `Err(ToolOutput::error(...))` on failure.
pub(crate) async fn validate_and_resolve_path(
    path: &str,
    base_dir: &Path,
    create_parents: bool,
) -> std::result::Result<PathBuf, ToolOutput> {
    if path.is_empty() {
        return Err(ToolOutput::error("'path' is required and cannot be empty."));
    }
    if path.len() > MAX_INPUT_LEN {
        return Err(ToolOutput::error(format!(
            "Path exceeds maximum length of {MAX_INPUT_LEN} characters."
        )));
    }

    // Reject absolute paths
    if Path::new(path).is_absolute() {
        return Err(ToolOutput::error(
            "Absolute paths are not allowed. Use a relative path within the directory.",
        ));
    }

    // Prevent path traversal using component inspection
    for component in Path::new(path).components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ToolOutput::error(
                    "Path traversal components ('..', root, or prefix) are not allowed.",
                ));
            }
            _ => {}
        }
    }

    let full_path = base_dir.join(path);

    if let Some(parent) = full_path.parent() {
        // Create parent directories only for write operations
        if create_parents && let Err(e) = tokio::fs::create_dir_all(parent).await {
            return Err(ToolOutput::error(format!(
                "Failed to create parent directories: {e}"
            )));
        }

        // Check for symlinks in the parent chain (only when parent exists)
        match tokio::fs::symlink_metadata(parent).await {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(ToolOutput::error(
                        "Symbolic links are not allowed in the path.",
                    ));
                }

                // Verify containment using canonicalize (parent exists)
                let canonical_parent = match parent.canonicalize() {
                    Ok(c) => c,
                    Err(e) => {
                        return Err(ToolOutput::error(format!(
                            "Failed to resolve parent directory: {e}"
                        )));
                    }
                };
                let base_canonical = match base_dir.canonicalize() {
                    Ok(c) => c,
                    Err(_) => {
                        return Err(ToolOutput::error("Base directory does not exist."));
                    }
                };
                if !canonical_parent.starts_with(&base_canonical) {
                    return Err(ToolOutput::error(
                        "Path resolves outside the base directory.",
                    ));
                }
            }
            Err(_) => {
                // Parent does not exist — only an error for write operations (create_parents would
                // have already failed above). For read operations, the file-not-found error will
                // be returned by the caller when it tries to open the file.
                if create_parents {
                    return Err(ToolOutput::error("Failed to verify parent directory."));
                }
            }
        }
    }

    Ok(full_path)
}

/// Registry of available tools.
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
    cached_defs: Vec<ToolDefinition>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            cached_defs: Vec::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.cached_defs.push(tool.definition());
        self.tools.push(tool);
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.cached_defs
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    /// Look up a cached tool definition by name.
    pub fn definition_by_name(&self, name: &str) -> Option<&ToolDefinition> {
        self.tools
            .iter()
            .zip(self.cached_defs.iter())
            .find(|(tool, _)| tool.name() == name)
            .map(|(_, def)| def)
    }
}

/// Create a registry with workspace tools for team execution.
pub fn team_tools(workspace_dir: &Path) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(read_workspace::ReadWorkspaceTool {
            workspace_dir: workspace_dir.to_path_buf(),
        }),
        Box::new(write_workspace::WriteWorkspaceTool {
            workspace_dir: workspace_dir.to_path_buf(),
        }),
        Box::new(list_workspace::ListWorkspaceTool {
            workspace_dir: workspace_dir.to_path_buf(),
        }),
    ]
}

/// Create a registry with all built-in tools.
pub fn default_tools() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(update_core_memory::UpdateCoreMemoryTool));
    registry.register(Box::new(store_fact::StoreFactTool));
    registry.register(Box::new(search_memory::SearchMemoryTool));
    registry.register(Box::new(update_fact::UpdateFactTool));
    registry.register(Box::new(create_reminder::CreateReminderTool));
    registry.register(Box::new(list_reminders::ListRemindersTool));
    registry.register(Box::new(cancel_reminder::CancelReminderTool));
    registry.register(Box::new(cancel_task::CancelTaskTool));
    registry.register(Box::new(complete_task::CompleteTaskTool));
    registry.register(Box::new(get_task::GetTaskTool));
    registry.register(Box::new(send_message::SendMessageTool));
    registry.register(Box::new(create_skill::CreateSkillTool));
    registry.register(Box::new(delete_skill::DeleteSkillTool));
    registry.register(Box::new(list_skills::ListSkillsTool));
    registry.register(Box::new(toggle_skill::ToggleSkillTool));
    registry.register(Box::new(update_skill::UpdateSkillTool));
    registry.register(Box::new(get_config::GetConfigTool));
    registry.register(Box::new(set_config::SetConfigTool));
    registry.register(Box::new(write_file::WriteFileTool));
    registry.register(Box::new(read_home_file::ReadHomeFileTool));
    registry.register(Box::new(list_home_files::ListHomeFilesTool));
    registry.register(Box::new(list_tasks::ListTasksTool));
    registry.register(Box::new(create_work_item::CreateWorkItemTool));
    registry.register(Box::new(update_work_item_status::UpdateWorkItemStatusTool));
    registry.register(Box::new(list_work_items::ListWorkItemsTool));
    registry.register(Box::new(query_timeline::QueryTimelineTool));
    registry.register(Box::new(get_session_messages::GetSessionMessagesTool));
    registry.register(Box::new(list_audit_events::ListAuditEventsTool));
    registry
}

/// Return management tools based on the current agent/team configuration.
///
/// `create_agent` and `list_agents` are always available so the agent can
/// create new agents even from a single-agent setup. Delegation and team
/// tools (`delegate_task`, `run_team`, etc.) are only added when multiple
/// agents or teams exist.
pub fn management_tools_if_needed(home_dir: &Path, settings: &Settings) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = vec![
        Box::new(create_agent::CreateAgentTool {
            home_dir: home_dir.to_path_buf(),
        }),
        Box::new(create_team::CreateTeamTool {
            home_dir: home_dir.to_path_buf(),
        }),
        Box::new(list_agents::ListAgentsTool {
            home_dir: home_dir.to_path_buf(),
        }),
    ];

    let agents = mika_common::agent::list_agents(home_dir);
    let teams = mika_common::team::list_teams(home_dir);
    if agents.len() > 1 || !teams.is_empty() {
        tools.push(Box::new(list_teams::ListTeamsTool {
            home_dir: home_dir.to_path_buf(),
        }));
        tools.push(Box::new(run_team::RunTeamTool {
            home_dir: home_dir.to_path_buf(),
            settings: settings.clone(),
        }));
        tools.push(Box::new(delegate_task::DelegateTaskTool {
            home_dir: home_dir.to_path_buf(),
            settings: settings.clone(),
        }));
        tools.push(Box::new(get_team_status::GetTeamStatusTool));
        tools.push(Box::new(get_team_history::GetTeamHistoryTool));
        tools.push(Box::new(delete_team::DeleteTeamTool {
            home_dir: home_dir.to_path_buf(),
        }));
        tools.push(Box::new(update_team::UpdateTeamTool {
            home_dir: home_dir.to_path_buf(),
        }));
    }

    tools
}
