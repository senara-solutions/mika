use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};

const MAX_CORE_MEMORY_TOKENS: i32 = 2000;

pub struct UpdateCoreMemoryTool;

#[async_trait(?Send)]
impl Tool for UpdateCoreMemoryTool {
    fn name(&self) -> &str {
        "update_core_memory"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_core_memory".to_string(),
            description: "Update your persistent core memory. Core memory is always visible to you in the system prompt. Use this to remember important facts about the user. Keys: 'user_summary', 'persona', 'current_goals'. You can also create custom keys.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "The memory key to update (e.g., 'user_summary', 'persona', 'current_goals')"
                    },
                    "value": {
                        "type": "string",
                        "description": "The new value for this memory key. Write in third person about the user."
                    }
                },
                "required": ["key", "value"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let key = input["key"].as_str().unwrap_or("");
        let value = input["value"].as_str().unwrap_or("");

        if key.is_empty() || value.is_empty() {
            return Ok(ToolOutput::error("Both 'key' and 'value' are required."));
        }

        // Check token limit before writing
        let current_total = ctx.db.total_core_memory_tokens()?;
        let new_tokens = (value.len() / 4) as i32;

        // Get current tokens for this key (if updating, we're replacing)
        let existing_tokens = ctx
            .db
            .get_core_memory(key)?
            .map(|e| e.token_count)
            .unwrap_or(0);

        let projected = current_total - existing_tokens + new_tokens;
        if projected > MAX_CORE_MEMORY_TOKENS {
            return Ok(ToolOutput::error(format!(
                "Core memory limit exceeded. Current: {current_total} tokens, this update would be: {projected} tokens (max: {MAX_CORE_MEMORY_TOKENS}). Please shorten the value or remove other entries first."
            )));
        }

        ctx.db.set_core_memory(key, value)?;

        Ok(ToolOutput::success(format!(
            "Updated core memory '{key}'. Total core memory: {projected}/{MAX_CORE_MEMORY_TOKENS} tokens."
        )))
    }
}
