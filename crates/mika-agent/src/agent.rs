use anyhow::{Context, Result};
use mika_common::claude::{
    ClaudeClient, ContentBlock, Message, MessageContent, MessagesRequest, StopReason,
};
use std::path::Path;
use std::sync::atomic::AtomicU32;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::db::Database;
use crate::prompt;
use crate::tools::{ToolContext, ToolOutput, ToolRegistry};

const MAX_TOOL_STEPS: usize = 10;
const TOOL_TIMEOUT_SECS: u64 = 30;
const AGENT_TOTAL_TIMEOUT_SECS: u64 = 300;

/// Parameters for running the agent loop.
pub struct AgentParams<'a> {
    pub db: &'a Database,
    pub claude: &'a ClaudeClient,
    pub tools: &'a ToolRegistry,
    pub user_message: &'a str,
    pub channel_type: &'a str,
    pub session_id: &'a str,
    pub home_dir: &'a Path,
    pub is_onboarding: bool,
}

/// Run the agent loop for a single inbound message.
/// Returns the assistant's text response.
pub async fn run_agent(params: &AgentParams<'_>) -> Result<String> {
    // Save the user message
    params
        .db
        .save_message("user", params.user_message, params.channel_type)?;

    let timeout_result = tokio::time::timeout(
        Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS),
        run_agent_inner(params),
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
            params
                .db
                .save_message("assistant", fallback, params.channel_type)?;
            Ok(fallback.to_string())
        }
    }
}

/// Inner agent loop, separated so the outer function can wrap it in a timeout.
async fn run_agent_inner(params: &AgentParams<'_>) -> Result<String> {
    let db = params.db;
    let claude = params.claude;
    let tools = params.tools;
    let channel_type = params.channel_type;

    // Load context: soul, identity, core memory → build system prompt
    let soul_content = std::fs::read_to_string(params.home_dir.join("soul.md")).unwrap_or_default();
    let identity = prompt::load_identity(params.home_dir);
    let core_memory = db.get_all_core_memory()?;

    let prompt_ctx = prompt::PromptContext {
        soul_content: &soul_content,
        identity: &identity,
        core_memory: &core_memory,
        is_onboarding: params.is_onboarding,
    };
    let system = prompt::build_system_prompt(&prompt_ctx);

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

    let core_memory_edit_count = AtomicU32::new(0);
    let tool_ctx = ToolContext {
        db,
        session_id: params.session_id,
        home_dir: params.home_dir,
        core_memory_edit_count: &core_memory_edit_count,
        is_onboarding: params.is_onboarding,
    };

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
        debug!(
            step,
            messages_len = request.messages.len(),
            "agent loop step"
        );

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
            ToolOutput::error(format!(
                "Tool '{name}' timed out after {TOOL_TIMEOUT_SECS}s"
            ))
        }
    }
}
