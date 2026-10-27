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
    /// Fallback tool-execution budget: the maximum across every skill loaded in
    /// the turn (`max_skill_timeout`). Applies to skill tools absent from
    /// [`Self::skill_tool_timeouts`] only — see that field for why.
    pub(crate) skill_timeout: u64,
    /// Per-tool-name execution budget, keyed by the tool's **owning** skill
    /// (mika#2276 M1).
    ///
    /// Built alongside `skill_tools` and `skill_data_grades` from the same
    /// `matched` slice under the same last-write-wins rule, so a tool's handler,
    /// its data grade and its budget always come from one skill.
    ///
    /// Before mika#2276 there was no such map and every skill tool ran under
    /// [`Self::skill_timeout`]. That let `run_shell` — declared at 30 s by
    /// `shell-exec` — execute under `build-mika`'s 300 s merely because
    /// `build-mika` is a declared dependency of `qa-review`, which is how a QA
    /// review turn spent 469 s of its ~506 s envelope on two
    /// `cargo test --release` calls and died before writing a verdict.
    pub(crate) skill_tool_timeouts: &'a HashMap<String, u64>,
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
    /// **MCP tools are in this map, and in force since mika#1959.**
    /// They reach `execute_tool` via the third dispatch tier (`mcp.call_tool`)
    /// and are not `skill_tools`, so `build_skill_data_grades` does not see
    /// them — F5 of the mika#1798 adversarial review. The map is keyed by the
    /// Claude-facing tool name, which an MCP tool has
    /// (`mcp__<server>__<tool>`), so the fix was to **seed** it rather than to
    /// add a second predicate: `McpServerConfig.data_grade` is joined onto each
    /// namespaced tool by `mcp::mcp_tool_data_grades`, exposed as
    /// `McpManager::tool_data_grades()`, and inserted here by
    /// `agent_loop::seed_mcp_data_grades` at each of the two sites that publish
    /// `mcp.tool_definitions()` to the model. **`execute_tool` was not modified**
    /// — the forward-compatible shape this comment used to promise was real,
    /// and the empty diff on that function is the proof.
    ///
    /// Silent/callback/heartbeat mode receives `None` in place of the manager
    /// and is out of population by construction, not by omission; the pairing
    /// is held by the structural guard
    /// `mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4`.
    ///
    /// **Residue, not closed:** a testimony MCP tool is still *published* in
    /// `tool_definitions()`, so the model can still propose it — only the call
    /// is refused. See § *Vigilance surface* of
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
    // mika#2276 M1: per-tool budget keyed by the owning skill. Built next to
    // `skill_tools` in the caller so the two maps cannot disagree on who owns
    // a tool. `skill_timeout` above stays the fallback for anything absent.
    skill_tool_timeouts: &HashMap<String, u64>,
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
    // mika#2136 — the turn's delivery sequence, in call order, accumulated
    // across steps. Travels by `&mut` rather than in `LoopResult`: that enum has
    // three variants each carrying a `Vec<ToolCallSummary>`, so a field would be
    // three writes to keep in sync, and the `&mut` gives the `MaxStepsExceeded`
    // / `DeadlineExceeded` exits their coverage for free — a turn cut off by its
    // envelope after a dead send owes the same admission as one that concludes.
    // Never persisted, never serialized; it dies with the turn.
    delivery_log: &mut Vec<crate::evidence::guards::DeliveryRecord>,
    // mika#2934 — ce que le tour a DÉJÀ exécuté avant ce pas, en ordre d'appel.
    // Seul le site de refus de `record_reviewed_head` le lit : la preuve qu'il
    // exige (« un `pr review` posté », « une lecture de CETTE tête ») peut avoir
    // été produite à un pas antérieur du même tour, et `summaries` ci-dessous ne
    // naît qu'avec ce pas-ci.
    //
    // En `&[…]` plutôt que dans `ToolContext` parce que cet historique **grandit
    // pendant le tour** : `ToolContext` est construit une fois par tour et ne
    // porte que des verdicts figés (voir `is_webhook_fallthrough_turn`).
    // L'emprunt est immuable et se termine avec l'appel, donc les sites d'appel
    // peuvent passer `&all_tool_summaries` puis l'`extend`er juste après.
    prior_summaries: &[ToolCallSummary],
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

            // Garde de preuve du tour sur `record_reviewed_head` (mika#2934).
            //
            // Le registre `qa_reviewed_head` dit sur quelles têtes mika-qa a
            // **statué**. Le 2026-10-27, trois têtes y ont été déclarées
            // conclues 9 à 17 minutes avant le premier verdict sur chacune, et
            // une quatrième ligne a attesté un verdict qu'un tour n'avait pas
            // encore posté : dans les quatre cas le tour demandait une ligne que
            // sa propre trace d'outils ne portait pas.
            //
            // Le refus est **en amont de l'exécution** : aucune ligne n'est
            // écrite, et ce n'est pas un `field_error` de l'outil — un refus de
            // validation nomme un champ fautif, celui-ci nomme une preuve
            // absente. Même forme que le gate de frontière ci-dessus : un
            // `ToolCallSummary` en échec, un `tool_result` `is_error` pour garder
            // l'historique apparié, `continue` sans appeler l'outil.
            //
            // Pas une garde EndTurn : à la fin du tour la ligne serait déjà en
            // base, et la garde nommerait le dégât au lieu de l'empêcher.
            if name == crate::qa_reviewed_head::RECORD_REVIEWED_HEAD_TOOL {
                // Un argument absent ou mal typé rend `""`, que
                // `preuve_du_tour_manquante` laisse passer : c'est l'outil qui
                // nomme le champ fautif, et lui seul. Cette garde ne parle que
                // de preuve.
                let outcome = arguments
                    .get("outcome")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let head_sha = arguments
                    .get("head_sha")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                // Les pas antérieurs du tour, puis ceux de ce bloc-ci, en ordre
                // d'appel. La copie est bornée par construction : au plus un
                // appel de cet outil par tour, et chaque résumé est déjà tronqué
                // à `INPUT_SUMMARY_MAX`.
                let deja_execute: Vec<ToolCallSummary> = prior_summaries
                    .iter()
                    .chain(summaries.iter())
                    .cloned()
                    .collect();
                if let Some(variante) = crate::qa_reviewed_head::preuve_du_tour_manquante(
                    outcome,
                    head_sha,
                    &deja_execute,
                ) {
                    let (variante_label, ce_qui_manque) = match variante {
                        crate::qa_reviewed_head::PreuveManquante::VerdictSansPost => (
                            "verdict_sans_post",
                            "`verdict_posted` attests a verdict this turn posted. No successful \
                             `run_gh(\"pr review …\")` appears in this turn's tool history. If you \
                             dispatched a build and are awaiting its callback, record nothing — \
                             the callback turn records the head after its own post (qa-review \
                             invariant 7)."
                                .to_string(),
                        ),
                        crate::qa_reviewed_head::PreuveManquante::StopSansLectureDeTete => (
                            "stop_sans_lecture_de_tete",
                            format!(
                                "a `stop_*` outcome attests a head you ruled out on the evidence \
                                 of THAT head's reviews. No successful `qa_head_reviews` call for \
                                 `{head_sha}` appears in this turn's tool history. Call it first \
                                 (step 3), then conclude."
                            ),
                        ),
                    };
                    let suppressed_msg = format!(
                        "[mika-engine] Tool call suppressed: record_reviewed_head needs this \
                         turn's own evidence (mika#2934). {ce_qui_manque}"
                    );
                    // La sonde de la garde. Son **zéro** est l'assertion que le
                    // prompt tient tout seul ; chaque ligne est un tour qui a
                    // demandé une attestation qu'il n'avait pas produite.
                    warn!(
                        trace_id = %tool_ctx.trace_id,
                        step,
                        outcome = %outcome,
                        variante = variante_label,
                        "reviewed_head_write_without_evidence: suppressed record_reviewed_head"
                    );
                    let input_summary_for_suppressed = truncate_summary(
                        &arguments.to_string(),
                        crate::planning::policy::INPUT_SUMMARY_MAX,
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
                    // Délibérément **pas** dans `suppressed_write_tools` : ce
                    // vecteur alimente le gate inter-pas de `send_message`, qui
                    // force un EndTurn. Ici le tour doit pouvoir continuer — et
                    // produire la preuve qui manque.
                    tool_results.push(LlmContentBlock::ToolResult {
                        tool_call_id: id.clone(),
                        content: LlmToolResultContent::Text(suppressed_msg),
                        is_error: true,
                    });
                    continue;
                }
            }

            let dedup_key = (
                name.clone(),
                serde_json::to_string(arguments).unwrap_or_default(),
            );
            let mut output = if let Some(cached) = dedup_cache.get(&dedup_key) {
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
                // mika#2136 — same reasoning, one field over: the delivery
                // verdict belongs to the one execution that actually happened.
                // Left on the clone, a replayed duplicate would push a SECOND
                // `DeliveryRecord` for a tool that ran once. The guard predicate
                // would stay correct (a duplicated `Failed` is still a `Failed`),
                // but `failed_count` would say two dead fragments where there is
                // one, and the line the engine writes to the user would be wrong
                // about the number.
                reused.delivery = None;
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
                    skill_tool_timeouts,
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

            // mika#2136 — record what became of a user-facing message, so the
            // EndTurn guard can refuse a turn that closes over an unrepaired
            // delivery failure. The text is TAKEN FROM THE VERDICT and never
            // re-read from `arguments`: the raw argument is a different register
            // (the tool measures and sends `cleaned`), it is truncated to 200
            // chars at the only site that captures it, and it is never captured
            // at all on a failed send — so a record built from it would be wrong
            // in three directions at once (E9). Full text, no truncation: the
            // repair predicate is an equality, and a shared prefix between two
            // fragments would read as a repair that did not happen.
            //
            // The boundary suppression above pushes nothing, and needs no
            // gesture: its `continue` precedes `execute`, so no verdict exists.
            // Do not "fix" that by synthesizing a record — a suppressed send did
            // not fail to deliver, it was never attempted, and counting it would
            // make `failed_count` lie about what the user did not read.
            if let Some(verdict) = output.delivery.take() {
                delivery_log.push(crate::evidence::guards::DeliveryRecord {
                    step,
                    text: verdict.text,
                    outcome: verdict.outcome,
                });
            }

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
/// closes the hot-reload race window, dynamic MCP registration (in force
/// since mika#1959 — see `skill_data_grades`), and DB-override paths that
/// Phase-2 eviction cannot cover.
///
/// **This function's body is unchanged by mika#1959.** The MCP tier was brought
/// under L4 by seeding `skill_data_grades`, not by a second predicate here;
/// only this parenthetical moved, because it asserted the opposite of what now
/// happens.
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
        // mika#2276 M1: the budget is the OWNING skill's, resolved once per turn
        // in `build_skill_tool_timeouts`. `skill_timeout` (the turn maximum) is
        // the fallback only — a skill tool present in `skill_tools` is always
        // present in this map, since both are built from the same slice, so the
        // fallback is reachable in practice only from a hand-built dispatch
        // context in tests.
        let owning_skill_timeout = dispatch
            .skill_tool_timeouts
            .get(name)
            .copied()
            .unwrap_or(dispatch.skill_timeout);
        // Builtin skill handlers dispatch to Rust functions with ToolContext access
        if let ToolHandler::Builtin { function } = &skill_tool.handler {
            let timeout = owning_skill_timeout;
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
            owning_skill_timeout,
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

    // -- mika#1959 : la troisième tier MCP sous la garde L4 --

    /// Outil MCP de référence des deux tests ci-dessous, et son serveur.
    const OUTIL_MCP: &str = "mcp__journal-perso__read_entry";
    const SERVEUR_MCP: &str = "journal-perso";

    /// Manager MCP sans connexion qui route `OUTIL_MCP`, au grade demandé.
    fn manager_mcp(grade: DataGrade) -> McpManager {
        let mut routage = HashMap::new();
        routage.insert(
            OUTIL_MCP.to_string(),
            (SERVEUR_MCP.to_string(), "read_entry".to_string()),
        );
        let mut grades = HashMap::new();
        grades.insert(OUTIL_MCP.to_string(), grade);
        McpManager::sans_connexion_pour_tests(routage, grades)
    }

    /// La phrase que rend `McpManager::call_tool` quand il est atteint sur un
    /// serveur non connecté. C'est le témoin d'atteinte du troisième étage.
    fn temoin_call_tool_atteint() -> String {
        format!("MCP server '{SERVEUR_MCP}' is not connected.")
    }

    #[tokio::test]
    async fn mika1959_un_outil_mcp_testimony_est_refuse_avant_call_tool() {
        let harness = crate::test_utils::test_helpers::TestHarness::new();
        let ctx = harness.ctx();
        let tools = ToolRegistry::new();
        let skill_tools: HashMap<String, &ResolvedSkillTool> = HashMap::new();
        let timeouts: HashMap<String, u64> = HashMap::new();
        let mcp = manager_mcp(DataGrade::Testimony);

        // La map L4 telle que `agent_loop::seed_mcp_data_grades` la produit.
        let dispatch = ToolDispatchCtx {
            tools: &tools,
            skill_tools: &skill_tools,
            ctx: &ctx,
            skill_timeout: 30,
            skill_tool_timeouts: &timeouts,
            mcp_manager: Some(&mcp),
            long_running_ctx: None,
            skill_data_grades: mcp.tool_data_grades().clone(),
        };

        let sortie = execute_tool(&dispatch, OUTIL_MCP, serde_json::json!({})).await;

        assert!(sortie.is_error, "un outil testimony doit être refusé");
        assert_eq!(
            sortie.content, TOOL_TESTIMONY_GRADE_FORBIDDEN,
            "le corps de refus doit être celui de la garde L4"
        );
        // Le témoin d'atteinte : son absence est la preuve qu'aucun appel
        // distant n'a été tenté. Le contrôle positif ci-dessous montre que
        // cette phrase sort bel et bien quand `call_tool` est atteint.
        assert!(
            !sortie.content.contains(&temoin_call_tool_atteint()),
            "la garde doit refuser AVANT le troisième étage, pas après"
        );
    }

    #[tokio::test]
    async fn mika1959_le_meme_outil_en_operational_nest_pas_refuse_par_l4() {
        let harness = crate::test_utils::test_helpers::TestHarness::new();
        let ctx = harness.ctx();
        let tools = ToolRegistry::new();
        let skill_tools: HashMap<String, &ResolvedSkillTool> = HashMap::new();
        let timeouts: HashMap<String, u64> = HashMap::new();
        let mcp = manager_mcp(DataGrade::Operational);

        let dispatch = ToolDispatchCtx {
            tools: &tools,
            skill_tools: &skill_tools,
            ctx: &ctx,
            skill_timeout: 30,
            skill_tool_timeouts: &timeouts,
            mcp_manager: Some(&mcp),
            long_running_ctx: None,
            skill_data_grades: mcp.tool_data_grades().clone(),
        };

        let sortie = execute_tool(&dispatch, OUTIL_MCP, serde_json::json!({})).await;

        assert_ne!(
            sortie.content, TOOL_TESTIMONY_GRADE_FORBIDDEN,
            "L4 ne doit pas refuser un outil MCP operational"
        );
        // Contrôle positif de la paire : le dispatch a poursuivi jusqu'au
        // troisième étage et `call_tool` a répondu. Sans cette assertion, le
        // test ci-dessus pourrait être vert sur un chemin qui n'atteint jamais
        // `call_tool` pour une raison étrangère au grade.
        assert_eq!(
            sortie.content,
            temoin_call_tool_atteint(),
            "le dispatch doit atteindre McpManager::call_tool"
        );
    }

    // ---- mika#2934 : le site de refus de `record_reviewed_head` -------------

    /// La tête poussée le 2026-10-27 sur #2931, et celle que son approbation du
    /// 26/10 portait. Les deux mêmes valeurs qu'`qa_reviewed_head.rs`, parce que
    /// les deux moitiés d'AC2 rejouent le même force-push.
    const TETE_NOUVELLE: &str = "1f649bf10e2d4c6a8b1d3f5e7a9c0b2d4e6f8a1c";
    const TETE_ANCIENNE: &str = "3b44b0359c7e5a3f1d0b8a6c4e2f0d9b7a5c3e1f";

    /// Requête minimale : `process_tool_calls` n'y pousse que des messages.
    fn requete_vide() -> LlmRequest {
        LlmRequest {
            model: "claude-sonnet-4-6".to_string(),
            system: None,
            messages: Vec::new(),
            tools: None,
            max_tokens: 1024,
            thinking: None,
        }
    }

    /// Les arguments que le prompt du webhook fait émettre à l'étape 4.
    fn appel_de_letape_4(tete: &str) -> serde_json::Value {
        serde_json::json!({
            "repo": "senara-solutions/mika",
            "pr": 2931,
            "head_sha": tete,
            "outcome": "stop_verdict_recorded",
        })
    }

    /// Le résumé qu'un `qa_head_reviews` réussi laisse derrière lui, sous la
    /// borne réelle du moteur — c'est la forme que la garde doit reconnaître.
    fn lecture_de_tete_reussie(tete: &str) -> ToolCallSummary {
        let arguments = serde_json::json!({
            "repo": "senara-solutions/mika",
            "pr": 2931,
            "head_sha": tete,
        });
        ToolCallSummary {
            step: 3,
            name: crate::qa_reviewed_head::QA_HEAD_REVIEWS_TOOL.to_string(),
            input_summary: truncate_summary(
                &arguments.to_string(),
                crate::planning::policy::INPUT_SUMMARY_MAX,
            ),
            output_summary: "[]".to_string(),
            success: true,
            non_zero_exit: false,
        }
    }

    /// Le dernier message d'une requête : celui que `process_tool_calls` remplit
    /// avec les `tool_result` du pas.
    fn resultats_doutils(request: &LlmRequest) -> Vec<LlmContentBlock> {
        match &request.messages.last().expect("un message Tool").content {
            LlmContent::Blocks(blocks) => blocks.clone(),
            other => panic!("le dernier message doit porter des blocs, pas {other:?}"),
        }
    }

    /// Lance un pas dont le seul bloc est l'appel d'écriture de l'étape 4, avec
    /// `prior_summaries` pour tout historique. Rend les résumés et la requête.
    async fn un_pas_decriture(
        harness: &crate::test_utils::test_helpers::TestHarness,
        tete: &str,
        prior_summaries: &[ToolCallSummary],
    ) -> (Vec<ToolCallSummary>, LlmRequest) {
        let ctx = harness.ctx();
        // Le registre **réel**, pas un bouchon : si la garde ne tirait pas,
        // `record_reviewed_head` s'exécuterait et écrirait sa ligne. C'est ce
        // qui rend l'assertion « l'outil n'a pas été appelé » falsifiable.
        let tools = crate::tools::default_tools();
        let skill_tools: HashMap<String, &ResolvedSkillTool> = HashMap::new();
        let grades: HashMap<String, DataGrade> = HashMap::new();
        let timeouts: HashMap<String, u64> = HashMap::new();
        let mut request = requete_vide();
        let mut boundary = false;
        let mut supprimes: Vec<String> = Vec::new();
        let mut capture = String::new();
        let mut livraisons: Vec<crate::evidence::guards::DeliveryRecord> = Vec::new();

        let summaries = process_tool_calls(
            vec![LlmResponseContent::ToolCall {
                id: "toolu_2934".to_string(),
                name: crate::qa_reviewed_head::RECORD_REVIEWED_HEAD_TOOL.to_string(),
                arguments: appel_de_letape_4(tete),
            }],
            &tools,
            &skill_tools,
            &grades,
            30,
            &timeouts,
            &ctx,
            &mut request,
            4,
            None,
            None,
            &harness.db,
            "test-session",
            false,
            None,
            &mut boundary,
            &mut supprimes,
            &mut capture,
            false,
            None,
            &mut livraisons,
            prior_summaries,
        )
        .await;

        (summaries, request)
    }

    /// AC2/AC3, moitié moteur : le site **supprime** l'appel au lieu de le
    /// laisser écrire.
    ///
    /// Trois assertions, et la troisième est celle qui compte : un résumé en
    /// échec et un `tool_result` `is_error` diraient la même chose d'un outil
    /// qui a tourné et refusé son entrée. Ce qui sépare les deux, c'est
    /// **l'absence de ligne en base** — le registre réel est enregistré, donc
    /// sans la garde l'appel aboutirait.
    ///
    /// Le contrôle positif est dans le même test : avec la lecture de tête du
    /// pas 3 dans l'historique, le même appel passe et la ligne existe. Sans
    /// lui, un site qui refuserait `record_reviewed_head` **toujours** serait
    /// vert ici, et aurait désarmé le registre entier.
    #[tokio::test]
    async fn mika2934_le_site_supprime_lappel_sans_lexecuter() {
        let harness = crate::test_utils::test_helpers::TestHarness::new();

        let (summaries, request) = un_pas_decriture(&harness, TETE_NOUVELLE, &[]).await;

        assert_eq!(summaries.len(), 1, "un bloc, un résumé");
        let resume = &summaries[0];
        assert_eq!(
            resume.name,
            crate::qa_reviewed_head::RECORD_REVIEWED_HEAD_TOOL,
            "le résumé doit nommer l'outil supprimé"
        );
        assert!(!resume.success, "le résumé du refus est en échec");
        assert!(
            !resume.non_zero_exit,
            "une suppression n'est pas un code de sortie non nul"
        );
        assert!(
            resume.output_summary.contains("mika#2934"),
            "le refus doit se nommer ; lu : {}",
            resume.output_summary
        );

        let resultats = resultats_doutils(&request);
        assert_eq!(resultats.len(), 1, "un `tool_result` pour garder la paire");
        match &resultats[0] {
            LlmContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_call_id, "toolu_2934", "la paire doit être appariée");
                assert!(
                    *is_error,
                    "le `tool_result` d'une suppression est `is_error`"
                );
                match content {
                    LlmToolResultContent::Text(texte) => assert!(
                        texte.contains("qa_head_reviews") && texte.contains(TETE_NOUVELLE),
                        "le message doit nommer la lecture manquante ET la tête ; lu : {texte}"
                    ),
                    autre => panic!("le refus doit être du texte, pas {autre:?}"),
                }
            }
            autre => panic!("le bloc doit être un `tool_result`, pas {autre:?}"),
        }

        // L'assertion centrale : l'outil n'a pas tourné.
        let lignes = harness
            .db
            .count_audit_events_by_tool_name(crate::qa_reviewed_head::QA_REVIEWED_HEAD_TOOL)
            .await
            .expect("lecture du registre");
        assert_eq!(
            lignes, 0,
            "un appel supprimé n'écrit RIEN — l'écrivain unique n'a pas été atteint"
        );

        // ---- Contrôle positif : la preuve présente, l'appel passe ----------
        let (summaries, request) = un_pas_decriture(
            &harness,
            TETE_NOUVELLE,
            &[lecture_de_tete_reussie(TETE_NOUVELLE)],
        )
        .await;

        assert!(
            summaries[0].success,
            "avec la lecture de CETTE tête, l'appel doit aboutir ; sortie : {}",
            summaries[0].output_summary
        );
        match &resultats_doutils(&request)[0] {
            LlmContentBlock::ToolResult { is_error, .. } => {
                assert!(!*is_error, "un appel satisfait n'est pas une erreur")
            }
            autre => panic!("le bloc doit être un `tool_result`, pas {autre:?}"),
        }
        let lignes = harness
            .db
            .count_audit_events_by_tool_name(crate::qa_reviewed_head::QA_REVIEWED_HEAD_TOOL)
            .await
            .expect("lecture du registre");
        assert_eq!(lignes, 1, "l'appel satisfait écrit sa ligne, et une seule");

        // ---- Et la lecture de l'ANCIENNE tête ne la satisfait pas ----------
        // La propriété que `reviewDecision` n'avait pas, vue depuis le site.
        let (summaries, _) = un_pas_decriture(
            &harness,
            TETE_NOUVELLE,
            &[lecture_de_tete_reussie(TETE_ANCIENNE)],
        )
        .await;
        assert!(
            !summaries[0].success,
            "une lecture de l'ancienne tête ne prouve rien sur la nouvelle"
        );
        let lignes = harness
            .db
            .count_audit_events_by_tool_name(crate::qa_reviewed_head::QA_REVIEWED_HEAD_TOOL)
            .await
            .expect("lecture du registre");
        assert_eq!(lignes, 1, "aucune seconde ligne : le force-push est refusé");
    }

    /// La garde correspond sur `input_summary`, qui est **tronqué** à
    /// [`crate::planning::policy::INPUT_SUMMARY_MAX`]. Si cette borne descendait
    /// sous la position du `head_sha` dans les arguments de `qa_head_reviews`,
    /// la garde cesserait de reconnaître une lecture pourtant faite — et
    /// refuserait **tous** les `stop_*`, en silence.
    ///
    /// Ce test épingle la borne par le besoin plutôt que par sa valeur : il
    /// rougit si la troncature mange le SHA, quelle que soit la raison.
    #[test]
    fn mika2934_la_borne_de_resume_dentree_laisse_passer_un_sha() {
        let resume = lecture_de_tete_reussie(TETE_NOUVELLE);

        // Anti-vacuité : la troncature a bien été appliquée à la forme réelle.
        assert!(
            resume.input_summary.len() <= crate::planning::policy::INPUT_SUMMARY_MAX,
            "le résumé doit être borné comme le moteur le borne"
        );
        assert!(
            resume.input_summary.contains(TETE_NOUVELLE),
            "le SHA doit survivre à la borne de {} octets ; lu : {}",
            crate::planning::policy::INPUT_SUMMARY_MAX,
            resume.input_summary
        );

        // Et la garde en tire bien son `None` : la borne sert la décision, pas
        // seulement la lisibilité du journal.
        assert_eq!(
            crate::qa_reviewed_head::preuve_du_tour_manquante(
                "stop_verdict_recorded",
                TETE_NOUVELLE,
                std::slice::from_ref(&resume),
            ),
            None,
            "sous la borne réelle, une lecture faite doit satisfaire la preuve"
        );
    }
}
