use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};

pub struct SetPreferenceTool;

#[async_trait(?Send)]
impl Tool for SetPreferenceTool {
    fn name(&self) -> &str {
        "set_preference"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "set_preference".to_string(),
            description: "Store a user preference. Use when the user expresses how they like things done.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Category of the preference (e.g., 'communication_style', 'meeting_time', 'report_format')"
                    },
                    "value": {
                        "type": "string",
                        "description": "Description of the preference (e.g., 'Prefers bullet points over paragraphs')"
                    }
                },
                "required": ["category", "value"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let category = input["category"].as_str().unwrap_or("");
        let value = input["value"].as_str().unwrap_or("");

        if category.is_empty() || value.is_empty() {
            return Ok(ToolOutput::error("Both 'category' and 'value' are required."));
        }

        ctx.db.set_preference(category, value)?;

        Ok(ToolOutput::success(format!(
            "Stored preference [{category}]: {value}"
        )))
    }
}
