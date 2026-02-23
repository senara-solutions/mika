use anyhow::{Context, Result};
use mika_common::claude::{
    ClaudeClient, ContentBlock, Message, MessageContent, MessagesRequest, StopReason,
};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::db::Database;
use crate::prompt;
use crate::tools::{ToolContext, ToolOutput, ToolRegistry};

const MAX_TOOL_STEPS: usize = 10;
const TOOL_TIMEOUT_SECS: u64 = 30;
const AGENT_TOTAL_TIMEOUT_SECS: u64 = 300;

/// Run the agent loop for a single inbound message.
/// Returns the assistant's text response.
pub async fn run_agent(
    db: &Database,
    claude: &ClaudeClient,
    tools: &ToolRegistry,
    user_message: &str,
    channel_type: &str,
) -> Result<String> {
    // Save the user message
    db.save_message("user", user_message, channel_type)?;

    let timeout_result = tokio::time::timeout(
        Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS),
        run_agent_inner(db, claude, tools, channel_type),
    )
    .await;

    match timeout_result {
        Ok(result) => result,
        Err(_elapsed) => {
            warn!(
                timeout_secs = AGENT_TOTAL_TIMEOUT_SECS,
                "agent loop total timeout exceeded"
            );
            let fallback =
                "I'm sorry, that took too long. Let me try a simpler approach next time.";
            db.save_message("assistant", fallback, channel_type)?;
            Ok(fallback.to_string())
        }
    }
}

/// Inner agent loop, separated so the outer function can wrap it in a timeout.
async fn run_agent_inner(
    db: &Database,
    claude: &ClaudeClient,
    tools: &ToolRegistry,
    channel_type: &str,
) -> Result<String> {
    // Load context
    let system = prompt::load_system_prompt(db)?;
    let history = db.load_recent_messages(20)?;
    let tool_defs = tools.definitions();

    // Build initial message list from history
    let messages: Vec<Message> = history
        .iter()
        .map(|msg| Message {
            role: msg.role.clone(),
            content: MessageContent::Text(msg.content.clone()),
        })
        .collect();

    let tool_ctx = ToolContext { db };

    // Build the request once; only messages changes between iterations.
    // We clone the request when passing to send_message (which takes ownership),
    // then push new messages directly onto the original to avoid rebuilding
    // system (~4KB) and tool_defs (all JSON schemas) each iteration.
    let mut request = MessagesRequest {
        model: claude.model.clone(),
        max_tokens: claude.max_tokens,
        system: Some(system),
        messages,
        tools: if tool_defs.is_empty() {
            None
        } else {
            Some(tool_defs)
        },
    };

    for step in 0..MAX_TOOL_STEPS {
        debug!(step, messages_len = request.messages.len(), "agent loop step");

        let response = claude
            .send_message(request.clone())
            .await
            .context("Claude API call failed")?;

        match response.stop_reason {
            StopReason::EndTurn | StopReason::MaxTokens => {
                let text = response.text();
                if !text.is_empty() {
                    db.save_message("assistant", &text, channel_type)?;
                }
                info!(step, stop_reason = ?response.stop_reason, "agent done");
                return Ok(text);
            }
            StopReason::ToolUse => {
                // Add the assistant's response (with tool_use blocks) directly to the request
                request.messages.push(Message {
                    role: "assistant".to_string(),
                    content: MessageContent::Blocks(response.content.clone()),
                });

                // Execute each tool call
                let mut tool_results = Vec::new();
                for block in &response.content {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        debug!(tool = %name, "executing tool");
                        let output = execute_tool(tools, name, input.clone(), &tool_ctx).await;
                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: output.content,
                            is_error: if output.is_error { Some(true) } else { None },
                        });
                    }
                }

                // Add tool results as a user message directly to the request
                request.messages.push(Message {
                    role: "user".to_string(),
                    content: MessageContent::Blocks(tool_results),
                });
            }
            StopReason::StopSequence => {
                let text = response.text();
                if !text.is_empty() {
                    db.save_message("assistant", &text, channel_type)?;
                }
                return Ok(text);
            }
        }
    }

    // Exceeded max steps
    warn!("agent loop exceeded {MAX_TOOL_STEPS} steps");
    let fallback = "I need a moment to think about that. Let me get back to you.";
    db.save_message("assistant", fallback, channel_type)?;
    Ok(fallback.to_string())
}

/// Execute a single tool with timeout.
async fn execute_tool(
    tools: &ToolRegistry,
    name: &str,
    input: serde_json::Value,
    ctx: &ToolContext<'_>,
) -> ToolOutput {
    let Some(tool) = tools.get(name) else {
        warn!(tool = %name, "unknown tool requested");
        return ToolOutput::error(format!("Unknown tool: {name}"));
    };

    match tokio::time::timeout(
        std::time::Duration::from_secs(TOOL_TIMEOUT_SECS),
        tool.execute(input, ctx),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            warn!(tool = %name, error = %e, "tool execution failed");
            ToolOutput::error(format!("Tool error: {e}"))
        }
        Err(_) => {
            warn!(tool = %name, "tool execution timed out");
            ToolOutput::error(format!("Tool '{name}' timed out after {TOOL_TIMEOUT_SECS}s"))
        }
    }
}
