mod cancel_reminder;
mod create_reminder;
mod list_reminders;
pub mod list_workspace;
pub mod read_workspace;
mod search_memory;
mod send_message;
mod store_fact;
mod update_core_memory;
mod update_fact;
pub mod write_workspace;

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use mika_common::embedding::EmbeddingClient;

/// Maximum length (in characters) allowed for any single string input to a tool.
pub const MAX_INPUT_LEN: usize = 10_000;

/// Context available to every tool during execution.
pub struct ToolContext<'a> {
    pub db: &'a AsyncDatabase,
    pub session_id: &'a str,
    pub home_dir: &'a Path,
    pub core_memory_edit_count: &'a AtomicU32,
    pub is_onboarding: bool,
    pub message_sender: Option<Arc<dyn MessageSender>>,
    pub embedding_client: Option<&'a EmbeddingClient>,
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
}

/// Result of a tool execution.
#[derive(Debug)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
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
    registry.register(Box::new(send_message::SendMessageTool));
    registry
}
