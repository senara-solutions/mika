use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use dashmap::DashMap;
use secrecy::ExposeSecret;
use tokio::sync::{OwnedMutexGuard, broadcast};
use tracing::{debug, error, info};
use uuid::Uuid;

use mika_a2a::jsonrpc::{
    A2aMethod, INTERNAL_ERROR, INVALID_PARAMS, JsonRpcError, JsonRpcId, JsonRpcRequest,
    JsonRpcResponse, METHOD_NOT_FOUND, TASK_NOT_CANCELABLE, TASK_NOT_FOUND,
};
use mika_a2a::params::{
    CALLER_SESSION_ID_KEY, MessageSendParams, ONLY_SKILLS_KEY, TaskIdParams, TaskQueryParams,
};
use mika_a2a::state_machine::TaskStateMachine;
use mika_a2a::streaming::{StreamEvent, TaskStatusUpdateEvent};
use mika_a2a::types::{Message, Part, Role, Task, TaskState, TaskStatus};

use crate::a2a_card::build_agent_card;
use crate::a2a_db::extract_text_from_parts;
use crate::agent::{self, AgentParams, check_onboarding};
use crate::server::a2a_wait_queue::{self, WaitSlot};
use crate::server::state::{AgentState, AppState};

/// Guard that removes a broadcaster entry from the DashMap when dropped.
/// Ensures cleanup happens even if the spawned task panics.
struct BroadcasterGuard {
    map: Arc<DashMap<String, broadcast::Sender<StreamEvent>>>,
    key: String,
}

impl Drop for BroadcasterGuard {
    fn drop(&mut self) {
        self.map.remove(&self.key);
    }
}

/// How a `message/stream` request reaches its turn (mika#2163).
///
/// The two variants are two different contracts, not two encodings of one. On the
/// kill-switch path the lock is taken in the handler and a busy agent is refused
/// there with the pre-mika#2163 JSON-RPC error; on the bounded path the caller
/// carries a place in the line into the spawned task and waits there, so the SSE
/// stream can open first.
enum StreamLockEntry {
    /// Kill-switch path: the lock is already held, taken in the handler.
    Held(OwnedMutexGuard<()>),
    /// Bounded path: a place in the line, to be spent waiting inside the spawn.
    Waiting(WaitSlot),
}

/// Handle A2A JSON-RPC POST requests.
pub async fn handle_a2a_jsonrpc(
    State(state): State<AppState>,
    Path(agent_name): Path<String>,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    debug!(method = %request.method, agent = %agent_name, "A2A JSON-RPC request");

    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            request.id.clone(),
            JsonRpcError::with_message(
                mika_a2a::jsonrpc::INVALID_REQUEST,
                "Invalid JSON-RPC version",
            ),
        ))
        .into_response();
    }

    // Resolve agent
    let agent_state = match state.resolve_agent(&agent_name).await {
        Some(a) => a,
        None => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(
                    INVALID_PARAMS,
                    format!("Agent not found: {agent_name}"),
                ),
            ))
            .into_response();
        }
    };

    // Parse method
    let method = match A2aMethod::parse(&request.method) {
        Some(m) => m,
        None => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::from_code(METHOD_NOT_FOUND),
            ))
            .into_response();
        }
    };

    // Dispatch
    match method {
        A2aMethod::MessageSend => handle_message_send(&state, &agent_state, request).await,
        A2aMethod::MessageStream => handle_message_stream(&state, &agent_state, request).await,
        A2aMethod::TasksGet => handle_tasks_get(&agent_state, request).await,
        A2aMethod::TasksCancel => handle_tasks_cancel(&agent_state, request).await,
        A2aMethod::PushNotificationConfigSet => handle_push_config_set(&agent_state, request).await,
        A2aMethod::PushNotificationConfigGet => handle_push_config_get(&agent_state, request).await,
        A2aMethod::PushNotificationConfigList => {
            handle_push_config_list(&agent_state, request).await
        }
        A2aMethod::PushNotificationConfigDelete => {
            handle_push_config_delete(&agent_state, request).await
        }
        A2aMethod::TasksResubscribe => {
            handle_tasks_resubscribe(&state, &agent_state, request).await
        }
    }
}

/// Run the agent loop for an A2A request, similar to handle_message.
///
/// `stream_ctx` is the per-task broadcast context the caller (typically
/// `handle_message_stream`) uses to publish SSE frames. It bundles the
/// sender, task_id, and optional context_id so `process_tool_calls` can
/// emit `ToolCallStart` / `ToolCallResult` frames (mika#1731 wire;
/// mika#1757 emission). Non-streaming callers (`message/send`) pass `None`.
async fn run_a2a_agent(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    session_id: &str,
    input_text: &str,
    task_id: &str,
    stream_ctx: Option<Arc<mika_a2a::streaming::ToolCallStreamContext>>,
    only_skills: &[String],
) -> Result<Option<String>, String> {
    // Hot-reload skills if dirty
    let skills = if agent_state.skills_dirty.load(Ordering::Acquire) {
        agent_state.skills_dirty.store(false, Ordering::Release);
        let mut registry =
            crate::skills::SkillRegistry::from_dir(&agent_state.home_dir.join("skills"));
        let identity = crate::prompt::load_identity(&agent_state.home_dir);
        if let Some(ref allowlist) = identity.skills.allowlist {
            registry.apply_identity_allowlist(allowlist);
        }
        if let Ok(overrides) = agent_state
            .db
            .get_skill_overrides(agent_state.db.agent_id())
            .await
        {
            registry.apply_overrides(&overrides);
        }
        // Phase 2 (mika#1798): testimony-grade ban.
        registry.apply_testimony_grade_ban();
        registry.log_summary();
        let new = Arc::new(registry);
        *agent_state.skills.lock().unwrap() = new.clone();
        new
    } else {
        agent_state.skills.lock().unwrap().clone()
    };

    // Per-turn restriction (mika#2363). The clone is what keeps this off the
    // shared registry: `skills` above is the `Arc` two concurrent turns hold, and
    // restricting through it would give one turn's `only_skills` to the other.
    // The restricted registry is never written back into
    // `*agent_state.skills.lock()`.
    //
    // An empty request leaves `skills` untouched — same `Arc`, no clone, no
    // allocation — so a caller that declares nothing gets today's turn byte for
    // byte.
    let skills = if only_skills.is_empty() {
        skills
    } else {
        let mut restricted = (*skills).clone();
        let outcome = restricted.apply_only_skills(only_skills);
        info!(
            event = "a2a_only_skills_applied",
            agent = %agent_state.db.agent_id(),
            task_id = %task_id,
            requested = %only_skills.join(","),
            kept = %outcome.kept.join(","),
            evicted_count = outcome.evicted.len(),
            unknown_count = outcome.unknown.len(),
            "restricted this turn's skill registry to the caller's declared set"
        );
        Arc::new(restricted)
    };

    let is_onboarding = check_onboarding(&agent_state.db).await;

    let params = AgentParams {
        db: &agent_state.db,
        tier: agent_state.tier,
        deployment: agent_state.deployment,
        llm: agent_state.llm.as_ref(),
        tools: &state.tools,
        skills: &skills,
        user_message: input_text,
        channel_type: "a2a",
        session_id,
        home_dir: &agent_state.home_dir,
        is_onboarding,
        message_sender: None, // A2A responses go back via JSON-RPC, not outbound messaging
        skip_compaction: true,
        embedding_client: agent_state.embedding_client.as_ref(),
        thinking: None,
        user_images: &[],
        brave_api_key: state.brave_api_key.as_deref(),
        github_token: state.github_token.as_deref(),
        gateway_url: Some(state.gateway_url.as_str()),
        internal_token: Some(state.internal_token.expose_secret()),
        github_app: agent_state.github_app.as_deref(),
        skills_dirty: &agent_state.skills_dirty,
        skill_nudge: Some(&agent_state.skill_nudge),
        mcp_manager: agent_state.mcp_manager.as_ref(),
        global_home_dir: Some(&state.global_home_dir),
        is_callback_turn: false,
        settings: Some(&agent_state.settings),
        trace_id: Some(task_id.to_string()),
        correlated_task_id: None,
        internal: false,
        pr_reviews_posted: Some(&state.pr_reviews_posted),
        stream_ctx,
    };

    match agent::run_agent(&params).await {
        Ok(output) => Ok(output.text),
        Err(e) => Err(e.to_string()),
    }
}

/// Extract the caller's session id from `message/send` request metadata
/// (mika#2070).
///
/// The key is the protocol crate's `CALLER_SESSION_ID_KEY`. It is advisory: a missing
/// key, a null, or a non-string value all yield `None`, and the caller then
/// mints its own session. Validation of the value itself belongs to
/// `Database::a2a_create_task`, which is the only layer that can check whether
/// this agent owns the named session.
fn caller_session_id(params: &MessageSendParams) -> Option<&str> {
    params
        .metadata
        .as_ref()?
        .get(CALLER_SESSION_ID_KEY)?
        .as_str()
}

/// Extract the caller's declared skill restriction from request metadata
/// (mika#2363).
///
/// Returns the names the caller wants kept, or an empty vector when it declared
/// nothing. Every malformed shape — key absent, `null`, not an array, an array
/// with non-string or blank entries — degrades to "no restriction" rather than
/// to an error: a caller from an older or a newer version of the protocol must
/// not be able to fail a turn with a field this server is free to ignore.
/// Non-string entries inside an otherwise valid array are dropped individually,
/// so one bad element does not discard the caller's whole declaration.
///
/// The names are not validated here. Whether this agent carries a given skill is
/// `SkillRegistry::apply_only_skills`'s question — it is the only layer that
/// holds the registry to answer it.
fn requested_only_skills(params: &MessageSendParams) -> Vec<String> {
    params
        .metadata
        .as_ref()
        .and_then(|m| m.get(ONLY_SKILLS_KEY))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Refuse a request that could not get the agent lock, and make the refusal
/// visible (mika#2163 AC8).
///
/// `err` already carries the wire shape — `-32000` with `data.reason` on the
/// bounded path, `-32603 "Agent is busy"` verbatim when the kill-switch is off.
/// This function does not decide the refusal, only records it: an audit row, a
/// WARN line, and the JSON-RPC response.
async fn refuse_busy(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    id: Option<JsonRpcId>,
    err: JsonRpcError,
    port: &'static str,
) -> Response {
    let agent_label = agent_state.db.agent_id().to_string();
    let reason = err
        .data
        .as_ref()
        .and_then(|d| d.get("reason"))
        .and_then(|v| v.as_str())
        // No `data` means the kill-switch path produced this refusal.
        .unwrap_or("queue_disabled")
        .to_string();
    let code = err.code;
    a2a_wait_queue::emit_a2a_queue_audit(
        state,
        agent_state,
        &agent_label,
        "a2a_queue_reject",
        // Throttle per reason: a flood of `queue_full` must not bury the one
        // `wait_timeout` that explains the incident.
        Some(&reason),
        &reason,
        &format!("/a2a {port} refused: reason={reason}, code={code}"),
    )
    .await;
    tracing::warn!(
        agent = %agent_label,
        port,
        reason = %reason,
        code,
        "a2a_queue_reject"
    );
    Json(JsonRpcResponse::error(id, err)).into_response()
}

/// Record a wait that actually happened (mika#2163 AC8).
///
/// A zero-length wait is the nominal case and is not worth a row; anything above
/// it is the contention this ticket exists to make legible.
async fn note_wait(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    waited_ms: u64,
    port: &'static str,
) {
    if waited_ms == 0 {
        return;
    }
    let agent_label = agent_state.db.agent_id().to_string();
    a2a_wait_queue::emit_a2a_queue_audit(
        state,
        agent_state,
        &agent_label,
        "a2a_queue_wait",
        // No discriminator: `wait_ms` has unbounded cardinality, and keying the
        // throttle on it would defeat the throttle entirely.
        None,
        &waited_ms.to_string(),
        &format!("/a2a {port} waited {waited_ms}ms for the agent lock"),
    )
    .await;
    info!(
        agent = %agent_label,
        port,
        wait_ms = waited_ms,
        "a2a_queue_wait"
    );
}

/// The text a completed turn that produced none is served with.
///
/// `message/stream` has served this literal since it was written; mika#2270 gives
/// `message/send` the same semantics, and the shared constant is what makes "the
/// same" checkable rather than a coincidence between two string literals.
const COMPLETED_WITHOUT_TEXT: &str = "Task completed.";

/// What the agent loop left in `handle_message_send`'s hand.
///
/// Named rather than an `Option<Option<String>>` because the three states are
/// semantically distinct and the nesting reads as a mistake: the loop failing, the
/// loop producing nothing, and the loop producing text are three different things
/// to do with a rebuilt Task.
enum TurnText {
    /// The loop finished. `Some` when it produced text of its own.
    Produced(Option<String>),
    /// The loop failed; the task is `failed` and there is no answer to serve.
    LoopFailed,
}

/// What `message/send` must do about the Task it has just rebuilt (mika#2270).
///
/// Kept pure so the three outcomes can be asserted without a server, a database
/// or a network — including the one that must change nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContentNet {
    /// The Task carries readable text. Serve it untouched: byte-identical to the
    /// pre-mika#2270 response, which is what AC3 pins.
    Nominal,
    /// The Task is empty and the loop produced nothing either. Serve the same
    /// literal `message/stream` serves. **Not** a loss — this is the shape of a
    /// turn that legitimately had nothing to say, and counting it as a loss would
    /// make the WARN useless by burying it in false positives.
    MuteTurn,
    /// The Task is empty and the loop's own answer is still in hand. Serve it, and
    /// say so: this is mika#2270's founding failure, the 43 108 input tokens paid
    /// for and thrown away.
    Rescued { text: String },
}

/// Decide the net from the rebuilt Task and the loop's own answer.
///
/// The emptiness predicate is [`mika_a2a::render::render_task_text`] — the very
/// function the CLI renders with. That sharing is load-bearing: a server-side
/// approximation could call a Task fine while the client found nothing readable,
/// which is precisely the gap this net exists to close.
///
/// Whitespace-only loop text counts as no text. It cannot be served (the renderer
/// would reject it right back) and reporting it as a rescued answer would be a
/// false positive on the one signal that must stay trustworthy.
fn decide_content_net(task: &Task, loop_text: Option<&str>) -> ContentNet {
    if mika_a2a::render::render_task_text(task).is_ok() {
        return ContentNet::Nominal;
    }
    match loop_text.filter(|t| !t.trim().is_empty()) {
        Some(text) => ContentNet::Rescued {
            text: text.to_string(),
        },
        None => ContentNet::MuteTurn,
    }
}

/// The facts that decide mika#2270's cause at the first recurrence — and only
/// those.
///
/// A WARN that repeats the symptom teaches nothing. These fields exist to
/// separate the two hypotheses `a2a_get_messages`' predicate leaves open (see
/// `Database::a2a_message_census`), so the follow-up ticket can be opened on a
/// measurement rather than on a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ContentLostReport {
    task_id: String,
    context_id: Option<String>,
    session_id: String,
    /// The id the agent loop stamped its rows with. Equal to `task_id` today —
    /// `run_a2a_agent` passes it as `AgentParams.trace_id` — and reported anyway,
    /// because that equality is exactly what a future reader has to be able to
    /// check rather than assume.
    trace_id: String,
    /// Conversational rows this session holds; `None` when the census itself failed.
    messages_for_session: Option<i64>,
    /// Conversational rows carrying this trace id; `None` when the census failed.
    messages_for_trace: Option<i64>,
    /// Byte length of the text the net saved.
    saved_text_len: usize,
}

impl ContentLostReport {
    /// One line for the `audit_events` row, so the SQL surface carries the same
    /// facts as the log without a join.
    fn reasoning(&self) -> String {
        fn count(value: Option<i64>) -> String {
            value.map_or_else(|| "unreadable".to_string(), |n| n.to_string())
        }
        format!(
            "message/send rebuilt an empty Task for a turn that produced {} bytes of text; \
             served from the loop's own answer. session={}, trace={}, context={}, \
             messages_for_session={}, messages_for_trace={}",
            self.saved_text_len,
            self.session_id,
            self.trace_id,
            self.context_id.as_deref().unwrap_or("none"),
            count(self.messages_for_session),
            count(self.messages_for_trace),
        )
    }
}

/// Make sure a terminal Task served by `message/send` carries text (mika#2270).
///
/// `run_a2a_agent` returns the turn's text, and this port used to discard it with
/// `Ok(_)` before rebuilding the Task from the database — so when that rebuild came
/// back empty, the answer was lost while still in the process's hand. The
/// `message/stream` port never had the defect: it serves the returned text
/// directly.
///
/// After this call, a Task served here always carries at least one non-empty slice.
/// That guarantee is what makes the CLI's fail-closed rendering safe: with the
/// "completed but empty" class removed from this port, the client can treat every
/// empty Task as a loss without ever misreading a legitimately mute turn.
///
/// **The net does not hide the defect, and that restraint is the point of the
/// brick.** A silent repair would turn the loop green and make the fault
/// permanently invisible — building the next occurrence of mika#2270. Hence the
/// WARN and the audit row, carrying the facts that settle the cause.
///
/// SOLE WRITER of `a2a_send_task_content_lost`, in the log and in `audit_events`.
/// Its absence under a symptom is therefore information: it says the loss is not
/// here. Reusing the name elsewhere would destroy exactly that property.
async fn ensure_send_task_carries_text(
    agent_state: &Arc<AgentState>,
    task: &mut Task,
    loop_text: Option<&str>,
    session_id: &str,
) {
    let (text, saved_len) = match decide_content_net(task, loop_text) {
        ContentNet::Nominal => return,
        ContentNet::MuteTurn => (COMPLETED_WITHOUT_TEXT.to_string(), None),
        ContentNet::Rescued { text } => {
            let len = text.len();
            (text, Some(len))
        }
    };

    let task_id = task.id.clone();
    let context_id = task.context_id.clone();
    task.status.message = Some(agent_text_message(&task_id, context_id.clone(), text));

    // A mute turn is served, not reported: nothing was produced, so nothing was
    // lost.
    let Some(saved_text_len) = saved_len else {
        return;
    };

    let census = agent_state
        .db
        .a2a_message_census(session_id, &task_id)
        .await;
    let (messages_for_session, messages_for_trace) = match census {
        Ok(c) => (Some(c.session_rows), Some(c.trace_rows)),
        Err(e) => {
            // An unreadable census must not swallow the loss report — the report
            // is the load-bearing half, the counts only sharpen it.
            error!(error = %e, task_id = %task_id, "a2a_send_task_census_unreadable");
            (None, None)
        }
    };

    let report = ContentLostReport {
        task_id,
        context_id,
        session_id: session_id.to_string(),
        trace_id: task.id.clone(),
        messages_for_session,
        messages_for_trace,
        saved_text_len,
    };

    tracing::warn!(
        task_id = %report.task_id,
        context_id = report.context_id.as_deref().unwrap_or("none"),
        session_id = %report.session_id,
        trace_id = %report.trace_id,
        messages_for_session = ?report.messages_for_session,
        messages_for_trace = ?report.messages_for_trace,
        saved_text_len = report.saved_text_len,
        "a2a_send_task_content_lost"
    );

    if let Err(e) = agent_state
        .db
        .log_audit_event(
            &report.session_id,
            "a2a_send_task_content_lost",
            &format!("a2a_task:{}", report.task_id),
            None,
            Some(&report.saved_text_len.to_string()),
            Some(&report.reasoning()),
            Some(&report.trace_id),
        )
        .await
    {
        tracing::warn!(error = %e, task_id = %report.task_id, "a2a_send_task_content_lost_audit_failed");
    }
}

/// An agent-role message carrying one text part.
fn agent_text_message(task_id: &str, context_id: Option<String>, text: String) -> Message {
    Message {
        message_id: Uuid::new_v4().to_string(),
        role: Role::Agent,
        parts: vec![Part::Text {
            text,
            metadata: None,
        }],
        context_id,
        task_id: Some(task_id.to_string()),
        metadata: None,
        reference_task_ids: None,
        extensions: None,
        kind: "message".to_string(),
    }
}

/// Handle `message/send` — synchronous message processing via the real agent loop.
async fn handle_message_send(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: MessageSendParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    let task_id = Uuid::new_v4().to_string();
    let context_id = params.message.context_id.clone();
    let caller_session = caller_session_id(&params).map(str::to_string);
    let return_immediately = params
        .configuration
        .as_ref()
        .and_then(|c| c.return_immediately)
        .unwrap_or(false);

    // Bounded wait for the agent lock (mika#2163). Take a place in the line, then
    // wait in it. The wait lives in the handler because `message/send` is
    // synchronous — the caller is holding the connection open for the completed
    // Task, so there is nothing to return early with. A client that disconnects
    // mid-wait drops this future, which drops both the lock wait and the place.
    //
    // **`returnImmediately` is exempt, and the exemption is new.** That branch
    // creates the task row and returns it in `submitted`; it never runs the agent
    // loop, so it never needs the lock that exists to serialise turns. Taking the
    // lock there was harmless before mika#2163 — a `try_lock`, granted or refused
    // in microseconds. Under a wait it stops being harmless: a fire-and-forget
    // caller would park for the whole budget, and hold one of the places in front
    // of callers that do need a turn, in order to make two database calls.
    let _lock_guard = if return_immediately {
        None
    } else {
        let slot =
            match a2a_wait_queue::try_take_slot(&agent_state.a2a_wait_slots, &agent_state.settings)
            {
                Ok(slot) => slot,
                Err(err) => {
                    return refuse_busy(state, agent_state, request.id.clone(), err, "send").await;
                }
            };
        match a2a_wait_queue::wait_for_agent_lock(
            Arc::clone(&agent_state.agent_lock),
            slot,
            &agent_state.settings,
        )
        .await
        {
            Ok(acquired) => {
                note_wait(state, agent_state, acquired.waited_ms, "send").await;
                Some(acquired.guard)
            }
            Err(err) => {
                return refuse_busy(state, agent_state, request.id.clone(), err, "send").await;
            }
        }
    };

    // Create task in DB (creates task row, session, and mapping)
    let session_id = match agent_state
        .db
        .a2a_create_task(&task_id, context_id.as_deref(), caller_session.as_deref())
        .await
    {
        Ok(sid) => sid,
        Err(e) => {
            error!(error = %e, "failed to create A2A task");
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response();
        }
    };

    if return_immediately {
        // Return task in submitted state immediately
        match agent_state.db.a2a_build_task(&task_id, None).await {
            Ok(Some(task)) => {
                let result = serde_json::to_value(&task).unwrap_or_default();
                Json(JsonRpcResponse::success(request.id, result)).into_response()
            }
            Ok(None) => Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(TASK_NOT_FOUND),
            ))
            .into_response(),
            Err(e) => {
                error!(error = %e, "failed to build A2A task");
                Json(JsonRpcResponse::error(
                    request.id,
                    JsonRpcError::from_code(INTERNAL_ERROR),
                ))
                .into_response()
            }
        }
    } else {
        // Process synchronously: transition to working, run agent loop, return completed task
        let _ = agent_state
            .db
            .a2a_update_task_state(&task_id, "working")
            .await;

        let input_text = extract_text_from_parts(&params.message.parts);

        // Run the real agent loop. Non-streaming `message/send` path — no
        // broadcast subscriber, so pass `None`.
        //
        // mika#2270: the returned text is KEPT. It used to be dropped with `Ok(_)`
        // right here, one line before the Task was rebuilt from the database — so
        // an empty rebuild lost an answer the process was still holding.
        let only_skills = requested_only_skills(&params);

        let turn_text = match run_a2a_agent(
            state,
            agent_state,
            &session_id,
            &input_text,
            &task_id,
            None,
            &only_skills,
        )
        .await
        {
            Ok(text) => {
                let _ = agent_state
                    .db
                    .a2a_update_task_state(&task_id, "completed")
                    .await;

                info!(task_id = %task_id, "A2A task completed via agent loop");
                TurnText::Produced(text)
            }
            Err(e) => {
                error!(error = %e, task_id = %task_id, "A2A agent loop failed");
                let _ = agent_state
                    .db
                    .a2a_update_task_state(&task_id, "failed")
                    .await;
                TurnText::LoopFailed
            }
        };

        match agent_state
            .db
            .a2a_build_task(
                &task_id,
                params.configuration.as_ref().and_then(|c| c.history_length),
            )
            .await
        {
            Ok(Some(mut task)) => {
                // A failed loop is served as `failed` with nothing to say — the
                // client turns that state into a named error of its own, so the net
                // has neither a text to serve nor a loss to report.
                if let TurnText::Produced(text) = &turn_text {
                    ensure_send_task_carries_text(
                        agent_state,
                        &mut task,
                        text.as_deref(),
                        &session_id,
                    )
                    .await;
                }
                let result = serde_json::to_value(&task).unwrap_or_default();
                Json(JsonRpcResponse::success(request.id, result)).into_response()
            }
            Ok(None) => Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(TASK_NOT_FOUND),
            ))
            .into_response(),
            Err(e) => {
                error!(error = %e, "failed to build A2A task");
                Json(JsonRpcResponse::error(
                    request.id,
                    JsonRpcError::from_code(INTERNAL_ERROR),
                ))
                .into_response()
            }
        }
    }
}

/// Handle `message/stream` — SSE streaming response via the real agent loop.
async fn handle_message_stream(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: MessageSendParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    // Bounded wait for the agent lock (mika#2163), split across the two halves of
    // this handler on purpose:
    //
    //   * the **place in the line** is taken here, before the spawn — take it
    //     inside the spawned task instead and the spawn itself is unbounded, so
    //     the backpressure would be decorative;
    //   * the **wait** happens inside the spawned task, so the SSE stream opens
    //     immediately and the caller sees an open, waiting stream rather than a
    //     silent connection that only speaks once the lock frees.
    //
    // Saturation is therefore still answerable with a JSON-RPC error: the stream
    // is not open yet at this point.
    let slot =
        match a2a_wait_queue::try_take_slot(&agent_state.a2a_wait_slots, &agent_state.settings) {
            Ok(slot) => slot,
            Err(err) => {
                return refuse_busy(state, agent_state, request.id.clone(), err, "stream").await;
            }
        };

    // AC5 on this port, and it needs its own shape. With the kill-switch off the
    // refusal must be the one this handler gave before mika#2163: a JSON-RPC
    // `-32603` **response**, with no task row, no broadcaster, no SSE stream and
    // no spawn. Deferring the `try_lock_owned()` into the spawned task like the
    // enabled path does would answer `200 OK` with an open stream that later
    // carries a `failed` frame — a different wire contract, reached by an operator
    // who believes they turned the feature off. So the legacy attempt happens
    // here, at the same point in the handler it always did, and only the bounded
    // path is deferred.
    let entry = match slot {
        WaitSlot::Disabled => match agent_state.agent_lock.clone().try_lock_owned() {
            Ok(guard) => StreamLockEntry::Held(guard),
            Err(_) => {
                return refuse_busy(
                    state,
                    agent_state,
                    request.id.clone(),
                    a2a_wait_queue::legacy_busy_error(),
                    "stream",
                )
                .await;
            }
        },
        queued => StreamLockEntry::Waiting(queued),
    };

    let task_id = Uuid::new_v4().to_string();
    let context_id = params.message.context_id.clone();
    let caller_session = caller_session_id(&params).map(str::to_string);

    // Create task in DB
    let session_id = match agent_state
        .db
        .a2a_create_task(&task_id, context_id.as_deref(), caller_session.as_deref())
        .await
    {
        Ok(sid) => sid,
        Err(e) => {
            error!(error = %e, "failed to create A2A task");
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response();
        }
    };

    // Create broadcast channel for this task
    let (tx, rx) = broadcast::channel::<StreamEvent>(32);
    let task_id_clone = task_id.clone();

    // Store broadcaster
    state.a2a_broadcasters.insert(task_id.clone(), tx.clone());

    // Spawn task processing with real agent loop
    let state_clone = state.clone();
    let agent_state_clone = Arc::clone(agent_state);
    let input_text = extract_text_from_parts(&params.message.parts);
    // mika#2363: `message/stream` shares `MessageSendParams` with `message/send`,
    // so it reads the same key. Honouring it on one port and ignoring it on the
    // other would make the field mean different things on two endpoints of the
    // same protocol, which is worse than not having it.
    let only_skills = requested_only_skills(&params);
    let broadcasters = Arc::clone(&state.a2a_broadcasters);
    tokio::spawn(async move {
        let _broadcaster_guard = BroadcasterGuard {
            map: broadcasters,
            key: task_id_clone.clone(),
        };
        let turn = StreamTurn {
            state: state_clone,
            agent_state: agent_state_clone,
            session_id,
            input_text,
            task_id: task_id_clone,
            context_id,
            tx,
            only_skills,
        };

        // The kill-switch path already holds the lock — it was taken in the
        // handler, at the same point it always was — so there is nothing here to
        // wait for and nothing to abandon.
        let slot = match entry {
            StreamLockEntry::Held(guard) => {
                run_a2a_stream_turn(guard, turn).await;
                return;
            }
            StreamLockEntry::Waiting(slot) => slot,
        };

        // AC7 — race the lock wait against the caller going away.
        //
        // A spawned task is not cancelled when the client disconnects, so without
        // this race a caller who hangs up mid-wait would still acquire the lock
        // and run a full agent turn nobody is reading — a turn paid for, and a
        // place held in front of everyone behind it. `tx.closed()` completes when
        // the last SSE receiver is dropped (tokio 1.53.1 `broadcast.rs:919`);
        // hyper drops it once it notices the peer is gone, and the stream's
        // `KeepAlive` bounds how long that takes by making it write periodically.
        //
        // The abandonment therefore lands BEFORE `agent_lock` is acquired, not
        // after: dropping the wait future gives up the place in the mutex's FIFO
        // and returns the queue permit.
        let acquired = tokio::select! {
            biased;
            _ = turn.tx.closed() => {
                info!(task_id = %turn.task_id, "a2a_queue_abandoned");
                let _ = turn
                    .agent_state
                    .db
                    .a2a_update_task_state(&turn.task_id, "canceled")
                    .await;
                return;
            }
            result = a2a_wait_queue::wait_for_agent_lock(
                Arc::clone(&turn.agent_state.agent_lock),
                slot,
                &turn.agent_state.settings,
            ) => result,
        };

        let guard = match acquired {
            Ok(a) => {
                note_wait(&turn.state, &turn.agent_state, a.waited_ms, "stream").await;
                a.guard
            }
            Err(err) => {
                // The stream is already open by now, so the refusal cannot travel
                // as a JSON-RPC error; it travels as a final `failed` frame.
                let reason = err
                    .data
                    .as_ref()
                    .and_then(|d| d.get("reason"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("queue_disabled")
                    .to_string();
                let agent_label = turn.agent_state.db.agent_id().to_string();
                a2a_wait_queue::emit_a2a_queue_audit(
                    &turn.state,
                    &turn.agent_state,
                    &agent_label,
                    "a2a_queue_reject",
                    Some(&reason),
                    &reason,
                    &format!(
                        "/a2a stream refused after waiting: reason={reason}, code={}",
                        err.code
                    ),
                )
                .await;
                tracing::warn!(
                    agent = %agent_label,
                    port = "stream",
                    reason = %reason,
                    code = err.code,
                    task_id = %turn.task_id,
                    "a2a_queue_reject"
                );
                let _ = turn
                    .agent_state
                    .db
                    .a2a_update_task_state(&turn.task_id, "failed")
                    .await;
                let _ = turn
                    .tx
                    .send(StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
                        task_id: turn.task_id.clone(),
                        context_id: turn.context_id.clone(),
                        status: TaskStatus {
                            state: TaskState::Failed,
                            message: Some(Message {
                                message_id: Uuid::new_v4().to_string(),
                                role: Role::Agent,
                                parts: vec![Part::Text {
                                    text: err.message.clone(),
                                    metadata: None,
                                }],
                                context_id: turn.context_id.clone(),
                                task_id: Some(turn.task_id.clone()),
                                // `error.data` is a JSON object by construction
                                // (`a2a_wait_queue::busy_error`); carry its fields so
                                // a streaming caller gets the same `reason` /
                                // `retry_after_ms` / `queue_depth` a `message/send`
                                // caller reads off the JSON-RPC error.
                                metadata: err.data.as_ref().and_then(|d| {
                                    d.as_object().map(|m| {
                                        m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                                    })
                                }),
                                reference_task_ids: None,
                                extensions: None,
                                kind: "message".to_string(),
                            }),
                            timestamp: Some(chrono::Utc::now().to_rfc3339()),
                        },
                        is_final: true,
                        metadata: None,
                    }));
                return;
            }
        };

        run_a2a_stream_turn(guard, turn).await;
    });

    // Return SSE stream
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx);
    let event_stream = tokio_stream::StreamExt::map(stream, |event| {
        let ev = match event {
            Ok(stream_event) => {
                let data = serde_json::to_string(&stream_event).unwrap_or_default();
                Event::default().data(data)
            }
            Err(_) => {
                // Channel closed
                Event::default().data("")
            }
        };
        Ok::<_, Infallible>(ev)
    });

    Sse::new(event_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Everything a `message/stream` turn needs once the agent lock is in hand.
///
/// Bundled rather than passed loose because both paths into the turn — the
/// kill-switch path, which takes the lock in the handler, and the bounded path,
/// which waits for it in the spawned task — hand over the same seven values.
struct StreamTurn {
    state: AppState,
    agent_state: Arc<AgentState>,
    session_id: String,
    input_text: String,
    task_id: String,
    context_id: Option<String>,
    tx: broadcast::Sender<StreamEvent>,
    /// Per-turn skill restriction (mika#2363), empty when the caller declared
    /// none. Carried here rather than re-read in the spawned task because
    /// `params` does not survive the spawn.
    only_skills: Vec<String>,
}

/// Run the streaming turn. The guard is taken by value and dropped with this
/// future, so the agent lock is held for exactly the turn's lifetime.
async fn run_a2a_stream_turn(_lock_guard: OwnedMutexGuard<()>, turn: StreamTurn) {
    let StreamTurn {
        state,
        agent_state,
        session_id,
        input_text,
        task_id,
        context_id,
        tx,
        only_skills,
    } = turn;

    // Transition to working
    let _ = agent_state
        .db
        .a2a_update_task_state(&task_id, "working")
        .await;

    let _ = tx.send(StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
        task_id: task_id.clone(),
        context_id: context_id.clone(),
        status: TaskStatus {
            state: TaskState::Working,
            message: None,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        },
        is_final: false,
        metadata: None,
    }));

    // Run the real agent loop. Streaming path (mika#1731 wire, mika#1757
    // emission) — bundle the per-task broadcaster + task_id + context_id
    // into a ToolCallStreamContext so process_tool_calls can inject
    // ToolCallStart / ToolCallResult frames as tools dispatch. The Arc
    // wrapper enables cheap threading through run_loop.
    let stream_ctx_for_agent: Option<Arc<mika_a2a::streaming::ToolCallStreamContext>> =
        Some(Arc::new(mika_a2a::streaming::ToolCallStreamContext::new(
            Arc::new(tx.clone()),
            task_id.clone(),
            context_id.clone(),
        )));
    match run_a2a_agent(
        &state,
        &agent_state,
        &session_id,
        &input_text,
        &task_id,
        stream_ctx_for_agent,
        &only_skills,
    )
    .await
    {
        Ok(response_text) => {
            // mika#2270: the literal is shared with `message/send`, which now
            // serves the same one for a turn that produced no text. Two ports
            // answering the same situation identically is a property worth being
            // able to check.
            let text = response_text.unwrap_or_else(|| COMPLETED_WITHOUT_TEXT.to_string());
            let response_message = agent_text_message(&task_id, context_id.clone(), text);

            let _ = agent_state
                .db
                .a2a_update_task_state(&task_id, "completed")
                .await;

            // Send completion event
            let _ = tx.send(StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
                task_id: task_id.clone(),
                context_id,
                status: TaskStatus {
                    state: TaskState::Completed,
                    message: Some(response_message),
                    timestamp: Some(chrono::Utc::now().to_rfc3339()),
                },
                is_final: true,
                metadata: None,
            }));
        }
        Err(e) => {
            error!(error = %e, task_id = %task_id, "A2A streaming agent loop failed");
            let _ = agent_state
                .db
                .a2a_update_task_state(&task_id, "failed")
                .await;

            let _ = tx.send(StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
                task_id: task_id.clone(),
                context_id,
                status: TaskStatus {
                    state: TaskState::Failed,
                    message: None,
                    timestamp: Some(chrono::Utc::now().to_rfc3339()),
                },
                is_final: true,
                metadata: None,
            }));
        }
    }
}

/// Handle `tasks/get`.
async fn handle_tasks_get(agent_state: &Arc<AgentState>, request: JsonRpcRequest) -> Response {
    let params: TaskQueryParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    // Resolve by task id first, then — only on a miss — as a caller-supplied
    // `context_id` (mika#2036).
    //
    // A caller whose `message/send` lost its response never learned the task id:
    // it is minted here with `Uuid::new_v4` and travels back only in the
    // envelope that was lost. The `context_id` the caller chose *is* known to it
    // and is already persisted on the mapping row, so it is the only handle that
    // survives the failure.
    //
    // The order is load-bearing: a real task id is looked up first and can
    // therefore never be shadowed by some other task's context that happens to
    // reuse its spelling. This widens what resolves; it changes nothing that
    // resolved before.
    let resolved = match agent_state
        .db
        .a2a_build_task(&params.id, params.history_length)
        .await
    {
        Ok(Some(task)) => Ok(Some(task)),
        Ok(None) => match agent_state.db.a2a_find_task_id_by_context(&params.id).await {
            Ok(Some(task_id)) => {
                debug!(
                    context_id = %params.id,
                    task_id = %task_id,
                    "tasks/get resolved via context id"
                );
                agent_state
                    .db
                    .a2a_build_task(&task_id, params.history_length)
                    .await
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    };

    match resolved {
        Ok(Some(task)) => {
            let result = serde_json::to_value(&task).unwrap_or_default();
            Json(JsonRpcResponse::success(request.id, result)).into_response()
        }
        Ok(None) => Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::from_code(TASK_NOT_FOUND),
        ))
        .into_response(),
        Err(e) => {
            error!(error = %e, "failed to get A2A task");
            Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response()
        }
    }
}

/// Handle `tasks/cancel`.
async fn handle_tasks_cancel(agent_state: &Arc<AgentState>, request: JsonRpcRequest) -> Response {
    let params: TaskIdParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    let state_str = match agent_state.db.a2a_get_task_state(&params.id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(TASK_NOT_FOUND),
            ))
            .into_response();
        }
        Err(e) => {
            error!(error = %e, "failed to get A2A task state");
            return Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response();
        }
    };

    let current_state: TaskState =
        match serde_json::from_value(serde_json::Value::String(state_str)) {
            Ok(s) => s,
            Err(_) => {
                return Json(JsonRpcResponse::error(
                    request.id,
                    JsonRpcError::from_code(INTERNAL_ERROR),
                ))
                .into_response();
            }
        };

    if !TaskStateMachine::can_transition(current_state, TaskState::Canceled) {
        return Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::from_code(TASK_NOT_CANCELABLE),
        ))
        .into_response();
    }

    if let Err(e) = agent_state
        .db
        .a2a_update_task_state(&params.id, "canceled")
        .await
    {
        error!(error = %e, "failed to cancel A2A task");
        return Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::from_code(INTERNAL_ERROR),
        ))
        .into_response();
    }

    match agent_state.db.a2a_build_task(&params.id, None).await {
        Ok(Some(task)) => {
            let result = serde_json::to_value(&task).unwrap_or_default();
            Json(JsonRpcResponse::success(request.id, result)).into_response()
        }
        Ok(None) => Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::from_code(TASK_NOT_FOUND),
        ))
        .into_response(),
        Err(e) => {
            error!(error = %e, "failed to build A2A task");
            Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response()
        }
    }
}

/// Handle `tasks/pushNotificationConfig/set`.
async fn handle_push_config_set(
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let config: mika_a2a::types::TaskPushNotificationConfig =
        match serde_json::from_value(request.params.clone()) {
            Ok(c) => c,
            Err(e) => {
                return Json(JsonRpcResponse::error(
                    request.id.clone(),
                    JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
                ))
                .into_response();
            }
        };

    if let Err(e) = agent_state.db.a2a_set_push_config(&config).await {
        error!(error = %e, "failed to set push config");
        return Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::from_code(INTERNAL_ERROR),
        ))
        .into_response();
    }

    let result = serde_json::to_value(&config).unwrap_or_default();
    Json(JsonRpcResponse::success(request.id, result)).into_response()
}

/// Handle `tasks/pushNotificationConfig/get`.
async fn handle_push_config_get(
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: TaskIdParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    match agent_state.db.a2a_get_push_config(&params.id).await {
        Ok(Some(config)) => {
            let result = serde_json::to_value(&config).unwrap_or_default();
            Json(JsonRpcResponse::success(request.id, result)).into_response()
        }
        Ok(None) => Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::with_message(TASK_NOT_FOUND, "Push notification config not found"),
        ))
        .into_response(),
        Err(e) => {
            error!(error = %e, "failed to get push config");
            Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response()
        }
    }
}

/// Handle `tasks/pushNotificationConfig/list`.
async fn handle_push_config_list(
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: TaskIdParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    match agent_state.db.a2a_list_push_configs(&params.id).await {
        Ok(configs) => {
            let result = serde_json::to_value(&configs).unwrap_or_default();
            Json(JsonRpcResponse::success(request.id, result)).into_response()
        }
        Err(e) => {
            error!(error = %e, "failed to list push configs");
            Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response()
        }
    }
}

/// Handle `tasks/pushNotificationConfig/delete`.
async fn handle_push_config_delete(
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: TaskIdParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    match agent_state.db.a2a_delete_push_config(&params.id).await {
        Ok(true) => {
            let result = serde_json::json!({"id": params.id, "deleted": true});
            Json(JsonRpcResponse::success(request.id, result)).into_response()
        }
        Ok(false) => Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::with_message(TASK_NOT_FOUND, "Push notification config not found"),
        ))
        .into_response(),
        Err(e) => {
            error!(error = %e, "failed to delete push config");
            Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response()
        }
    }
}

/// Handle `tasks/resubscribe` — reconnect to an existing task's SSE stream.
async fn handle_tasks_resubscribe(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: TaskIdParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    // Check task exists
    match agent_state.db.a2a_get_task_state(&params.id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(TASK_NOT_FOUND),
            ))
            .into_response();
        }
        Err(e) => {
            error!(error = %e, "failed to get A2A task state");
            return Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response();
        }
    }

    // Get or create broadcaster
    let rx = match state.a2a_broadcasters.get(&params.id) {
        Some(tx) => tx.subscribe(),
        None => {
            // Task exists but no active broadcaster — task might be in terminal state
            // Return current task state as a single SSE event
            match agent_state.db.a2a_build_task(&params.id, None).await {
                Ok(Some(task)) => {
                    let stream = tokio_stream::once(Ok::<_, Infallible>(Event::default().data(
                        serde_json::to_string(&StreamEvent::Task(task)).unwrap_or_default(),
                    )));
                    return Sse::new(stream).into_response();
                }
                _ => {
                    return Json(JsonRpcResponse::error(
                        request.id,
                        JsonRpcError::from_code(TASK_NOT_FOUND),
                    ))
                    .into_response();
                }
            }
        }
    };

    let stream = tokio_stream::wrappers::BroadcastStream::new(rx);
    let event_stream = tokio_stream::StreamExt::map(stream, |event| {
        let ev = match event {
            Ok(stream_event) => {
                let data = serde_json::to_string(&stream_event).unwrap_or_default();
                Event::default().data(data)
            }
            Err(_) => Event::default().data(""),
        };
        Ok::<_, Infallible>(ev)
    });

    Sse::new(event_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Handle GET request for the Agent Card.
pub async fn handle_agent_card(
    State(state): State<AppState>,
    Path(agent_name): Path<String>,
) -> Response {
    let agent_state = match state.resolve_agent(&agent_name).await {
        Some(a) => a,
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
                .into_response();
        }
    };

    let skills = agent_state.skills.lock().unwrap().clone();
    let card = build_agent_card(&agent_name, "Mika AI Agent", &skills, &state.gateway_url);

    Json(card).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mika_a2a::types::{Message, Part, Role};
    use std::collections::HashMap;

    fn params_with_metadata(
        metadata: Option<HashMap<String, serde_json::Value>>,
    ) -> MessageSendParams {
        MessageSendParams {
            message: Message {
                message_id: "msg-1".to_string(),
                role: Role::User,
                parts: vec![Part::Text {
                    text: "hello".to_string(),
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
            metadata,
        }
    }

    fn with_key(value: serde_json::Value) -> MessageSendParams {
        params_with_metadata(Some(HashMap::from([(
            CALLER_SESSION_ID_KEY.to_string(),
            value,
        )])))
    }

    #[test]
    fn caller_session_id_is_read_from_request_metadata() {
        let params = with_key(serde_json::Value::String("rt005-c1-r7".to_string()));
        assert_eq!(caller_session_id(&params), Some("rt005-c1-r7"));
    }

    #[test]
    fn caller_session_id_is_absent_without_metadata() {
        assert_eq!(caller_session_id(&params_with_metadata(None)), None);
    }

    #[test]
    fn caller_session_id_is_absent_when_the_key_is_missing() {
        let params = params_with_metadata(Some(HashMap::from([(
            "something.else".to_string(),
            serde_json::Value::String("s1".to_string()),
        )])));
        assert_eq!(caller_session_id(&params), None);
    }

    // --- mika#2270: `message/send` stops throwing away the answer it holds -----

    fn completed_task(status_message: Option<Message>) -> Task {
        Task {
            id: "951dc60c-23cd-420d-8981-cdc0b7fd91be".to_string(),
            context_id: Some("ctx-2266".to_string()),
            status: TaskStatus {
                state: TaskState::Completed,
                message: status_message,
                timestamp: None,
            },
            artifacts: None,
            history: None,
            metadata: None,
            kind: "task".to_string(),
        }
    }

    fn agent_reply(text: &str) -> Message {
        agent_text_message(
            "951dc60c-23cd-420d-8981-cdc0b7fd91be",
            None,
            text.to_string(),
        )
    }

    /// **AC1.** The measured mika#2270 shape: the loop produced a full plan review
    /// and the rebuilt Task came back empty. The text is in hand, so it is served.
    #[test]
    fn a_turn_whose_task_rebuilt_empty_is_served_from_the_loop_text() {
        let task = completed_task(None);
        assert_eq!(
            decide_content_net(&task, Some("Disposition: READY")),
            ContentNet::Rescued {
                text: "Disposition: READY".to_string()
            }
        );
    }

    /// **AC3.** The nominal case must decide *nothing*. This is the assertion that
    /// keeps the net from biting where it should not: a correctable failure on the
    /// return channel that alters healthy replies is worse than the silence it
    /// replaces.
    #[test]
    fn a_task_that_already_carries_content_is_left_alone() {
        let task = completed_task(Some(agent_reply("Disposition: READY")));
        assert_eq!(
            decide_content_net(&task, Some("ignored")),
            ContentNet::Nominal
        );
        // …and equally when the loop returned nothing: the Task is the answer.
        assert_eq!(decide_content_net(&task, None), ContentNet::Nominal);
    }

    /// **AC6.** A turn that produced no text is not a loss. It is served with the
    /// same literal `message/stream` serves, which is what removes the whole
    /// "completed but empty" class from this port — and therefore what lets the CLI
    /// treat every empty Task as a loss without a false positive.
    #[test]
    fn a_mute_turn_is_served_with_the_stream_literal_and_is_not_a_loss() {
        let task = completed_task(None);
        assert_eq!(decide_content_net(&task, None), ContentNet::MuteTurn);
    }

    /// Whitespace-only text is no text: it could not be served (the renderer would
    /// reject it straight back) and reporting it as a rescue would be a false
    /// positive on the one signal that has to stay trustworthy.
    #[test]
    fn whitespace_only_loop_text_counts_as_no_text() {
        let task = completed_task(None);
        for blank in ["", "   ", "\n\t "] {
            assert_eq!(decide_content_net(&task, Some(blank)), ContentNet::MuteTurn);
        }
    }

    /// **AC2, the report half.** The six diagnostic facts must reach the operator,
    /// including the `trace_id`/`task_id` equality — reported rather than assumed,
    /// because the day it stops holding is the day the census stops meaning
    /// anything.
    #[test]
    fn the_loss_report_carries_the_facts_that_settle_the_cause() {
        let report = ContentLostReport {
            task_id: "951dc60c".to_string(),
            context_id: Some("ctx-2266".to_string()),
            session_id: "probe-a".to_string(),
            trace_id: "951dc60c".to_string(),
            messages_for_session: Some(2),
            messages_for_trace: Some(0),
            saved_text_len: 1481,
        };
        let reasoning = report.reasoning();
        for needle in [
            "probe-a",
            "951dc60c",
            "ctx-2266",
            "messages_for_session=2",
            "messages_for_trace=0",
            "1481",
        ] {
            assert!(reasoning.contains(needle), "missing {needle}: {reasoning}");
        }
    }

    /// An unreadable census must not swallow the report — the counts sharpen the
    /// diagnosis, they are not a precondition for admitting the loss.
    #[test]
    fn an_unreadable_census_still_reports_the_loss() {
        let report = ContentLostReport {
            task_id: "951dc60c".to_string(),
            context_id: None,
            session_id: "probe-a".to_string(),
            trace_id: "951dc60c".to_string(),
            messages_for_session: None,
            messages_for_trace: None,
            saved_text_len: 12,
        };
        let reasoning = report.reasoning();
        assert!(reasoning.contains("messages_for_session=unreadable"));
        assert!(reasoning.contains("messages_for_trace=unreadable"));
        assert!(reasoning.contains("context=none"));
    }

    /// **SOLE WRITER.** `a2a_send_task_content_lost` is written at exactly one
    /// site, in the log and in `audit_events`. That is what makes its *absence*
    /// under a symptom informative — it says the loss is not here. A second writer
    /// would break no test about the decision, which is why this guard is lexical:
    /// the regression would make nothing wrong, only unattributable.
    #[test]
    fn the_loss_signal_has_exactly_one_writer_in_this_module() {
        let source = include_str!("a2a.rs");
        let emissions = source.matches("\"a2a_send_task_content_lost\"").count();
        assert_eq!(
            emissions, 2,
            "expected exactly two emission literals (one WARN, one audit row), found {emissions}"
        );
    }

    #[test]
    fn non_string_caller_session_ids_are_ignored() {
        // A client sending the wrong JSON type must degrade to a minted session,
        // never fail the turn (mika#2070 AC3).
        for value in [
            serde_json::Value::Null,
            serde_json::json!(42),
            serde_json::json!(["s1"]),
        ] {
            assert_eq!(caller_session_id(&with_key(value)), None);
        }
    }

    // --- mika#2363: the per-turn skill restriction (V2, R3, R4) ----------------

    fn with_only_skills(value: serde_json::Value) -> MessageSendParams {
        params_with_metadata(Some(HashMap::from([(ONLY_SKILLS_KEY.to_string(), value)])))
    }

    #[test]
    fn mika2363_only_skills_is_read_from_request_metadata() {
        let params = with_only_skills(serde_json::json!(["mika-arch-groom-ticket"]));
        assert_eq!(
            requested_only_skills(&params),
            vec!["mika-arch-groom-ticket".to_string()]
        );
    }

    #[test]
    fn mika2363_absent_metadata_means_no_restriction() {
        // R3: the byte-for-byte-unchanged path. An empty vector is what
        // `run_a2a_agent` short-circuits on, so this is the assertion that a
        // caller declaring nothing gets the pre-mika#2363 turn.
        assert!(requested_only_skills(&params_with_metadata(None)).is_empty());
        assert!(
            requested_only_skills(&params_with_metadata(Some(HashMap::new()))).is_empty(),
            "an empty metadata map declares nothing"
        );
    }

    #[test]
    fn mika2363_an_empty_array_is_treated_as_absent() {
        // Deliberately NOT read as "keep no skills". Nobody means that by an
        // empty array, and reading it that way would let a serialization quirk
        // silently strip a turn of every skill it has.
        assert!(requested_only_skills(&with_only_skills(serde_json::json!([]))).is_empty());
    }

    #[test]
    fn mika2363_malformed_shapes_degrade_to_no_restriction() {
        // A field this server is free to ignore must never be able to fail a
        // turn — same discipline as `caller_session_id` above.
        for value in [
            serde_json::Value::Null,
            serde_json::json!(42),
            serde_json::json!("mika-arch-groom-ticket"),
            serde_json::json!({"skill": "mika-arch-groom-ticket"}),
        ] {
            assert!(
                requested_only_skills(&with_only_skills(value.clone())).is_empty(),
                "unexpected restriction parsed out of {value}"
            );
        }
    }

    #[test]
    fn mika2363_bad_entries_are_dropped_individually() {
        // One malformed element must not discard the caller's whole declaration:
        // dropping the array would silently restore the triple injection this
        // ticket exists to remove.
        let params = with_only_skills(serde_json::json!([
            "mika-arch-groom-ticket",
            42,
            "  ",
            null,
            "  mika-arch-second-review  "
        ]));
        assert_eq!(
            requested_only_skills(&params),
            vec![
                "mika-arch-groom-ticket".to_string(),
                "mika-arch-second-review".to_string()
            ]
        );
    }

    /// V5 / R4, structural. A restriction is `&mut`, so applying it to the
    /// registry behind `AgentState.skills` would leak one turn's `only_skills`
    /// into every concurrent turn of the same agent. The clone is the mechanism
    /// that prevents it, and its removal would break no assertion anywhere: the
    /// turn would still be correctly restricted, and the *next* turn would be
    /// silently wrong. Hence a lexical guard rather than a behavioural test.
    #[test]
    fn mika2363_the_restriction_is_applied_to_a_clone_and_never_written_back() {
        // Production half only — this test module quotes the same identifiers,
        // and counting its own assertions would make the guard report itself.
        let source = include_str!("a2a.rs")
            .split("\n#[cfg(test)]\n")
            .next()
            .expect("split always yields a first element");

        assert_eq!(
            source.matches("apply_only_skills(").count(),
            1,
            "the restriction must be applied at exactly one site in this module"
        );
        assert!(
            source.contains("let mut restricted = (*skills).clone();"),
            "the restriction must be applied to a per-turn clone of the shared registry"
        );
        // One writer, and it is the hot-reload path — not the restriction.
        let writebacks = source
            .matches("*agent_state.skills.lock().unwrap() =")
            .count();
        assert_eq!(
            writebacks, 1,
            "expected exactly one write to the cached registry (the dirty-reload); \
             found {writebacks} — a restricted registry must never be one of them"
        );
    }
}
