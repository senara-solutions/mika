use anyhow::Result;
use mika_common::claude::{
    ClaudeClient, ContentBlock, Message, MessageContent, MessagesRequest, StopReason,
};
use std::collections::HashMap;
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
use crate::skills::SkillRegistry;
use crate::skills::builtin_handlers;
use crate::skills::executor;
use crate::skills::index::{ResolvedSkillTool, SkillEntry};
use crate::skills::manifest::ToolHandler;
use crate::tools::{ToolContext, ToolOutput, ToolRegistry};
use mika_common::embedding::EmbeddingClient;

const MAX_TOOL_STEPS: usize = 10;
const TOOL_TIMEOUT_SECS: u64 = 30;
const AGENT_TOTAL_TIMEOUT_SECS: u64 = 300;

/// Fallback message sent when the agent completes without producing text output
/// (e.g., all work done via tool calls).
pub const EMPTY_RESPONSE_FALLBACK: &str = "Done.";

/// Output from the agent loop, including text response, thinking, and usage.
pub struct AgentOutput {
    pub text: Option<String>,
    pub thinking: Option<String>,
    pub usage: Option<mika_common::claude::Usage>,
}

// -- Shared helpers --

/// Shared context loaded from the agent's home directory and database.
struct AgentContext {
    soul_content: String,
    identity: prompt::Identity,
    core_memory: Vec<crate::db::CoreMemoryEntry>,
    timezone: Option<String>,
}

async fn load_agent_context(db: &AsyncDatabase, home_dir: &Path) -> Result<AgentContext> {
    let soul_content = tokio::fs::read_to_string(home_dir.join("soul.md"))
        .await
        .unwrap_or_default();
    let identity = prompt::load_identity_async(home_dir).await;
    let core_memory = db.get_all_core_memory().await?;
    let timezone = db.get_customer_config("timezone").await?;
    Ok(AgentContext {
        soul_content,
        identity,
        core_memory,
        timezone,
    })
}

/// Parameterizes behavioral differences between the three agent loop variants.
enum LoopMode<'a> {
    /// Standard conversation: captures thinking, tracks usage, saves to DB, follows up on empty.
    Conversation { channel_type: &'a str },
    /// Silent background task: saves to DB but no thinking/usage/follow-up.
    Silent { channel_type: &'a str },
    /// Team sub-agent: follows up on empty but no thinking/usage/DB saves.
    Team,
}

impl LoopMode<'_> {
    fn is_conversation(&self) -> bool {
        matches!(self, Self::Conversation { .. })
    }

    fn follow_up_on_empty(&self) -> bool {
        matches!(self, Self::Conversation { .. } | Self::Team)
    }

    fn channel_type(&self) -> Option<&str> {
        match self {
            Self::Conversation { channel_type } | Self::Silent { channel_type } => {
                Some(channel_type)
            }
            Self::Team => None,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Conversation { .. } => "agent",
            Self::Silent { .. } => "silent agent",
            Self::Team => "team agent",
        }
    }
}

/// Result from the shared tool-step loop.
struct LoopResult {
    text: Option<String>,
    thinking: Option<String>,
    usage: Option<mika_common::claude::Usage>,
    max_steps_exceeded: bool,
}

/// Shared tool-step loop used by all three agent variants.
///
/// Iterates up to `MAX_TOOL_STEPS`, dispatching tool calls and collecting the
/// final text response. Behavior is parameterized by `LoopMode`.
#[allow(clippy::too_many_arguments)]
async fn run_loop(
    claude: &ClaudeClient,
    tools: &ToolRegistry,
    skill_tool_map: &HashMap<String, &ResolvedSkillTool>,
    skill_timeout: u64,
    tool_ctx: &ToolContext<'_>,
    request: &mut MessagesRequest,
    mode: &LoopMode<'_>,
    db: &AsyncDatabase,
) -> Result<LoopResult> {
    let mut tool_use_occurred = false;
    let mut follow_up_attempted = false;
    let mut last_usage = None;
    let mut thinking_text = None;
    let channel_type = mode.channel_type();

    for step in 0..MAX_TOOL_STEPS {
        debug!(
            step,
            label = mode.label(),
            channel_type,
            messages_len = request.messages.len(),
            "agent loop step"
        );

        let response = claude.send_message(request).await?;

        if mode.is_conversation() {
            last_usage = Some(response.usage.clone());
        }

        if mode.is_conversation() && step == 0 {
            thinking_text = response.thinking();
        }

        match response.stop_reason {
            StopReason::EndTurn | StopReason::MaxTokens | StopReason::StopSequence => {
                let text = response.text();

                if !text.is_empty() {
                    if let Some(ct) = channel_type {
                        db.save_message("assistant", &text, ct).await?;
                    }
                    info!(step, stop_reason = ?response.stop_reason, label = mode.label(), channel_type, "agent done");
                    return Ok(LoopResult {
                        text: Some(text),
                        thinking: thinking_text,
                        usage: last_usage,
                        max_steps_exceeded: false,
                    });
                }

                if !mode.follow_up_on_empty() {
                    info!(step, label = mode.label(), channel_type, "agent done");
                    return Ok(LoopResult {
                        text: None,
                        thinking: None,
                        usage: None,
                        max_steps_exceeded: false,
                    });
                }

                // Tool-only turn with no text: re-prompt once for acknowledgment
                if tool_use_occurred && !follow_up_attempted {
                    follow_up_attempted = true;
                    debug!(
                        step,
                        stop_reason = ?response.stop_reason,
                        label = mode.label(),
                        channel_type,
                        "injecting follow-up after empty tool response"
                    );
                    request.messages.push(Message {
                        role: "assistant".to_string(),
                        content: MessageContent::Blocks(response.content),
                    });
                    request.messages.push(Message {
                        role: "user".to_string(),
                        content: MessageContent::Text(
                            "[Briefly confirm what you just did.]".to_string(),
                        ),
                    });
                    continue;
                }

                if tool_use_occurred {
                    warn!(
                        step,
                        label = mode.label(),
                        channel_type,
                        "agent returned empty text after follow-up"
                    );
                }
                info!(step, stop_reason = ?response.stop_reason, label = mode.label(), channel_type, "agent done");
                return Ok(LoopResult {
                    text: None,
                    thinking: thinking_text,
                    usage: last_usage,
                    max_steps_exceeded: false,
                });
            }
            StopReason::ToolUse => {
                tool_use_occurred = true;
                process_tool_calls(
                    response.content,
                    tools,
                    skill_tool_map,
                    skill_timeout,
                    tool_ctx,
                    request,
                )
                .await;
            }
        }
    }

    warn!(
        label = mode.label(),
        channel_type, "agent exceeded {MAX_TOOL_STEPS} steps"
    );
    Ok(LoopResult {
        text: None,
        thinking: thinking_text,
        usage: last_usage,
        max_steps_exceeded: true,
    })
}

// -- Conversation Agent Loop --

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
    /// Extended thinking configuration (None = disabled).
    pub thinking: Option<mika_common::claude::ThinkingConfig>,
    /// User-attached images to include with the message.
    pub user_images: &'a [mika_common::claude::ImageSource],
}

/// Run the agent loop for a single inbound message.
/// Returns `AgentOutput` with text response, thinking, and usage info.
pub async fn run_agent(params: &AgentParams<'_>) -> Result<AgentOutput> {
    // Save the user message (with image annotation if images attached)
    let save_text = if params.user_images.is_empty() {
        params.user_message.to_string()
    } else {
        format!(
            "[{} image(s) attached]\n{}",
            params.user_images.len(),
            params.user_message
        )
    };
    params
        .db
        .save_message("user", &save_text, params.channel_type)
        .await?;

    let timeout_result = tokio::time::timeout(
        Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS),
        run_agent_inner(params),
    )
    .await;

    match timeout_result {
        Ok(Ok(output)) => {
            // Post-turn compaction: summarize old messages if threshold exceeded.
            // Runs inline (not spawned) — acceptable latency for CLI mode.
            // Server mode sets skip_compaction=true and spawns compaction outside the agent lock.
            if !params.skip_compaction
                && let Err(e) = compaction::maybe_compact(params.db, params.claude).await
            {
                warn!(error = %e, "post-turn compaction failed");
            }
            Ok(output)
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
            Ok(AgentOutput {
                text: Some(fallback.to_string()),
                thinking: None,
                usage: None,
            })
        }
    }
}

/// Inner agent loop, separated so the outer function can wrap it in a timeout.
async fn run_agent_inner(params: &AgentParams<'_>) -> Result<AgentOutput> {
    let db = params.db;
    let claude = params.claude;
    let tools = params.tools;
    let channel_type = params.channel_type;

    let ctx = load_agent_context(db, params.home_dir).await?;

    let chat_id = db.get_customer_config("chat_id").await?;
    let prompt_ctx = prompt::PromptContext {
        soul_content: &ctx.soul_content,
        identity: &ctx.identity,
        core_memory: &ctx.core_memory,
        is_onboarding: params.is_onboarding,
        current_utc: chrono::Utc::now(),
        timezone: ctx.timezone,
        global_home_dir: Some(params.home_dir),
        channel_type: Some(params.channel_type),
        telegram_configured: chat_id.is_some(),
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
    let skill_tool_defs = inject_skills_and_resolve_tools(&matched, tools, &mut system);
    let skill_tool_map = build_skill_tool_map(&matched);
    let skill_timeout = max_skill_timeout(&matched);

    let history = db.load_recent_messages(20, None).await?;

    // Build initial message list from history.
    // The last message in history is the user message we just saved.
    // If user_images is non-empty, replace the last message with a multi-block version.
    let mut messages: Vec<Message> = history
        .iter()
        .map(|msg| Message {
            role: msg.role.clone(),
            content: MessageContent::Text(msg.content.clone()),
        })
        .collect();

    // Attach images to the last user message if present
    if let Some(last) = messages
        .last_mut()
        .filter(|m| m.role == "user" && !params.user_images.is_empty())
    {
        let text = match &last.content {
            MessageContent::Text(t) => t.clone(),
            MessageContent::Blocks(_) => String::new(),
        };
        let mut blocks: Vec<ContentBlock> = params
            .user_images
            .iter()
            .map(|img| ContentBlock::Image {
                source: img.clone(),
            })
            .collect();
        blocks.push(ContentBlock::Text { text });
        last.content = MessageContent::Blocks(blocks);
    }

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

    // Auto-adjust max_tokens when thinking is enabled
    let max_tokens = if let Some(mika_common::claude::ThinkingConfig::Enabled { budget_tokens }) =
        &params.thinking
    {
        claude.max_tokens.max(budget_tokens.saturating_add(4096))
    } else {
        claude.max_tokens
    };

    let mut request = MessagesRequest {
        model: claude.model.clone(),
        max_tokens,
        system: Some(system),
        messages,
        tools: if skill_tool_defs.is_empty() {
            None
        } else {
            Some(skill_tool_defs)
        },
        thinking: params.thinking.clone(),
    };

    let mode = LoopMode::Conversation { channel_type };
    let result = run_loop(
        claude,
        tools,
        &skill_tool_map,
        skill_timeout,
        &tool_ctx,
        &mut request,
        &mode,
        db,
    )
    .await?;

    if result.max_steps_exceeded {
        let fallback = "I need a moment to think about that. Let me get back to you.";
        db.save_message("assistant", fallback, channel_type).await?;
        return Ok(AgentOutput {
            text: Some(fallback.to_string()),
            thinking: result.thinking,
            usage: result.usage,
        });
    }

    Ok(AgentOutput {
        text: result.text,
        thinking: result.thinking,
        usage: result.usage,
    })
}

/// Execute tool-use blocks from a response and push both assistant and
/// tool-result messages onto the request. Shared between conversation and
/// silent agent loops.
async fn process_tool_calls(
    response_content: Vec<ContentBlock>,
    tools: &ToolRegistry,
    skill_tools: &HashMap<String, &ResolvedSkillTool>,
    skill_timeout: u64,
    tool_ctx: &ToolContext<'_>,
    request: &mut MessagesRequest,
) {
    let mut tool_results = Vec::new();
    for block in &response_content {
        if let ContentBlock::ToolUse { id, name, input } = block {
            debug!(tool = %name, "executing tool");
            let output = execute_tool(
                tools,
                skill_tools,
                name,
                input.clone(),
                tool_ctx,
                skill_timeout,
            )
            .await;
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
///
/// Routing: builtin tools (from ToolRegistry) first, then skill-defined tools,
/// then "unknown tool" error.
async fn execute_tool(
    tools: &ToolRegistry,
    skill_tools: &HashMap<String, &ResolvedSkillTool>,
    name: &str,
    input: serde_json::Value,
    ctx: &ToolContext<'_>,
    skill_timeout: u64,
) -> ToolOutput {
    // 1. Try builtin tool
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

    // 2. Try skill-defined tool
    if let Some(skill_tool) = skill_tools.get(name) {
        // Builtin skill handlers dispatch to Rust functions with ToolContext access
        if let ToolHandler::Builtin { function } = &skill_tool.handler {
            return match tokio::time::timeout(
                std::time::Duration::from_secs(TOOL_TIMEOUT_SECS),
                builtin_handlers::execute(function, input, ctx),
            )
            .await
            {
                Ok(output) => output,
                Err(_) => {
                    warn!(tool = %name, "builtin handler timed out");
                    ToolOutput::error(format!(
                        "Builtin tool '{name}' timed out after {TOOL_TIMEOUT_SECS}s"
                    ))
                }
            };
        }
        return executor::execute_skill_tool(skill_tool, input, skill_timeout).await;
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

    let ctx = load_agent_context(db, params.home_dir).await?;
    let pending_commitments = db.list_commitments("pending").await?;

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

    let chat_id = db.get_customer_config("chat_id").await?;
    let silent_ctx = prompt::SilentPromptContext {
        soul_content: &ctx.soul_content,
        identity: &ctx.identity,
        core_memory: &ctx.core_memory,
        pending_commitments: &pending_commitments,
        trigger_context: &trigger_context,
        current_utc: chrono::Utc::now(),
        timezone: ctx.timezone,
        telegram_configured: chat_id.is_some(),
    };
    let mut system = prompt::build_silent_prompt(&silent_ctx);

    // Match skills: heartbeat uses always-on skills directly (no fake trigger text),
    // reminders use keyword matching against the reminder message.
    let matched = match &params.trigger {
        SilentTrigger::Heartbeat => params.skills.always_on_skills(),
        SilentTrigger::Reminder { message, .. } => params.skills.match_message(message),
    };
    let skill_tool_defs = inject_skills_and_resolve_tools(&matched, tools, &mut system);
    let skill_tool_map = build_skill_tool_map(&matched);
    let skill_timeout = max_skill_timeout(&matched);

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
        thinking: None,
    };

    let mode = LoopMode::Silent { channel_type };
    run_loop(
        claude,
        tools,
        &skill_tool_map,
        skill_timeout,
        &tool_ctx,
        &mut request,
        &mode,
        db,
    )
    .await?;

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
/// - Returns `Some(text)` when the assistant produced a text response,
///   or `None` for tool-use-only turns.
pub async fn run_team_agent(params: &TeamAgentParams<'_>) -> Result<Option<String>> {
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
            Ok(Some(
                "Agent timed out while processing team task.".to_string(),
            ))
        }
    }
}

async fn run_team_agent_inner(params: &TeamAgentParams<'_>) -> Result<Option<String>> {
    let claude = params.claude;
    let tools = params.tools;

    let ctx = load_agent_context(params.db, params.home_dir).await?;

    let prompt_ctx = prompt::PromptContext {
        soul_content: &ctx.soul_content,
        identity: &ctx.identity,
        core_memory: &ctx.core_memory,
        is_onboarding: false,
        current_utc: chrono::Utc::now(),
        timezone: ctx.timezone,
        global_home_dir: None, // Team agents don't need team discovery in their prompt
        channel_type: None,
        telegram_configured: false,
    };
    let mut system = prompt::build_system_prompt(&prompt_ctx);

    // Inject team context after the base system prompt
    system.push_str("\n## Team Context\n");
    system.push_str(params.team_context);
    system.push('\n');

    // Match skills and resolve tool definitions
    let matched = params.skills.match_message(params.task_message);
    let skill_tool_defs = inject_skills_and_resolve_tools(&matched, tools, &mut system);
    let skill_tool_map = build_skill_tool_map(&matched);
    let skill_timeout = max_skill_timeout(&matched);

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
        thinking: None,
    };

    let mode = LoopMode::Team;
    let result = run_loop(
        claude,
        tools,
        &skill_tool_map,
        skill_timeout,
        &tool_ctx,
        &mut request,
        &mode,
        params.db,
    )
    .await?;

    if result.max_steps_exceeded {
        return Ok(Some("Agent exceeded maximum tool steps.".to_string()));
    }

    Ok(result.text)
}

// -- Skill helpers --

/// Build a lookup map from tool name → ResolvedSkillTool for matched skills.
fn build_skill_tool_map<'a>(matched: &[&'a SkillEntry]) -> HashMap<String, &'a ResolvedSkillTool> {
    matched
        .iter()
        .flat_map(|e| e.skill_tools.iter())
        .map(|st| (st.definition.name.clone(), st))
        .collect()
}

/// Compute the maximum timeout across matched skills (for skill tool execution).
/// Falls back to TOOL_TIMEOUT_SECS if no skills matched.
fn max_skill_timeout(matched: &[&SkillEntry]) -> u64 {
    matched
        .iter()
        .map(|e| e.manifest.skill.timeout_secs)
        .max()
        .unwrap_or(TOOL_TIMEOUT_SECS)
}

/// Inject matched skill prompt snippets into the system prompt and resolve
/// tool definitions. Always includes all builtin tools plus skill-defined tools.
fn inject_skills_and_resolve_tools(
    matched: &[&SkillEntry],
    tools: &ToolRegistry,
    system: &mut String,
) -> Vec<mika_common::claude::ToolDefinition> {
    // Always include ALL builtin tools
    let mut tool_defs = tools.definitions().to_vec();
    let mut seen: std::collections::HashSet<String> =
        tool_defs.iter().map(|d| d.name.clone()).collect();

    // Add skill prompt snippets and skill-defined tools from matched skills
    for entry in matched {
        if !entry.prompt_snippet.is_empty() {
            write!(
                system,
                "\n<context type=\"skill\" trust=\"local\">\n## {} Skill\n{}\n</context>\n",
                entry.manifest.skill.name, entry.prompt_snippet
            )
            .unwrap();
        }
        for st in &entry.skill_tools {
            if seen.insert(st.definition.name.clone()) {
                tool_defs.push(st.definition.clone());
            }
        }
    }

    tool_defs
}
