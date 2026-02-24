use anyhow::{Context, Result};
use mika_common::claude::{
    ClaudeClient, ContentBlock, Message, MessageContent, MessagesRequest, StopReason,
};
use std::path::Path;
use std::sync::atomic::AtomicU32;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::compaction;
use crate::db::Database;
use crate::messaging::MessageSender;
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
        Ok(Ok(ref response)) => {
            // Post-turn compaction: summarize old messages if threshold exceeded.
            // Runs inline (not spawned) — acceptable latency for Phase 1 CLI.
            if let Err(e) = compaction::maybe_compact(params.db, params.claude).await {
                warn!(error = %e, "post-turn compaction failed");
            }
            Ok(response.clone())
        }
        Ok(Err(e)) => Err(e),
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
    let timezone = db.get_customer_config("timezone")?;

    let prompt_ctx = prompt::PromptContext {
        soul_content: &soul_content,
        identity: &identity,
        core_memory: &core_memory,
        is_onboarding: params.is_onboarding,
        current_utc: chrono::Utc::now(),
        timezone,
    };
    let mut system = prompt::build_system_prompt(&prompt_ctx);

    // Inject conversation summary into system prompt if one exists
    if let Some(summary) = db.load_conversation_summary()? {
        system.push_str("\n## Conversation Summary\n");
        system.push_str("Summary of earlier conversation (older messages have been compacted):\n");
        system.push_str(&summary.content);
        system.push('\n');
    }

    let history = db.load_recent_messages(20, Some(&["cli", "telegram"]))?;
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
        message_sender: None,
    };

    // Build the request once; only messages changes between iterations.
    // send_message takes a reference, so we push new messages directly onto
    // the original to avoid rebuilding system (~4KB) and tool_defs each iteration.
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
            .send_message(&request)
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
                process_tool_calls(response.content, tools, &tool_ctx, &mut request)
                    .await;
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

/// Execute tool-use blocks from a response and push both assistant and
/// tool-result messages onto the request. Shared between conversation and
/// silent agent loops.
async fn process_tool_calls(
    response_content: Vec<ContentBlock>,
    tools: &ToolRegistry,
    tool_ctx: &ToolContext<'_>,
    request: &mut MessagesRequest,
) {
    let mut tool_results = Vec::new();
    for block in &response_content {
        if let ContentBlock::ToolUse { id, name, input } = block {
            debug!(tool = %name, "executing tool");
            let output = execute_tool(tools, name, input.clone(), tool_ctx).await;
            tool_results.push(ContentBlock::ToolResult {
                tool_use_id: id.clone(),
                content: output.content,
                is_error: if output.is_error { Some(true) } else { None },
            });
        }
    }

    request.messages.push(Message {
        role: "assistant".to_string(),
        content: MessageContent::Blocks(response_content),
    });
    request.messages.push(Message {
        role: "user".to_string(),
        content: MessageContent::Blocks(tool_results),
    });
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

// -- Silent Mode Agent Loop --

/// What triggered a silent-mode agent run.
pub enum SilentTrigger {
    Heartbeat,
    Reminder { id: i64, message: String },
}

/// Parameters for running the silent agent loop (heartbeat/reminders).
pub struct SilentAgentParams<'a> {
    pub db: &'a Database,
    pub claude: &'a ClaudeClient,
    pub tools: &'a ToolRegistry,
    pub trigger: SilentTrigger,
    pub home_dir: &'a Path,
    pub session_id: &'a str,
    pub message_sender: Option<&'a dyn MessageSender>,
}

/// Run a silent-mode agent loop for background tasks (heartbeat, reminders).
///
/// Unlike `run_agent`, the agent's text output is NOT delivered to the user.
/// The agent must use `send_message` tool to contact the user.
/// If no `send_message` call is made, the run is a silent no-op.
pub async fn run_silent_agent(params: &SilentAgentParams<'_>) -> Result<()> {
    let channel_type = match &params.trigger {
        SilentTrigger::Heartbeat => "heartbeat",
        SilentTrigger::Reminder { .. } => "reminder",
    };

    let timeout_result = tokio::time::timeout(
        Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS),
        run_silent_inner(params, channel_type),
    )
    .await;

    match timeout_result {
        Ok(result) => {
            // For reminders, mark delivered/failed based on result
            if let SilentTrigger::Reminder { id, .. } = &params.trigger {
                match &result {
                    Ok(_) => params.db.mark_reminder_delivered(*id)?,
                    Err(_) => params.db.mark_reminder_failed(*id)?,
                }
            }
            result
        }
        Err(_elapsed) => {
            warn!(
                timeout_secs = AGENT_TOTAL_TIMEOUT_SECS,
                channel_type, "silent agent timeout exceeded"
            );
            if let SilentTrigger::Reminder { id, .. } = &params.trigger {
                params.db.mark_reminder_failed(*id)?;
            }
            Ok(())
        }
    }
}

async fn run_silent_inner(params: &SilentAgentParams<'_>, channel_type: &str) -> Result<()> {
    let db = params.db;
    let claude = params.claude;
    let tools = params.tools;

    // Build silent-mode system prompt
    let soul_content = std::fs::read_to_string(params.home_dir.join("soul.md")).unwrap_or_default();
    let identity = prompt::load_identity(params.home_dir);
    let core_memory = db.get_all_core_memory()?;
    let pending_commitments = db.list_commitments("pending")?;
    let timezone = db.get_customer_config("timezone")?;

    let trigger_context = match &params.trigger {
        SilentTrigger::Heartbeat => {
            "This is a scheduled HEARTBEAT check-in. Review the user's commitments, \
             upcoming events, and recent context. If there is something timely and \
             worthwhile to share, use send_message. Otherwise, do nothing."
                .to_string()
        }
        SilentTrigger::Reminder { message, .. } => {
            format!(
                "This is a REMINDER firing. The user asked to be reminded:\n\"{message}\"\n\
                 Deliver this reminder using send_message, adding any relevant context."
            )
        }
    };

    let silent_ctx = prompt::SilentPromptContext {
        soul_content: &soul_content,
        identity: &identity,
        core_memory: &core_memory,
        pending_commitments: &pending_commitments,
        trigger_context: &trigger_context,
        current_utc: chrono::Utc::now(),
        timezone,
    };
    let system = prompt::build_silent_prompt(&silent_ctx);

    // For silent mode, provide a brief "trigger" as the user message
    let user_msg = match &params.trigger {
        SilentTrigger::Heartbeat => "[heartbeat trigger]".to_string(),
        SilentTrigger::Reminder { message, .. } => {
            format!("[reminder trigger: {message}]")
        }
    };

    let messages = vec![Message {
        role: "user".to_string(),
        content: MessageContent::Text(user_msg),
    }];

    let tool_defs = tools.definitions();
    let core_memory_edit_count = AtomicU32::new(0);
    let tool_ctx = ToolContext {
        db,
        session_id: params.session_id,
        home_dir: params.home_dir,
        core_memory_edit_count: &core_memory_edit_count,
        is_onboarding: false,
        message_sender: params.message_sender,
    };

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
        debug!(step, channel_type, "silent agent step");

        let response = claude
            .send_message(&request)
            .await
            .context("Claude API call failed in silent mode")?;

        match response.stop_reason {
            StopReason::EndTurn | StopReason::MaxTokens | StopReason::StopSequence => {
                // Save the assistant's internal text (not delivered to user)
                let text = response.text();
                if !text.is_empty() {
                    db.save_message("assistant", &text, channel_type)?;
                }
                info!(step, channel_type, "silent agent done");
                return Ok(());
            }
            StopReason::ToolUse => {
                process_tool_calls(response.content, tools, &tool_ctx, &mut request)
                    .await;
            }
        }
    }

    warn!(channel_type, "silent agent exceeded {MAX_TOOL_STEPS} steps");
    Ok(())
}
