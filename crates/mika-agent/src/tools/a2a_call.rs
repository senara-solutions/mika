use anyhow::Result;
use async_trait::async_trait;
use mika_a2a::MessageSendParams;
use mika_a2a::client::A2aClient;
use mika_a2a::types::{Message, Part, Role};
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use tracing::debug;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct A2aCallTool;

#[async_trait]
impl Tool for A2aCallTool {
    fn name(&self) -> &str {
        "a2a_call"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "a2a_call".to_string(),
            description: "Call a remote A2A (Agent-to-Agent) agent. Sends a message via the \
                A2A protocol's message/send method and returns the agent's response. Use this \
                to interact with external agents that expose an A2A endpoint."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The A2A agent's endpoint URL (e.g. 'https://agent.example.com/a2a')"
                    },
                    "message": {
                        "type": "string",
                        "description": "The message to send to the remote agent"
                    },
                    "api_key": {
                        "type": "string",
                        "description": "Optional Bearer token for authenticating with the remote agent"
                    }
                },
                "required": ["url", "message"]
            }),
        }
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(120)
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let url = input["url"].as_str().unwrap_or("");
        if url.is_empty() {
            return Ok(ToolOutput::error("'url' is required."));
        }
        if url.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'url' too long: {} characters (max: {MAX_INPUT_LEN})",
                url.len()
            )));
        }

        let message_text = input["message"].as_str().unwrap_or("");
        if message_text.is_empty() {
            return Ok(ToolOutput::error("'message' is required."));
        }
        if message_text.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'message' too long: {} characters (max: {MAX_INPUT_LEN})",
                message_text.len()
            )));
        }

        let api_key = input["api_key"].as_str().map(|s| s.to_string());

        let client = A2aClient::new(url, api_key);

        let params = MessageSendParams {
            message: Message {
                message_id: uuid::Uuid::new_v4().to_string(),
                role: Role::User,
                parts: vec![Part::Text {
                    text: message_text.to_string(),
                    metadata: None,
                }],
                context_id: None,
                task_id: None,
                metadata: None,
                reference_task_ids: None,
                extensions: None,
                kind: "message".to_string(),
            },
            configuration: None,
            metadata: None,
        };

        debug!(url = %url, "a2a_call: sending message/send request");

        let task = match client.send_message(params).await {
            Ok(task) => task,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "A2A call to '{url}' failed: {e}"
                )));
            }
        };

        debug!(
            task_id = %task.id,
            state = %task.status.state,
            "a2a_call: received response"
        );

        // Extract text from artifacts and history
        let mut parts_text = Vec::new();

        // 1. Extract text from artifacts
        if let Some(artifacts) = &task.artifacts {
            for artifact in artifacts {
                for part in &artifact.parts {
                    if let Part::Text { text, .. } = part {
                        parts_text.push(text.clone());
                    }
                }
            }
        }

        // 2. Extract text from history (agent messages only)
        if let Some(history) = &task.history {
            for msg in history {
                if msg.role == Role::Agent {
                    for part in &msg.parts {
                        if let Part::Text { text, .. } = part {
                            parts_text.push(text.clone());
                        }
                    }
                }
            }
        }

        // 3. Fall back to the status message if no text was found above
        if parts_text.is_empty()
            && let Some(ref status_msg) = task.status.message
        {
            for part in &status_msg.parts {
                if let Part::Text { text, .. } = part {
                    parts_text.push(text.clone());
                }
            }
        }

        if parts_text.is_empty() {
            Ok(ToolOutput::success(format!(
                "A2A task {} completed with state '{}' but produced no text output.",
                task.id, task.status.state
            )))
        } else {
            Ok(ToolOutput::success(parts_text.join("\n\n")))
        }
    }
}
