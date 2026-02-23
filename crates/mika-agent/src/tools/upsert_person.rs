use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput, MAX_INPUT_LEN};

pub struct UpsertPersonTool;

#[async_trait(?Send)]
impl Tool for UpsertPersonTool {
    fn name(&self) -> &str {
        "upsert_person"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "upsert_person".to_string(),
            description: "Remember a person the user mentions. Creates a new entry or updates an existing one. Use this whenever someone is mentioned by name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Full name of the person (canonical form, e.g., 'Sarah Chen')"
                    },
                    "relationship": {
                        "type": "string",
                        "description": "Relationship to the user (e.g., 'colleague', 'manager', 'spouse', 'client')"
                    },
                    "notes": {
                        "type": "string",
                        "description": "Key facts about this person (e.g., 'VP of Engineering at Acme Corp, prefers morning meetings')"
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let name = input["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return Ok(ToolOutput::error("'name' is required."));
        }

        if name.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "Input too long: 'name' is {} characters (max: {}). Please shorten it.",
                name.len(),
                MAX_INPUT_LEN
            )));
        }

        let relationship = input["relationship"].as_str();
        let notes = input["notes"].as_str();

        if let Some(r) = relationship {
            if r.len() > MAX_INPUT_LEN {
                return Ok(ToolOutput::error(format!(
                    "Input too long: 'relationship' is {} characters (max: {}). Please shorten it.",
                    r.len(),
                    MAX_INPUT_LEN
                )));
            }
        }
        if let Some(n) = notes {
            if n.len() > MAX_INPUT_LEN {
                return Ok(ToolOutput::error(format!(
                    "Input too long: 'notes' is {} characters (max: {}). Please shorten it.",
                    n.len(),
                    MAX_INPUT_LEN
                )));
            }
        }

        ctx.db.upsert_person(name, relationship, notes)?;

        Ok(ToolOutput::success(format!("Remembered {name}.")))
    }
}
