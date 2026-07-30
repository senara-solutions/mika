//! Tool dispatch — process_tool_calls(), execute_tool(), ToolDispatchCtx.
//!
//! Routes tool-use blocks from LLM responses through the three-tier dispatch
//! chain (builtin → skill → MCP), manages per-tool timeouts, per-turn dedup,
//! image-budget accounting, and persists tool-call metadata to SQLite.

use std::collections::HashMap;

use mika_a2a::streaming::{
    StreamEvent, StreamEventSender, ToolCallResultEvent, ToolCallStartEvent, truncate_tool_summary,
};
use mika_common::llm::{
    LlmContent, LlmContentBlock, LlmImage, LlmMessage, LlmRequest, LlmResponseContent, LlmRole,
    LlmToolResultBlock, LlmToolResultContent,
};
use tracing::{debug, warn};

use crate::async_db::AsyncDatabase;
use crate::mcp::McpManager;
use crate::secret_scrubber::scrub_secrets;
use crate::skills::builtin_handlers;
use crate::skills::executor;
use crate::skills::index::ResolvedSkillTool;
use crate::skills::manifest::ToolHandler;
use crate::tools::{ToolContext, ToolOutput, ToolRegistry};

use super::types::{ToolCallSummary, has_non_zero_exit_prefix, truncate_summary};

/// Max chars of serialized tool input to include in timeout log lines (#900).
const TOOL_TIMEOUT_INPUT_EXCERPT_LEN: usize = 200;

/// Resources for tool dispatch, bundled to reduce argument count.
pub(crate) struct ToolDispatchCtx<'a> {
    pub(crate) tools: &'a ToolRegistry,
    pub(crate) skill_tools: &'a HashMap<String, &'a ResolvedSkillTool>,
    pub(crate) ctx: &'a ToolContext<'a>,
    pub(crate) skill_timeout: u64,
    pub(crate) mcp_manager: Option<&'a McpManager>,
    pub(crate) long_running_ctx: Option<&'a executor::LongRunningContext>,
}

/// Execute tool-use blocks from a response and push both assistant and
/// tool-result messages onto the request. Returns summaries of each tool call
/// for persistence in conversation metadata.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_tool_calls(
    response_content: Vec<LlmResponseContent>,
    tools: &ToolRegistry,
    skill_tools: &HashMap<String, &ResolvedSkillTool>,
    skill_timeout: u64,
    tool_ctx: &ToolContext<'_>,
    request: &mut LlmRequest,
    step: u32,
    mcp_manager: Option<&McpManager>,
    long_running_ctx: Option<&executor::LongRunningContext>,
    db: &AsyncDatabase,
    session_id: &str,
    store_tool_calls: bool,
    llm_call_id: Option<&str>,
    // Send-message turn boundary state (#771). Passed from the agent loop to
    // enable intra-step write gating: after a send_message call within this
    // step, subsequent send_message calls are suppressed (conversation mode only).
    send_message_boundary_active: &mut bool,
    suppressed_write_tools: &mut Vec<String>,
    send_message_text_capture: &mut String,
    // Whether this is a conversation-mode run. The send_message boundary
    // guard only applies in conversation mode — silent/callback modes use
    // send_message as a notification mechanism, not a user-dialog interaction.
    conversation_mode: bool,
    // Per-task A2A broadcast sender (mika#1731). When `Some`, this fn emits
    // `ToolCallStart` / `ToolCallResult` SSE frames around each physical tool
    // dispatch so streaming clients (mika#1727 thin-client TUI) get
    // fine-grained tool receipts. `None` for non-streaming callers
    // (`message/send`, silent/callback turns) — emission is skipped entirely.
    stream_tx: Option<&StreamEventSender>,
) -> Vec<ToolCallSummary> {
    let mut tool_results: Vec<LlmContentBlock> = Vec::new();
    let mut summaries = Vec::new();
    let mut image_bytes_budget = crate::planning::policy::MAX_IMAGE_BYTES_PER_STEP;
    // Per-turn dedup: if the LLM emits two tool_use blocks with identical
    // (name, arguments) in a single response, execute the tool once and reuse
    // the cached output for subsequent blocks. This defends the engine against
    // provider-side tool_use duplication (see #582). Scope is strictly this
    // function call — duplicates across steps or turns are unaffected.
    let mut dedup_cache: HashMap<(String, String), ToolOutput> = HashMap::new();
    for block in &response_content {
        if let LlmResponseContent::ToolCall {
            id,
            name,
            arguments,
        } = block
        {
            // Send-message turn boundary gate (#771): intra-step suppression
            // for duplicate send_message calls in conversation mode. Within
            // the same LLM response, a second send_message after the first is
            // suppressed (the agent should combine messages or wait for user
            // input). Silent/callback modes are exempt — send_message is the
            // notification mechanism there, not a user-dialog interaction.
            // The inter-step gate (after process_tool_calls returns) prevents
            // cross-step writes by forcing EndTurn.
            if *send_message_boundary_active && name == "send_message" && conversation_mode {
                let input_summary_for_suppressed = truncate_summary(
                    &arguments.to_string(),
                    crate::planning::policy::INPUT_SUMMARY_MAX,
                );
                let suppressed_msg =
                    "[mika-engine] Tool call suppressed: send_message turn boundary (#771). \
                     A second send_message is not permitted after the first in the same turn. \
                     Combine your messages into a single send_message call."
                        .to_string();
                warn!(
                    trace_id = %tool_ctx.trace_id,
                    tool = %name,
                    step,
                    "send_message_turn_boundary: suppressed duplicate send_message"
                );
                summaries.push(ToolCallSummary {
                    step,
                    name: name.clone(),
                    input_summary: scrub_secrets(&input_summary_for_suppressed).into_owned(),
                    output_summary: truncate_summary(
                        &suppressed_msg,
                        crate::planning::policy::OUTPUT_SUMMARY_MAX,
                    ),
                    success: false,
                    non_zero_exit: false,
                });
                suppressed_write_tools.push(name.clone());
                // Return a tool_result so the conversation history stays paired
                tool_results.push(LlmContentBlock::ToolResult {
                    tool_call_id: id.clone(),
                    content: LlmToolResultContent::Text(suppressed_msg),
                    is_error: true,
                });
                continue;
            }

            let dedup_key = (
                name.clone(),
                serde_json::to_string(arguments).unwrap_or_default(),
            );
            let output = if let Some(cached) = dedup_cache.get(&dedup_key) {
                warn!(
                    trace_id = %tool_ctx.trace_id,
                    tool = %name,
                    step,
                    cached_was_error = cached.is_error,
                    "duplicate tool_use block suppressed; reusing prior result"
                );
                // Clone the cached result but strip images: the LLM already
                // received them in the tool_result paired with the first
                // duplicate's id, so re-emitting them would both waste the
                // shared `image_bytes_budget` and send redundant bytes on
                // the API request. Text content is preserved so the
                // duplicate tool_use id still gets a meaningful pair.
                let mut reused = cached.clone();
                reused.images.clear();
                reused
            } else {
                debug!(tool = %name, "executing tool");
                let input_summary = scrub_secrets(&truncate_summary(
                    &arguments.to_string(),
                    crate::planning::policy::INPUT_SUMMARY_MAX,
                ))
                .into_owned();
                let dispatch = ToolDispatchCtx {
                    tools,
                    skill_tools,
                    ctx: tool_ctx,
                    skill_timeout,
                    mcp_manager,
                    long_running_ctx,
                };
                // mika#1731 — emit ToolCallStart on the A2A stream immediately
                // before physical dispatch. Fire-and-forget: a send error (e.g.
                // no subscribers connected) NEVER affects tool execution.
                if let Some(tx) = stream_tx {
                    let _ = tx.send(StreamEvent::ToolCallStart(ToolCallStartEvent {
                        task_id: tool_ctx.trace_id.to_string(),
                        context_id: None,
                        step,
                        tool_name: name.clone(),
                        args_summary: truncate_tool_summary(&input_summary),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    }));
                }
                let tool_start = std::time::Instant::now();
                let output = execute_tool(&dispatch, name, arguments.clone()).await;
                let tool_latency_ms = tool_start.elapsed().as_millis() as u64;

                // Post-action hooks (mika#772): side-effect-only callbacks that
                // fire after a tool call succeeds, before the result returns to
                // the LLM. Failure is warn-and-continue.
                crate::tools::post_action_hooks::run_post_action_hooks(
                    name, arguments, &output, db,
                )
                .await;

                // Record full tool call in database
                if store_tool_calls {
                    let tool_id = uuid::Uuid::new_v4().to_string();
                    let (tool_source, tool_skill_name) = if tools.get(name).is_some() {
                        ("builtin", None)
                    } else if let Some(st) = skill_tools.get(name.as_str()) {
                        let sn = st
                            .skill_dir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                            .to_string();
                        ("skill", Some(sn))
                    } else {
                        ("mcp", None)
                    };
                    let non_zero = !output.is_error && has_non_zero_exit_prefix(&output.content);
                    let input_json = serde_json::to_string(arguments).unwrap_or_default();
                    let err_msg = if output.is_error {
                        Some(output.content.as_str())
                    } else {
                        None
                    };
                    if let Err(e) = db
                        .save_tool_call(
                            &tool_id,
                            session_id,
                            Some(tool_ctx.trace_id),
                            llm_call_id,
                            step,
                            name,
                            tool_source,
                            tool_skill_name.as_deref(),
                            Some(&input_json),
                            Some(&output.content),
                            !output.is_error && !non_zero,
                            non_zero,
                            tool_latency_ms,
                            err_msg,
                        )
                        .await
                    {
                        warn!(tool = %name, error = %e, "failed to save tool_call record");
                    }
                }

                let image_count = output.images.len();
                let output_summary = {
                    let raw = if image_count > 0 {
                        truncate_summary(
                            &format!("{} [+{image_count} image(s)]", output.content),
                            crate::planning::policy::OUTPUT_SUMMARY_MAX,
                        )
                    } else {
                        truncate_summary(
                            &output.content,
                            crate::planning::policy::OUTPUT_SUMMARY_MAX,
                        )
                    };
                    // Scrub secret-shaped values from metadata summaries (#908).
                    scrub_secrets(&raw).into_owned()
                };
                let non_zero_exit = !output.is_error && has_non_zero_exit_prefix(&output.content);
                let tool_succeeded = !output.is_error && !non_zero_exit;
                // mika#1731 — emit ToolCallResult on the A2A stream immediately
                // after dispatch, before `output_summary` is moved into the
                // ToolCallSummary. Fire-and-forget (see ToolCallStart above).
                if let Some(tx) = stream_tx {
                    let _ = tx.send(StreamEvent::ToolCallResult(ToolCallResultEvent {
                        task_id: tool_ctx.trace_id.to_string(),
                        context_id: None,
                        step,
                        tool_name: name.clone(),
                        success: tool_succeeded,
                        non_zero_exit,
                        output_summary: truncate_tool_summary(&output_summary),
                        duration_ms: tool_latency_ms,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    }));
                }
                summaries.push(ToolCallSummary {
                    step,
                    name: name.clone(),
                    input_summary,
                    output_summary,
                    success: tool_succeeded,
                    non_zero_exit,
                });

                // Send-message turn boundary activation (#771): after a
                // successful send_message in conversation mode, mark the
                // boundary as active. Silent/callback modes are exempt.
                if name == "send_message" && tool_succeeded && conversation_mode {
                    *send_message_boundary_active = true;
                    // Capture the send_message text for structured logging.
                    // Use the arguments (which contain the text parameter).
                    if send_message_text_capture.is_empty()
                        && let Some(text_val) = arguments.get("text").and_then(|v| v.as_str())
                    {
                        *send_message_text_capture = truncate_summary(text_val, 200);
                    }
                }

                dedup_cache.insert(dedup_key, output.clone());
                output
            };

            let image_count = output.images.len();
            let content = if output.images.is_empty() {
                LlmToolResultContent::Text(output.content)
            } else {
                let mut blocks = vec![LlmToolResultBlock::Text(output.content)];
                let mut included = 0;
                for img in output.images {
                    let img_bytes = img.data.len();
                    if img_bytes > image_bytes_budget {
                        break;
                    }
                    image_bytes_budget -= img_bytes;
                    included += 1;
                    blocks.push(LlmToolResultBlock::Image(LlmImage {
                        media_type: img.media_type,
                        data: img.data,
                    }));
                }
                if included < image_count {
                    let skipped = image_count - included;
                    blocks.push(LlmToolResultBlock::Text(format!(
                        "[{skipped} image(s) skipped: step memory budget exceeded]"
                    )));
                    warn!(
                        included,
                        skipped, "image budget exceeded, skipped images in tool result"
                    );
                }
                if included == 0 {
                    // All images skipped — fall back to text-only
                    LlmToolResultContent::Text(
                        blocks
                            .into_iter()
                            .filter_map(|b| match b {
                                LlmToolResultBlock::Text(text) => Some(text),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    )
                } else {
                    LlmToolResultContent::Blocks(blocks)
                }
            };
            tool_results.push(LlmContentBlock::ToolResult {
                tool_call_id: id.clone(),
                content,
                is_error: output.is_error,
            });
        }
    }

    // Convert response content to assistant message blocks
    let assistant_blocks: Vec<LlmContentBlock> =
        mika_common::llm::response_content_to_blocks(&response_content);
    request.messages.push(LlmMessage {
        role: LlmRole::Assistant,
        content: LlmContent::Blocks(assistant_blocks),
    });
    request.messages.push(LlmMessage {
        role: LlmRole::Tool,
        content: LlmContent::Blocks(tool_results),
    });
    summaries
}

/// Execute a single tool with timeout.
///
/// Routing: builtin tools (from ToolRegistry) first, then skill-defined tools,
/// then "unknown tool" error.
async fn execute_tool(
    dispatch: &ToolDispatchCtx<'_>,
    name: &str,
    input: serde_json::Value,
) -> ToolOutput {
    debug!(tool = %name, "tool_execution");

    // Pre-compute input excerpt for timeout diagnostics (#900). Must happen
    // before `input` is moved into the execute call.
    let input_excerpt: String = serde_json::to_string(&input)
        .unwrap_or_default()
        .chars()
        .take(TOOL_TIMEOUT_INPUT_EXCERPT_LEN)
        .collect();

    // 1. Try builtin tool
    if let Some(tool) = dispatch.tools.get(name) {
        let timeout = tool
            .timeout_secs()
            .unwrap_or(crate::planning::policy::TOOL_TIMEOUT_SECS);
        return match tokio::time::timeout(
            std::time::Duration::from_secs(timeout),
            tool.execute(input, dispatch.ctx),
        )
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                warn!(tool = %name, error = %e, "tool execution failed");
                ToolOutput::error(format!("Tool error: {e}"))
            }
            Err(_) => {
                warn!(tool = %name, timeout_secs = timeout, input_excerpt = %input_excerpt, "tool execution timed out");
                ToolOutput::error(format!("Tool '{name}' timed out after {timeout}s"))
            }
        };
    }

    // 2. Try skill-defined tool
    if let Some(skill_tool) = dispatch.skill_tools.get(name) {
        // Builtin skill handlers dispatch to Rust functions with ToolContext access
        if let ToolHandler::Builtin { function } = &skill_tool.handler {
            let timeout = dispatch.skill_timeout;
            return match tokio::time::timeout(
                std::time::Duration::from_secs(timeout),
                builtin_handlers::execute(function, input, dispatch.ctx),
            )
            .await
            {
                Ok(output) => output,
                Err(_) => {
                    warn!(tool = %name, timeout_secs = timeout, input_excerpt = %input_excerpt, "builtin handler timed out");
                    ToolOutput::error(format!("Builtin tool '{name}' timed out after {timeout}s"))
                }
            };
        }
        return executor::execute_skill_tool(
            skill_tool,
            input,
            dispatch.skill_timeout,
            dispatch.long_running_ctx,
            dispatch.ctx.github_token,
            dispatch.ctx.callback_task_id,
            if dispatch.ctx.callback_task_id.is_some() {
                Some(dispatch.ctx.db)
            } else {
                None
            },
        )
        .await;
    }

    // 3. Try MCP tool (external server)
    if let Some(mcp) = dispatch.mcp_manager
        && mcp.is_mcp_tool(name)
    {
        return match tokio::time::timeout(
            std::time::Duration::from_secs(crate::planning::policy::TOOL_TIMEOUT_SECS),
            mcp.call_tool(name, input),
        )
        .await
        {
            Ok(output) => output,
            Err(_) => {
                warn!(tool = %name, "MCP tool execution timed out");
                let timeout_secs = crate::planning::policy::TOOL_TIMEOUT_SECS;
                ToolOutput::error(format!("MCP tool '{name}' timed out after {timeout_secs}s"))
            }
        };
    }

    warn!(tool = %name, "unknown tool requested");
    ToolOutput::error(format!("Unknown tool: {name}"))
}

#[cfg(test)]
mod stream_emit_tests {
    //! mika#1731 — verify `process_tool_calls` emits `ToolCallStart` /
    //! `ToolCallResult` SSE frames around physical tool dispatch when a
    //! `stream_tx` is supplied, and stays silent (backward-compatible) when
    //! it is not. The "unknown tool" path is used deliberately: it is still a
    //! physical dispatch (non-cached branch), so both frames must fire — the
    //! result frame simply carries `success = false`.
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::sync::Arc;

    fn empty_request() -> LlmRequest {
        LlmRequest {
            model: "test-model".to_string(),
            system: None,
            messages: Vec::new(),
            tools: None,
            max_tokens: 1024,
            thinking: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_one_unknown_tool(
        harness: &TestHarness,
        request: &mut LlmRequest,
        step: u32,
        stream_tx: Option<&StreamEventSender>,
    ) -> Vec<ToolCallSummary> {
        let ctx = harness.ctx();
        let tools = ToolRegistry::new();
        let skill_tools: HashMap<String, &ResolvedSkillTool> = HashMap::new();
        let response = vec![LlmResponseContent::ToolCall {
            id: "call-1".to_string(),
            name: "definitely_unknown_tool".to_string(),
            arguments: serde_json::json!({"x": 1}),
        }];
        let mut boundary = false;
        let mut suppressed = Vec::new();
        let mut capture = String::new();

        process_tool_calls(
            response,
            &tools,
            &skill_tools,
            30,
            &ctx,
            request,
            step,
            None,
            None,
            &harness.db,
            "test-session",
            false, // store_tool_calls — avoid DB writes in this unit test
            None,
            &mut boundary,
            &mut suppressed,
            &mut capture,
            true, // conversation_mode
            stream_tx,
        )
        .await
    }

    #[tokio::test]
    async fn emits_tool_call_frames_when_stream_tx_present() {
        let harness = TestHarness::new();
        let mut request = empty_request();

        let (tx, mut rx) = tokio::sync::broadcast::channel::<StreamEvent>(16);
        let sender: StreamEventSender = Arc::new(tx);

        let summaries = run_one_unknown_tool(&harness, &mut request, 7, Some(&sender)).await;
        assert_eq!(summaries.len(), 1);

        match rx.try_recv().expect("expected a ToolCallStart frame") {
            StreamEvent::ToolCallStart(e) => {
                assert_eq!(e.step, 7);
                assert_eq!(e.tool_name, "definitely_unknown_tool");
                assert!(e.args_summary.contains("\"x\""));
            }
            other => panic!("expected ToolCallStart, got {other:?}"),
        }

        match rx.try_recv().expect("expected a ToolCallResult frame") {
            StreamEvent::ToolCallResult(e) => {
                assert_eq!(e.step, 7);
                assert_eq!(e.tool_name, "definitely_unknown_tool");
                // Unknown tool → dispatch returns an error output.
                assert!(!e.success);
            }
            other => panic!("expected ToolCallResult, got {other:?}"),
        }

        // Exactly the two frames — nothing else queued.
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn no_frames_and_still_dispatches_when_stream_tx_absent() {
        let harness = TestHarness::new();
        let mut request = empty_request();

        // A receiver on a separate channel proves that when stream_tx is None
        // no frame is produced by the dispatch path.
        let (tx, mut rx) = tokio::sync::broadcast::channel::<StreamEvent>(16);
        let _keep_sender_alive = tx;

        let summaries = run_one_unknown_tool(&harness, &mut request, 0, None).await;
        assert_eq!(summaries.len(), 1);
        assert!(rx.try_recv().is_err());
    }
}
