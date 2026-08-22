//! Tool dispatch — process_tool_calls(), execute_tool(), ToolDispatchCtx.
//!
//! Routes tool-use blocks from LLM responses through the three-tier dispatch
//! chain (builtin → skill → MCP), manages per-tool timeouts, per-turn dedup,
//! image-budget accounting, and persists tool-call metadata to SQLite.

use std::collections::HashMap;
use std::sync::Arc;

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
    /// Per-tool-name data-grade lookup (mika#1798 Layer 4).
    ///
    /// Built once per dispatch context from `SkillRegistry::tool_data_grades()`.
    /// The execute-time guardrail in `execute_tool` reads this map to reject
    /// any skill-registered tool whose owning skill declared
    /// `data_grade = "testimony"`. Catches hot-reload race windows and DB
    /// overrides that re-enable a testimony skill post-Phase-2 — cases
    /// where Phase-2 eviction is incomplete but the execute-time gate still
    /// fires.
    ///
    /// **MCP note (aspirational, not enforced today):** dynamically-registered
    /// MCP tools reach `execute_tool` via the third dispatch tier (`mcp.call_tool`)
    /// and are NOT keyed in `skill_data_grades` because they are not
    /// `skill_tools`. The map's shape is forward-compatible: when MCP manifests
    /// gain a `data_grade` field and MCP tools are seeded into this map, the
    /// L4 predicate will fire on them without any change to `execute_tool`.
    /// Until then, MCP-registered testimony tools bypass L4 and must be gated
    /// at MCP manifest ingestion — the vigilance-surface entry documented in
    /// `crates/mika-agent/docs/non-transit-data-grade.md`.
    pub(crate) skill_data_grades: HashMap<String, crate::skills::manifest::DataGrade>,
}

/// Execute tool-use blocks from a response and push both assistant and
/// tool-result messages onto the request. Returns summaries of each tool call
/// for persistence in conversation metadata.
///
/// `stream_ctx` (mika#1757) — when `Some`, this function emits
/// `StreamEvent::ToolCallStart` immediately before each PHYSICAL
/// `execute_tool()` dispatch and `StreamEvent::ToolCallResult` immediately
/// after. The per-turn dedup replay path (cached `ToolOutput` for identical
/// `(name, arguments)` blocks) does NOT emit — consumers see one Start +
/// one Result per REAL dispatch, matching the `tool_calls` DB row shape.
/// Suppressed send-message calls (intra-step boundary gate, #771) also do
/// not emit since no `execute_tool` runs. Emission is fire-and-forget:
/// broadcast errors are logged at `debug!` and never fail the tool call.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_tool_calls(
    response_content: Vec<LlmResponseContent>,
    tools: &ToolRegistry,
    skill_tools: &HashMap<String, &ResolvedSkillTool>,
    // mika#1798 Layer 4: per-tool `data_grade` lookup — cloned into the
    // per-step `ToolDispatchCtx` so the execute-time testimony guardrail can
    // refuse skill-registered tools whose owning skill declared
    // `data_grade = "testimony"` before the handler runs. Threaded from
    // `run_loop`; built alongside `skill_tools` in the caller so the two
    // maps stay consistent under last-write-wins collision semantics.
    skill_data_grades: &HashMap<String, crate::skills::manifest::DataGrade>,
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
    // mika#1757 — Optional broadcast context for `ToolCallStart` /
    // `ToolCallResult` emission. `None` for CLI, silent, team, delegate,
    // A2A `message/send`, and gateway `/message`; `Some` for A2A
    // `message/stream`. See docstring above for emission semantics.
    stream_ctx: Option<&Arc<mika_a2a::streaming::ToolCallStreamContext>>,
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
                    // mika#1798 Layer 4: clone the map so the dispatch context
                    // owns its own copy. The map is small (one entry per
                    // skill-registered tool) and cloned once per step, not per
                    // dedup lookup — same allocation profile as skill_tools.
                    skill_data_grades: skill_data_grades.clone(),
                };
                // mika#1757 — emit ToolCallStart immediately BEFORE physical
                // dispatch. Fire-and-forget: send errors (zero subscribers)
                // are debug-logged and never fail the tool call. Skipped for
                // dedup replays (this branch is the physical-dispatch path)
                // and for suppressed send_message calls (they never enter
                // this branch — the send-message boundary gate above uses
                // `continue`).
                if let Some(sc) = stream_ctx {
                    sc.emit_start(step, name, &input_summary);
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

                // mika#1757 — emit ToolCallResult AFTER dispatch completes,
                // BEFORE summary push and metadata build. Fire-and-forget:
                // send errors (zero subscribers) are debug-logged and never
                // fail the tool call. Carries the same-shape output_summary
                // the summary row uses (already scrubbed + truncated above).
                if let Some(sc) = stream_ctx {
                    sc.emit_result(
                        step,
                        name,
                        tool_succeeded,
                        non_zero_exit,
                        &output_summary,
                        tool_latency_ms,
                    );
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

/// Structured refusal body for the execute-time testimony guardrail (mika#1798 Layer 4).
///
/// Emitted before the tool handler runs when a skill-registered tool's owning
/// skill declared `data_grade = "testimony"`. The shape mirrors
/// `TESTIMONY_GRADE_FORBIDDEN_GMAIL` in `builtin_handlers.rs` so consumers
/// can pattern-match on `error = "testimony_grade_forbidden"` regardless of
/// which layer fired.
const TOOL_TESTIMONY_GRADE_FORBIDDEN: &str = r#"{"error":"testimony_grade_forbidden","doctrine":"mika#1798","reason":"This tool's owning skill declares data_grade = \"testimony\". Mika may NEVER access nor propose accessing testimony-grade data. This tool call is refused structurally at the execute-time guardrail (Layer 4)."}"#;

/// Execute a single tool with timeout.
///
/// Routing: builtin tools (from ToolRegistry) first, then skill-defined tools,
/// then "unknown tool" error.
///
/// **mika#1798 Layer 4 (execute-time guardrail):** before the skill-tool
/// dispatch fires, `dispatch.skill_data_grades` is consulted. If the tool's
/// owning skill declared `data_grade = "testimony"`, the call is rejected
/// with `TOOL_TESTIMONY_GRADE_FORBIDDEN` before reaching the handler. This
/// closes the hot-reload race window, dynamic MCP registration (forward-
/// compatible), and DB-override paths that Phase-2 eviction cannot cover.
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

    // mika#1798 Layer 4: execute-time testimony guardrail. Runs BEFORE the
    // three-tier dispatch chain so ANY tool (builtin, skill, MCP) whose
    // owning skill declared `data_grade = "testimony"` is refused. The
    // lookup is O(1) against the pre-built `skill_data_grades` map. Only
    // skill-registered tools are keyed in the map — builtin-only tools
    // like `run_gh` do not have a `data_grade` classification and pass
    // through untouched (their gate lives in the handler, see the Layer 3
    // Gmail subcommand ban in `builtin_handlers::validate_gws_input`).
    if let Some(&grade) = dispatch.skill_data_grades.get(name)
        && grade == crate::skills::manifest::DataGrade::Testimony
    {
        tracing::warn!(
            event = "tool_testimony_ban",
            tool = %name,
            doctrine = "mika#1798",
            "tool call refused by execute-time testimony guardrail — owning skill declares data_grade = \"testimony\""
        );
        return ToolOutput::error(TOOL_TESTIMONY_GRADE_FORBIDDEN);
    }

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
mod tests {
    use super::*;
    use crate::skills::manifest::DataGrade;

    // -- mika#1798 Layer 4 tests --

    #[test]
    fn testimony_forbidden_body_is_well_formed_json() {
        // The refusal body must parse as JSON and carry the structured
        // discriminator fields — consumers pattern-match on
        // `error = "testimony_grade_forbidden"` regardless of which layer
        // (subcommand ban, execute-time guardrail, registry-eviction path)
        // emitted it.
        let parsed: serde_json::Value =
            serde_json::from_str(TOOL_TESTIMONY_GRADE_FORBIDDEN).expect("must parse as JSON");
        assert_eq!(
            parsed.get("error").and_then(|v| v.as_str()),
            Some("testimony_grade_forbidden")
        );
        assert_eq!(
            parsed.get("doctrine").and_then(|v| v.as_str()),
            Some("mika#1798")
        );
        assert!(
            parsed
                .get("reason")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "reason field must be present and non-empty"
        );
    }

    #[test]
    fn tool_dispatch_ctx_holds_skill_data_grades() {
        // Structural test: the ToolDispatchCtx field is public within the
        // crate, mapped one-to-one with the tool set the dispatch runs
        // against. This test exercises the `#[allow(dead_code)]`-free
        // wiring at the type level.
        let mut grades = HashMap::new();
        grades.insert("gmail_fetch".to_string(), DataGrade::Testimony);
        grades.insert("web_search".to_string(), DataGrade::Operational);

        assert_eq!(grades.get("gmail_fetch"), Some(&DataGrade::Testimony));
        assert_eq!(grades.get("web_search"), Some(&DataGrade::Operational));
        assert_eq!(grades.get("unknown_tool"), None);

        // The predicate the guardrail evaluates.
        assert!(
            matches!(grades.get("gmail_fetch"), Some(&DataGrade::Testimony)),
            "Layer 4 predicate must classify gmail_fetch as Testimony"
        );
        assert!(
            !matches!(grades.get("web_search"), Some(&DataGrade::Testimony)),
            "Layer 4 predicate must not classify web_search as Testimony"
        );
    }
}
