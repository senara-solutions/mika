use anyhow::Result;
use mika_common::claude::{
    ClaudeClient, ContentBlock, Message, MessageContent, MessagesRequest, StopReason,
};
use std::fmt::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;
use crate::compaction;
use crate::messaging::MessageSender;
use crate::prompt;
use crate::skills::index::SkillEntry;
use crate::skills::{self, SkillRegistry};
use crate::tools::{ToolContext, ToolOutput, ToolRegistry};
use mika_common::embedding::EmbeddingClient;

const MAX_TOOL_STEPS: usize = 10;
const TOOL_TIMEOUT_SECS: u64 = 30;
const AGENT_TOTAL_TIMEOUT_SECS: u64 = 300;

/// Check if this is a new user (user_summary still at default value).
/// Used by both CLI and server to set `is_onboarding` on agent params.
pub async fn check_onboarding(db: &AsyncDatabase) -> bool {
    let default = crate::db::CORE_MEMORY_SECTIONS
        .iter()
        .find(|(k, _)| *k == "user_summary")
        .map(|(_, v)| *v)
        .unwrap_or("New user. No information yet.");
    db.get_core_memory("user_summary")
        .await
        .ok()
        .flatten()
        .map(|e| e.value == default)
        .unwrap_or(true)
}

/// Parameters for running the agent loop.
pub struct AgentParams<'a> {
    pub db: &'a AsyncDatabase,
    pub claude: &'a ClaudeClient,
    pub tools: &'a ToolRegistry,
    pub skills: &'a SkillRegistry,
    pub user_message: &'a str,
    pub channel_type: &'a str,
    pub session_id: &'a str,
    pub home_dir: &'a Path,
    pub is_onboarding: bool,
    pub message_sender: Option<Arc<dyn MessageSender>>,
    /// When true, skip inline post-turn compaction (server mode spawns it separately).
    pub skip_compaction: bool,
    /// Optional embedding client for Layer 3 vector search.
    pub embedding_client: Option<&'a EmbeddingClient>,
}

/// Run the agent loop for a single inbound message.
/// Returns the assistant's text response.
pub async fn run_agent(params: &AgentParams<'_>) -> Result<String> {
    // Save the user message
    params
        .db
        .save_message("user", params.user_message, params.channel_type)
        .await?;

    let timeout_result = tokio::time::timeout(
        Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS),
        run_agent_inner(params),
    )
    .await;

    match timeout_result {
        Ok(Ok(response)) => {
            // Post-turn compaction: summarize old messages if threshold exceeded.
            // Runs inline (not spawned) — acceptable latency for CLI mode.
            // Server mode sets skip_compaction=true and spawns compaction outside the agent lock.
            if !params.skip_compaction {
                if let Err(e) = compaction::maybe_compact(params.db, params.claude).await {
                    warn!(error = %e, "post-turn compaction failed");
                }
            }
            Ok(response)
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
                .save_message("assistant", fallback, params.channel_type)
                .await?;
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
    let soul_content = tokio::fs::read_to_string(params.home_dir.join("soul.md"))
        .await
        .unwrap_or_default();
    let identity = prompt::load_identity_async(params.home_dir).await;
    let core_memory = db.get_all_core_memory().await?;
    let timezone = db.get_customer_config("timezone").await?;

    let prompt_ctx = prompt::PromptContext {
        soul_content: &soul_content,
        identity: &identity,
        core_memory: &core_memory,
        is_onboarding: params.is_onboarding,
        current_utc: chrono::Utc::now(),
        timezone,
        global_home_dir: Some(params.home_dir),
    };
    let mut system = prompt::build_system_prompt(&prompt_ctx);

    // Inject conversation summary into system prompt if one exists
    if let Some(summary) = db.load_conversation_summary().await? {
        system.push_str("\n## Conversation Summary\n");
        system.push_str("<context type=\"summary\" trust=\"data\">\n");
        system.push_str(&summary.content);
        system.push_str("\n</context>\n");
    }

    // Match skills and resolve tool definitions
    let matched = params.skills.match_message(params.user_message);
    let skill_tool_defs =
        inject_skills_and_resolve_tools(&matched, params.skills, tools, &mut system);

    let history = db.load_recent_messages(20, None).await?;

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
        message_sender: params.message_sender.clone(),
        embedding_client: params.embedding_client,
    };

    // Build the request once; only messages changes between iterations.
    // send_message takes a reference, so we push new messages directly onto
    // the original to avoid rebuilding system (~4KB) and tool_defs each iteration.
    let mut request = MessagesRequest {
        model: claude.model.clone(),
        max_tokens: claude.max_tokens,
        system: Some(system),
        messages,
        tools: if skill_tool_defs.is_empty() {
            None
        } else {
            Some(skill_tool_defs)
        },
    };

    for step in 0..MAX_TOOL_STEPS {
        debug!(
            step,
            messages_len = request.messages.len(),
            "agent loop step"
        );

        let response = claude.send_message(&request).await?;

        match response.stop_reason {
            StopReason::EndTurn | StopReason::MaxTokens => {
                let text = response.text();
                if !text.is_empty() {
                    db.save_message("assistant", &text, channel_type).await?;
                } else if step > 0 {
                    warn!(step, stop_reason = ?response.stop_reason, "agent returned empty text after tool use");
                }
                info!(step, stop_reason = ?response.stop_reason, "agent done");
                return Ok(text);
            }
            StopReason::ToolUse => {
                process_tool_calls(response.content, tools, &tool_ctx, &mut request).await;
            }
            StopReason::StopSequence => {
                let text = response.text();
                if !text.is_empty() {
                    db.save_message("assistant", &text, channel_type).await?;
                } else if step > 0 {
                    warn!(
                        step,
                        "agent returned empty text on StopSequence after tool use"
                    );
                }
                return Ok(text);
            }
        }
    }

    // Exceeded max steps
    warn!("agent loop exceeded {MAX_TOOL_STEPS} steps");
    let fallback = "I need a moment to think about that. Let me get back to you.";
    db.save_message("assistant", fallback, channel_type).await?;
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
    if let Some(tool) = tools.get(name) {
        return match tokio::time::timeout(
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
        };
    }

    warn!(tool = %name, "unknown tool requested");
    ToolOutput::error(format!("Unknown tool: {name}"))
}

// -- Silent Mode Agent Loop --

/// What triggered a silent-mode agent run.
pub enum SilentTrigger {
    Heartbeat,
    Reminder { id: i64, message: String },
}

/// Parameters for running the silent agent loop (heartbeat/reminders).
pub struct SilentAgentParams<'a> {
    pub db: &'a AsyncDatabase,
    pub claude: &'a ClaudeClient,
    pub tools: &'a ToolRegistry,
    pub skills: &'a SkillRegistry,
    pub trigger: SilentTrigger,
    pub home_dir: &'a Path,
    pub session_id: &'a str,
    pub message_sender: Option<Arc<dyn MessageSender>>,
    pub embedding_client: Option<&'a EmbeddingClient>,
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
                    Ok(_) => params.db.mark_reminder_delivered(*id).await?,
                    Err(_) => params.db.mark_reminder_failed(*id).await?,
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
                params.db.mark_reminder_failed(*id).await?;
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
    let soul_content = tokio::fs::read_to_string(params.home_dir.join("soul.md"))
        .await
        .unwrap_or_default();
    let identity = prompt::load_identity_async(params.home_dir).await;
    let core_memory = db.get_all_core_memory().await?;
    let pending_commitments = db.list_commitments("pending").await?;
    let timezone = db.get_customer_config("timezone").await?;

    let trigger_context = match &params.trigger {
        SilentTrigger::Heartbeat => {
            "This is a scheduled HEARTBEAT check-in. Review the user's commitments, \
             upcoming events, and recent context. If there is something timely and \
             worthwhile to share, use send_message. Otherwise, do nothing."
                .to_string()
        }
        SilentTrigger::Reminder { message, .. } => {
            format!(
                "This is a REMINDER firing. The user asked to be reminded:\n\
                 <reminder-data>{message}</reminder-data>\n\
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
    let mut system = prompt::build_silent_prompt(&silent_ctx);

    // Match skills: heartbeat uses always-on skills directly (no fake trigger text),
    // reminders use keyword matching against the reminder message.
    let matched = match &params.trigger {
        SilentTrigger::Heartbeat => params.skills.always_on_skills(),
        SilentTrigger::Reminder { message, .. } => params.skills.match_message(message),
    };
    let skill_tool_defs =
        inject_skills_and_resolve_tools(&matched, params.skills, tools, &mut system);

    // For silent mode, provide a brief "trigger" as the user message
    let user_msg = match &params.trigger {
        SilentTrigger::Heartbeat => "[heartbeat trigger]".to_string(),
        SilentTrigger::Reminder { message, .. } => {
            format!("[reminder trigger: <reminder-data>{message}</reminder-data>]")
        }
    };

    let messages = vec![Message {
        role: "user".to_string(),
        content: MessageContent::Text(user_msg),
    }];

    let core_memory_edit_count = AtomicU32::new(0);
    let tool_ctx = ToolContext {
        db,
        session_id: params.session_id,
        home_dir: params.home_dir,
        core_memory_edit_count: &core_memory_edit_count,
        is_onboarding: false,
        message_sender: params.message_sender.clone(),
        embedding_client: params.embedding_client,
    };

    let mut request = MessagesRequest {
        model: claude.model.clone(),
        max_tokens: claude.max_tokens,
        system: Some(system),
        messages,
        tools: if skill_tool_defs.is_empty() {
            None
        } else {
            Some(skill_tool_defs)
        },
    };

    for step in 0..MAX_TOOL_STEPS {
        debug!(step, channel_type, "silent agent step");

        let response = claude.send_message(&request).await?;

        match response.stop_reason {
            StopReason::EndTurn | StopReason::MaxTokens | StopReason::StopSequence => {
                // Save the assistant's internal text (not delivered to user)
                let text = response.text();
                if !text.is_empty() {
                    db.save_message("assistant", &text, channel_type).await?;
                }
                info!(step, channel_type, "silent agent done");
                return Ok(());
            }
            StopReason::ToolUse => {
                process_tool_calls(response.content, tools, &tool_ctx, &mut request).await;
            }
        }
    }

    warn!(channel_type, "silent agent exceeded {MAX_TOOL_STEPS} steps");
    Ok(())
}

// -- Team Agent Loop --

/// Parameters for running an agent within a team execution context.
pub struct TeamAgentParams<'a> {
    pub db: &'a AsyncDatabase,
    pub claude: &'a ClaudeClient,
    pub tools: &'a ToolRegistry,
    pub skills: &'a SkillRegistry,
    pub home_dir: &'a Path,
    pub task_message: &'a str,
    pub team_context: &'a str,
    pub session_id: &'a str,
    pub embedding_client: Option<&'a EmbeddingClient>,
}

/// Run an agent within a team execution context.
///
/// This is a simplified variant of `run_agent_inner` that:
/// - Loads soul, identity, core_memory from the agent's home (retains personality)
/// - Injects team_context into the system prompt after identity
/// - Uses a single-turn message (just the task — no conversation history)
/// - Does NOT save messages to DB and does NOT run compaction
/// - Returns the assistant's text response
pub async fn run_team_agent(params: &TeamAgentParams<'_>) -> Result<String> {
    let timeout_result = tokio::time::timeout(
        Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS),
        run_team_agent_inner(params),
    )
    .await;

    match timeout_result {
        Ok(result) => result,
        Err(_elapsed) => {
            warn!(
                timeout_secs = AGENT_TOTAL_TIMEOUT_SECS,
                "team agent loop total timeout exceeded"
            );
            Ok("Agent timed out while processing team task.".to_string())
        }
    }
}

async fn run_team_agent_inner(params: &TeamAgentParams<'_>) -> Result<String> {
    let claude = params.claude;
    let tools = params.tools;

    // Load context from agent's home
    let soul_content = tokio::fs::read_to_string(params.home_dir.join("soul.md"))
        .await
        .unwrap_or_default();
    let identity = prompt::load_identity_async(params.home_dir).await;
    let core_memory = params.db.get_all_core_memory().await?;
    let timezone = params.db.get_customer_config("timezone").await?;

    let prompt_ctx = prompt::PromptContext {
        soul_content: &soul_content,
        identity: &identity,
        core_memory: &core_memory,
        is_onboarding: false,
        current_utc: chrono::Utc::now(),
        timezone,
        global_home_dir: None, // Team agents don't need team discovery in their prompt
    };
    let mut system = prompt::build_system_prompt(&prompt_ctx);

    // Inject team context after the base system prompt
    system.push_str("\n## Team Context\n");
    system.push_str(params.team_context);
    system.push('\n');

    // Match skills and resolve tool definitions
    let matched = params.skills.match_message(params.task_message);
    let skill_tool_defs =
        inject_skills_and_resolve_tools(&matched, params.skills, tools, &mut system);

    // Single-turn: just the task message, no history
    let messages = vec![Message {
        role: "user".to_string(),
        content: MessageContent::Text(params.task_message.to_string()),
    }];

    let core_memory_edit_count = AtomicU32::new(0);
    let tool_ctx = ToolContext {
        db: params.db,
        session_id: params.session_id,
        home_dir: params.home_dir,
        core_memory_edit_count: &core_memory_edit_count,
        is_onboarding: false,
        message_sender: None,
        embedding_client: params.embedding_client,
    };

    let mut request = MessagesRequest {
        model: claude.model.clone(),
        max_tokens: claude.max_tokens,
        system: Some(system),
        messages,
        tools: if skill_tool_defs.is_empty() {
            None
        } else {
            Some(skill_tool_defs)
        },
    };

    for step in 0..MAX_TOOL_STEPS {
        debug!(step, "team agent step");

        let response = claude.send_message(&request).await?;

        match response.stop_reason {
            StopReason::EndTurn | StopReason::MaxTokens | StopReason::StopSequence => {
                let text = response.text();
                info!(step, "team agent done");
                return Ok(text);
            }
            StopReason::ToolUse => {
                process_tool_calls(response.content, tools, &tool_ctx, &mut request).await;
            }
        }
    }

    warn!("team agent exceeded {MAX_TOOL_STEPS} steps");
    Ok("Agent exceeded maximum tool steps.".to_string())
}

/// Inject matched skill prompt snippets into the system prompt and resolve
/// tool definitions. Shared between conversation and silent agent loops.
fn inject_skills_and_resolve_tools(
    matched: &[&SkillEntry],
    skills: &SkillRegistry,
    tools: &ToolRegistry,
    system: &mut String,
) -> Vec<mika_common::claude::ToolDefinition> {
    if !matched.is_empty() {
        for entry in matched {
            if !entry.prompt_snippet.is_empty() {
                write!(
                    system,
                    "\n<context type=\"skill\" trust=\"local\">\n## {} Skill\n{}\n</context>\n",
                    entry.manifest.name, entry.prompt_snippet
                )
                .unwrap();
            }
        }
        skills::resolve_matched_skills(tools, matched)
    } else if !skills.has_skills() {
        // Fallback: no skills dir → use all builtin tools (pre-skill behavior)
        tools.definitions().to_vec()
    } else {
        Vec::new()
    }
}
