use anyhow::Result;
use async_trait::async_trait;
use mika_a2a::MessageSendParams;
use mika_a2a::client::A2aClient;
use mika_a2a::types::{Message, Part, Role};
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use tracing::debug;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

/// Error returned when `a2a_call` is targeted at a loopback/private address
/// (mika#1653). The address shape signals a local agent, which `a2a_call`
/// cannot reach — so the message redirects to the correct mechanism instead of
/// dead-ending on a refusal.
const LOCAL_TARGET_HINT: &str = "This looks like a local agent on this host \
    (loopback/private address). `a2a_call` only reaches external, remote \
    (cross-container) agents by URL. To reach a local agent, use `delegate_task` \
    in conversation mode, or assign it a task in your team decompose JSON array \
    in a team run.";

pub struct A2aCallTool;

#[async_trait]
impl Tool for A2aCallTool {
    fn name(&self) -> &str {
        "a2a_call"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "a2a_call".to_string(),
            description: "Call an external, remote (cross-container) A2A \
                (Agent-to-Agent) agent addressed by URL. Sends a message via the \
                A2A protocol's message/send method and returns the agent's \
                response. Use this ONLY for external agents that expose a public \
                A2A endpoint. This is NOT how you reach a local agent on this \
                host: local agents are addressed by name, not URL. To reach a \
                local agent in conversation mode use `delegate_task`; in a team \
                run, assign the member a task in your decompose JSON array (the \
                engine spawns it for you). `a2a_call` cannot reach local siblings \
                and will fail on loopback/private URLs."
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

        // SSRF protection: validate URL scheme, host, and IP ranges
        let parsed_url = match url::Url::parse(url) {
            Ok(u) => u,
            Err(e) => return Ok(ToolOutput::error(format!("Invalid URL: {e}"))),
        };

        if !matches!(parsed_url.scheme(), "http" | "https") {
            return Ok(ToolOutput::error("Only http and https URLs are allowed."));
        }

        if let Some(host) = parsed_url.host_str() {
            if host == "localhost" || host == "127.0.0.1" || host == "::1" {
                return Ok(ToolOutput::error(LOCAL_TARGET_HINT));
            }
            if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                let is_private = match ip {
                    std::net::IpAddr::V4(v4) => {
                        v4.is_private() || v4.is_loopback() || v4.is_link_local()
                    }
                    std::net::IpAddr::V6(v6) => v6.is_loopback(),
                };
                if is_private {
                    return Ok(ToolOutput::error(LOCAL_TARGET_HINT));
                }
            }
        } else {
            return Ok(ToolOutput::error("URL must have a host."));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[test]
    fn test_description_is_remote_only_and_points_to_delegate_task() {
        // mika#1653 Layer 3: the static description must mark the tool as
        // remote/external-only and name `delegate_task` as the local alternative.
        let desc = A2aCallTool.definition().description;
        let lower = desc.to_lowercase();
        assert!(
            lower.contains("remote") || lower.contains("external"),
            "a2a_call description must mark itself remote/external-only; got: {desc}"
        );
        assert!(
            desc.contains("delegate_task"),
            "a2a_call description must redirect local targets to delegate_task; got: {desc}"
        );
    }

    #[tokio::test]
    async fn test_localhost_target_redirects_to_delegate_task() {
        // The SSRF guard fires before any network/DB access, so the harness ctx
        // is sufficient. The rejection must be actionable, not a dead end.
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = A2aCallTool;

        let result = tool
            .execute(
                serde_json::json!({"url": "http://localhost:8080/a2a", "message": "hi"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error, "localhost target must be rejected");
        assert!(
            result.content.contains("delegate_task"),
            "localhost rejection must point to delegate_task; got: {}",
            result.content
        );
        assert!(
            result.content.to_lowercase().contains("local agent"),
            "localhost rejection must name the local-agent alternative; got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_private_ip_target_redirects_to_delegate_task() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = A2aCallTool;

        let result = tool
            .execute(
                serde_json::json!({"url": "http://10.0.0.5/a2a", "message": "hi"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error, "private-IP target must be rejected");
        assert!(
            result.content.contains("delegate_task"),
            "private-IP rejection must point to delegate_task; got: {}",
            result.content
        );
    }
}
