mod cancel_reminder;
mod create_reminder;
mod list_reminders;
mod search_memory;
mod send_message;
mod store_fact;
mod update_core_memory;
mod update_fact;

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::AtomicU32;

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;

/// Maximum length (in characters) allowed for any single string input to a tool.
pub const MAX_INPUT_LEN: usize = 10_000;

/// Context available to every tool during execution.
pub struct ToolContext<'a> {
    pub db: &'a AsyncDatabase,
    pub session_id: &'a str,
    pub home_dir: &'a Path,
    pub core_memory_edit_count: &'a AtomicU32,
    pub is_onboarding: bool,
    pub message_sender: Option<&'a dyn MessageSender>,
}

/// A tool that the agent can invoke via Claude's tool_use.
#[async_trait(?Send)]
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
