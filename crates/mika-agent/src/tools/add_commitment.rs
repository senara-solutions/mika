use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput, MAX_INPUT_LEN};

pub struct AddCommitmentTool;

#[async_trait(?Send)]
impl Tool for AddCommitmentTool {
    fn name(&self) -> &str {
        "add_commitment"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "add_commitment".to_string(),
            description: "Track a commitment or task the user mentions. Duplicates are automatically ignored.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "What needs to be done (e.g., 'Review Q4 budget with Sarah')"
                    },
                    "due_date": {
                        "type": "string",
                        "description": "Due date in ISO 8601 format (e.g., '2026-03-01'). Optional."
                    },
                    "person_name": {
                        "type": "string",
                        "description": "Name of the person involved (must match a previously stored person). Optional."
                    }
                },
                "required": ["description"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let description = input["description"].as_str().unwrap_or("");
        if description.is_empty() {
            return Ok(ToolOutput::error("'description' is required."));
        }

        if description.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "Input too long: 'description' is {} characters (max: {}). Please shorten it.",
                description.len(),
                MAX_INPUT_LEN
            )));
        }

        let due_date = input["due_date"].as_str();

        if let Some(d) = due_date {
            if d.len() > MAX_INPUT_LEN {
                return Ok(ToolOutput::error(format!(
                    "Input too long: 'due_date' is {} characters (max: {}). Please shorten it.",
                    d.len(),
                    MAX_INPUT_LEN
                )));
            }
        }

        // Look up person by name if provided
        let person_id = if let Some(name) = input["person_name"].as_str() {
            if name.len() > MAX_INPUT_LEN {
                return Ok(ToolOutput::error(format!(
                    "Input too long: 'person_name' is {} characters (max: {}). Please shorten it.",
                    name.len(),
                    MAX_INPUT_LEN
                )));
            }
            ctx.db.get_person(name)?.map(|p| p.id)
        } else {
            None
        };

        ctx.db.add_commitment(description, due_date, person_id)?;

        let due_info = due_date
            .map(|d| format!(" (due: {d})"))
            .unwrap_or_default();
        Ok(ToolOutput::success(format!(
            "Tracked commitment: \"{description}\"{due_info}"
        )))
    }
}
