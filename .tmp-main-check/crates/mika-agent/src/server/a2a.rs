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
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use mika_a2a::jsonrpc::{
    A2aMethod, INTERNAL_ERROR, INVALID_PARAMS, JsonRpcError, JsonRpcId, JsonRpcRequest,
    JsonRpcResponse, METHOD_NOT_FOUND, TASK_NOT_CANCELABLE, TASK_NOT_FOUND,
};
use mika_a2a::params::{
    CALLER_SESSION_ID_KEY, EFFECTIVE_MODEL_KEY, MODEL_OVERRIDE_KEY, MessageSendParams,
    ONLY_SKILLS_KEY, RUN_USAGE_KEY, RunUsage, SESSION_ISOLATED_APPLIED_KEY, SESSION_ISOLATED_KEY,
    TURN_FAILURE_CLASS_KEY, TaskIdParams, TaskQueryParams,
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

/// Why an abandoned turn's row was closed (mika#2379). Stored in `tasks.result`
/// as `{"a2a_close_reason": ...}` so `tasks/get` still reads the row.
const TURN_ABANDONED_REASON: &str = "abandoned: the turn ended before writing a terminal \
                                     state (caller disconnected, or the turn panicked)";

/// Guarantees an A2A turn's task row ends in a terminal state (mika#2379).
///
/// `message/send` runs its turn inside the axum handler future. When the caller
/// hangs up — `mika ask`'s client budget and the server's turn envelope are both
/// 660 s in production, and the server's clock starts later — hyper drops that
/// future mid-turn, neither the `completed` nor the `failed` write runs, and the
/// row stays `in_progress` for ever with `updated_at == created_at`. A panic in
/// the `message/stream` spawned task has the same shape.
///
/// Armed once the row exists, disarmed after the normal terminal write. Dropped
/// while still armed, it closes the row as `cancelled` — conditionally, so a
/// terminal state already written is never overwritten — and says so with one
/// `a2a_turn_abandoned` WARN. `Drop` cannot await, so the write is spawned on the
/// current runtime; with no runtime (process shutting down) the row is left to
/// the startup recovery, which fails every A2A row a dead process left open.
struct TurnGuard {
    db: crate::async_db::AsyncDatabase,
    task_id: String,
    port: &'static str,
    started: std::time::Instant,
    armed: bool,
}

impl TurnGuard {
    fn arm(db: &crate::async_db::AsyncDatabase, task_id: &str, port: &'static str) -> Self {
        Self {
            db: db.clone(),
            task_id: task_id.to_string(),
            port,
            started: std::time::Instant::now(),
            armed: true,
        }
    }

    /// The turn wrote its own terminal state; nothing is left to close.
    fn disarm(&mut self) {
        self.armed = false;
    }

    /// Disarm only if the turn's own terminal write landed. A failed write
    /// (e.g. `SQLITE_BUSY` on the shared database) leaves the guard armed, so
    /// its conditional close becomes a second attempt instead of the row
    /// staying `in_progress` until the next daemon restart.
    fn settle(&mut self, terminal_write: anyhow::Result<()>) {
        match terminal_write {
            Ok(()) => self.disarm(),
            Err(e) => tracing::warn!(
                agent = %self.db.agent_id(),
                task_id = %self.task_id,
                port = self.port,
                error = %e,
                "a2a terminal write failed; the turn guard stays armed to close the row"
            ),
        }
    }
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        let (db, task_id, port) = (self.db.clone(), self.task_id.clone(), self.port);
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                agent = %db.agent_id(),
                task_id = %task_id,
                port,
                "a2a turn abandoned with no runtime to close its row; left to startup recovery"
            );
            return;
        };
        handle.spawn(async move {
            match db
                .a2a_abandon_task_if_live(&task_id, TURN_ABANDONED_REASON)
                .await
            {
                Ok(true) => tracing::warn!(
                    event = "a2a_turn_abandoned",
                    agent = %db.agent_id(),
                    task_id = %task_id,
                    port,
                    elapsed_ms,
                    "a2a turn ended without a terminal state; task marked cancelled"
                ),
                Ok(false) => {}
                Err(e) => tracing::warn!(
                    agent = %db.agent_id(),
                    task_id = %task_id,
                    port,
                    error = %e,
                    "failed to close an abandoned a2a turn; left to startup recovery"
                ),
            }
        });
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
///
/// `model_override` is the provider the caller asked for (mika#2304), already
/// resolved and constructed by [`resolve_caller_model_override`] — so by the time
/// it gets here the API-key refusal has already happened and the turn is allowed
/// to run. `None` is the nominal path and costs nothing: `agent_state.llm` is
/// passed through unchanged, with no provider built.
// Eighth parameter added by mika#2304. The per-turn provider must be borrowed
// from an `Arc` the *caller* owns (`AgentParams.llm` is a `&dyn`), so it cannot
// be folded into a struct built here.
#[allow(clippy::too_many_arguments)]
async fn run_a2a_agent(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    session_id: &str,
    input_text: &str,
    task_id: &str,
    stream_ctx: Option<Arc<mika_a2a::streaming::ToolCallStreamContext>>,
    only_skills: &[String],
    model_override: Option<&Arc<dyn mika_common::llm::LlmProvider>>,
    session_isolated: bool,
) -> Result<A2aTurn, A2aTurnFailure> {
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
        // mika#2304. `AgentParams.llm` is a `&dyn`, and `make_provider_for`
        // hands back an `Arc`, so the caller of this function owns the `Arc` for
        // the whole turn and only a borrow crosses here.
        llm: model_override
            .map(|arc| arc.as_ref())
            .unwrap_or_else(|| agent_state.llm.as_ref()),
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
        // mika#2304 D7: when the caller named the model, a matched skill's
        // `[llm]` section must not displace it.
        caller_model_override: model_override.is_some(),
        // mika#1951: the caller asked to read its own session only. Strictly
        // subtractive — see the field's doc comment on `AgentParams`.
        session_isolated,
        trace_id: Some(task_id.to_string()),
        correlated_task_id: None,
        internal: false,
        pr_reviews_posted: Some(&state.pr_reviews_posted),
        stream_ctx,
    };

    match agent::run_agent(&params).await {
        Ok(output) => Ok(A2aTurn {
            text: output.text,
            // mika#1883: the turn's total, taken from the loop's own report for
            // the same reason as the two fields around it — it exists only in
            // this process's hand, and the Task rebuilt from the database does
            // not carry it. Never `output.usage`, which is the last call.
            run_usage: output.run_usage,
            // mika#2304 D3: taken from the loop's own report, never recomputed
            // here. This function knows the provider it handed *in*;
            // `agent_loop` may substitute a per-skill one downstream, and an
            // attestation that named the wrong one would carry the authority of
            // a server statement while saying something false — the defect this
            // ticket closes, one field further on.
            effective_model: output.effective_model,
        }),
        // mika#2522: the classification happens HERE and nowhere downstream.
        // This `Err(e.to_string())` was the end of the line for the error's
        // *variant* — after it, `handle_message_send` held a rendered sentence
        // and could only have classified by `contains()`, which mika#2179 and
        // mika#2289 both refuse in as many words. So the class is taken while
        // `e` is still an `anyhow::Error` and travels with the failure.
        Err(e) => Err(A2aTurnFailure::from_anyhow(&e)),
    }
}

/// Why an A2A turn failed: the sentence, and the class behind it (mika#2522).
///
/// Replaces the bare `String` this function used to return. `Display` renders
/// the message alone, so every existing `%e` site — the `message/stream` error
/// arm included — prints byte for byte what it printed before; only the type
/// gained a second field. Same shape, and the same reason, as `mika-cli`'s
/// `TransportClass`: the class travels as a typed marker rather than as
/// something a reader has to recover from prose.
struct A2aTurnFailure {
    message: String,
    /// One of `mika_common::llm::error_class`'s seven spellings.
    class: std::borrow::Cow<'static, str>,
}

impl A2aTurnFailure {
    /// Read the class off the `anyhow` cause chain, then render the message.
    ///
    /// The order matters only in that both must happen here: once the error is
    /// a `String` the variant is gone. `classify_anyhow_error` (mika#2289)
    /// `downcast_ref`s the whole chain and answers `other` for anything that is
    /// not an `LlmError` — a statement, not a fallback.
    fn from_anyhow(err: &anyhow::Error) -> Self {
        Self {
            message: err.to_string(),
            class: mika_common::llm::error::classify_anyhow_error(err),
        }
    }
}

impl std::fmt::Display for A2aTurnFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What a completed A2A turn reports back to its port.
///
/// A struct rather than a tuple because the second field is easy to drop
/// silently: mika#2270 was caused by exactly that, `Ok(_)` throwing away the
/// turn's own text one line before the Task was rebuilt without it.
struct A2aTurn {
    /// The turn's assistant text, as the loop produced it.
    text: Option<String>,
    /// The model that actually served, `provider/model` (mika#2304).
    effective_model: Option<String>,
    /// Sum of every LLM call of the turn (mika#1883).
    ///
    /// The third field, and the doc comment above says why the struct is not a
    /// tuple: a third member is exactly as easy to drop in silence as the
    /// second was.
    run_usage: Option<mika_common::llm::LlmUsage>,
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

/// Extract the caller's session-isolation request (mika#1951).
///
/// # Absence is soft, a malformed value is not
///
/// No metadata, key absent, or `null` → `Ok(false)`: no restriction, and the turn
/// is byte-identical to the one before this key existed. That is the property the
/// three sister keys already hold, and it is what lets an older and a newer
/// caller produce the same turn.
///
/// Any **present, non-boolean** value → `Err(INVALID_PARAMS)`. It is never read
/// as `false`. The asymmetry with [`requested_only_skills`] is
/// [`resolve_caller_model_override`]'s, for the same measured reason: a skill
/// restriction silently dropped makes a turn *wider*, which is visible and
/// falsifies no measurement; an isolation silently dropped makes the measurement
/// **wrong while producing a plausible answer**. mika#1951's founding battery
/// produced ten contaminated latency samples that looked valid — a silent no-op
/// here *is* that defect.
///
/// `"true"` as a string is the shape a hand-written client is most likely to
/// send, and it is exactly the one that must not pass: a bench that asked for
/// isolation and did not get it is the incident, not a nuance.
fn requested_session_isolation(params: &MessageSendParams) -> Result<bool, JsonRpcError> {
    let Some(value) = params
        .metadata
        .as_ref()
        .and_then(|m| m.get(SESSION_ISOLATED_KEY))
    else {
        return Ok(false);
    };
    match value {
        serde_json::Value::Null => Ok(false),
        serde_json::Value::Bool(b) => Ok(*b),
        other => Err(JsonRpcError::with_message(
            INVALID_PARAMS,
            format!(
                "`{SESSION_ISOLATED_KEY}` must be a boolean; got {}. \
                 A malformed isolation request is refused rather than read as \
                 `false`: a turn that believes it is isolated and is not \
                 produces plausible, contaminated measurements (mika#1951).",
                shape_of(other)
            ),
        )),
    }
}

/// Name a JSON value's shape for an operator-facing refusal, without echoing it.
///
/// The value came from an authenticated caller, but it lands in a JSON-RPC error
/// that may be logged; naming the type is what the operator needs to fix the
/// call, and it cannot carry a payload.
fn shape_of(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// Stamp what the turn **really did** about session isolation onto the Task the
/// caller receives (mika#1951 U3).
///
/// Written on **every** synchronous turn, isolated or not, for the reason
/// [`stamp_effective_model`] spells out for its own field: without that, absence
/// would be ambiguous between "this server predates mika#1951" and "no isolation
/// was asked for", and the client could not safely treat absence as *I do not
/// know* — which is the whole basis on which it refuses to print its own flag.
fn stamp_session_isolation(task: &mut Task, isolated: bool) {
    task.metadata
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(
            SESSION_ISOLATED_APPLIED_KEY.to_string(),
            serde_json::Value::Bool(isolated),
        );
}

/// Extract the model the caller wants this turn to run under (mika#2304).
///
/// **Reading is fail-soft, exactly like [`requested_only_skills`]:** key absent,
/// no metadata, `null`, a number, an array, an object, an empty or blank string
/// — all yield `None`, and the turn runs under the agent's configured model as
/// it always did. A caller from an older or a newer version of the protocol must
/// not be able to fail a turn with a field this server is free to ignore.
///
/// **Applying it is not** — see [`resolve_caller_model_override`]. That
/// asymmetry, inside one feature, is the whole design decision of mika#2304: the
/// server may decline to *notice* an override, but once it has noticed one it may
/// never quietly run under a different model.
///
/// The string is returned raw. Alias resolution and prefix stripping belong to
/// the executing side (mika#1591 semantics depend on *this* agent's
/// `llm_provider`), which on the `--remote` path the caller does not know.
fn requested_model_override(params: &MessageSendParams) -> Option<&str> {
    params
        .metadata
        .as_ref()?
        .get(MODEL_OVERRIDE_KEY)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Turn a caller-declared model id into the provider that will serve the turn,
/// or refuse the request by name (mika#2304, D2).
///
/// # Fail-closed, and why it is placed here
///
/// A declared override this agent cannot honour **fails the request**; it is
/// never degraded to "no override". The refusal is raised *before* the task row
/// is created and before the agent lock is taken, so the caller gets a JSON-RPC
/// error carrying the sentence — `Provider 'x' has no API key configured. Cannot
/// route model 'y'.` — instead of a task in state `failed` whose reason lives
/// only in the server log. `mika ask` renders that as `remote error: …`, which is
/// the "message clair" the founding ticket asked for as its fallback option,
/// obtained without giving up the capability.
///
/// # The order of the two steps is the point
///
/// [`mika_common::llm::model_override::resolve_model_override`] checks the API
/// key **before** `make_provider_for` builds anything.
/// `create_provider_with_budget` routes the ten OpenAI-compatible variants —
/// OpenRouter among them, the rail the founding measurement ran on — to
/// `OpenAiCompatibleProvider::new`, which returns no `Result` and never consults
/// `api_key`. A missing key therefore *succeeds* at construction. Inverting the
/// two would leave a built provider, an absent key, and a turn that departs
/// anyway: the fail-closed policy written down and absent from the binary.
///
/// What no layer can check before the call is whether the provider actually
/// serves that model id. An unknown id fails at the first request on the
/// provider's own 400/404 — already fail-closed, and nothing to write for it.
fn resolve_caller_model_override(
    agent_state: &Arc<AgentState>,
    params: &MessageSendParams,
    task_id: &str,
) -> Result<Option<Arc<dyn mika_common::llm::LlmProvider>>, JsonRpcError> {
    let Some(requested) = requested_model_override(params) else {
        return Ok(None);
    };

    match caller_model_provider(&agent_state.settings, requested) {
        Ok(provider) => {
            info!(
                event = "a2a_model_override_applied",
                agent = %agent_state.db.agent_id(),
                task_id = %task_id,
                requested = %requested,
                resolved = %format!("{}/{}", provider.provider_name(), provider.model_name()),
                "running this turn under the caller's model (mika#2304)"
            );
            Ok(Some(provider))
        }
        Err(reason) => {
            warn!(
                event = "a2a_model_override_refused",
                agent = %agent_state.db.agent_id(),
                task_id = %task_id,
                requested = %requested,
                reason = %reason,
                "caller declared a model this agent cannot serve — refusing the turn (mika#2304)"
            );
            Err(JsonRpcError::with_message(INVALID_PARAMS, reason))
        }
    }
}

/// The decision half of [`resolve_caller_model_override`], with no `AgentState`
/// and no logging — so it can be asserted without standing a server up.
///
/// `Err` carries the operator-facing sentence. There is no third outcome: the
/// signature itself is the fail-closed policy. A `Result<Option<_>>` here, or an
/// `.ok()` on either step, would be the silent degradation mika#2304 exists to
/// remove.
fn caller_model_provider(
    settings: &mika_common::config::Settings,
    requested: &str,
) -> Result<Arc<dyn mika_common::llm::LlmProvider>, String> {
    // Key check FIRST — `make_provider_for` cannot refuse a missing key on the
    // OpenAI-compatible rail (see this function's neighbours' doc comments), so
    // swapping these two lines would leave a built provider, an absent key, and a
    // turn that departs anyway.
    let (provider_kind, model_id) =
        mika_common::llm::model_override::resolve_model_override(settings, requested)
            .map_err(|e| e.to_string())?;

    settings
        .make_provider_for(provider_kind, Some(&model_id))
        .map_err(|e| format!("Cannot build provider '{provider_kind}' for model '{model_id}': {e}"))
}

/// Stamp the model that served a turn onto the Task the caller receives
/// (mika#2304, D3).
///
/// Written on **every** turn, override or not. Without that, an absent
/// attestation would be ambiguous between "this server predates mika#2304" and
/// "no override was asked for", and the client could not safely treat absence as
/// "I do not know" — which is the whole basis on which it refuses to print a
/// local value.
///
/// `None` means the turn produced no `AgentOutput` to read it from (the loop
/// failed before the effective provider was resolved). That population is
/// honestly *not attested*; nothing is written, and the client shows no model.
fn stamp_effective_model(task: &mut Task, effective_model: Option<&str>) {
    let Some(model) = effective_model else {
        return;
    };
    task.metadata
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(
            EFFECTIVE_MODEL_KEY.to_string(),
            serde_json::Value::String(model.to_string()),
        );
}

/// Stamp the turn's **total** token usage onto the Task the caller receives
/// (mika#1883).
///
/// Third sibling of [`stamp_effective_model`] and [`stamp_session_isolation`],
/// at the same intervention point and for the same reason: `task` is already
/// `mut` there, `a2a_build_task` is upstream, and nothing downstream can
/// overwrite the field.
///
/// **Unlike its two siblings it is NOT written unconditionally, and that
/// asymmetry is deliberate.** They are the answer to a flag, so an absence would
/// be ambiguous between "this server is old" and "nothing was asked for", and
/// the client's whole basis for refusing to print a local value would collapse.
/// This one is a *measurement*: there is no flag to answer, and its only
/// legitimate absence is "the turn produced no call whose usage could be read".
/// Writing a zero there would be indistinguishable from a real turn — the same
/// reasoning `llm_calls.request_bytes` carries (mika#2331: *"`null` is never
/// `0`"*), transposed. The residual ambiguity — a pre-mika#1883 server and an
/// unmeasured turn read alike — is accepted because both call for the same
/// client conduct: show nothing.
fn stamp_run_usage(task: &mut Task, run_usage: Option<&mika_common::llm::LlmUsage>) {
    let Some(usage) = run_usage else {
        return;
    };
    let wire = RunUsage {
        input: usage.input_tokens,
        output: usage.output_tokens,
        // Absent stays absent across the boundary: `Some(0)` here would claim a
        // provider reported a zero it never reported.
        cache_read: usage.cache_read_input_tokens,
        cache_write: usage.cache_creation_input_tokens,
    };
    let Ok(value) = serde_json::to_value(wire) else {
        // A four-integer struct cannot fail to serialize; if it ever did, the
        // honest outcome is *not attested* rather than a half-written object.
        return;
    };
    task.metadata
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(RUN_USAGE_KEY.to_string(), value);
}

/// Stamp the **class** of the failure that killed a turn onto the Task the
/// caller receives (mika#2522).
///
/// Fourth sibling of [`stamp_effective_model`], [`stamp_session_isolation`] and
/// [`stamp_run_usage`], at the same intervention point and for the same reason:
/// `task` is already `mut` here, `a2a_build_task` is upstream, and nothing
/// downstream can overwrite the field.
///
/// **Written unconditionally on the failure branch, and on no other.** Its
/// absence therefore means "this server attested nothing" — a binary predating
/// mika#2522, `message/stream`, `returnImmediately` — and never "the class is
/// `contract`". Like [`stamp_run_usage`] it is a *measurement* rather than the
/// answer to a flag, so it carries no value on the success path: a class on a
/// turn that did not fail would be a statement about a failure that did not
/// happen.
fn stamp_turn_failure_class(task: &mut Task, class: &str) {
    task.metadata
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(
            TURN_FAILURE_CLASS_KEY.to_string(),
            serde_json::Value::String(class.to_string()),
        );
}

/// The model label a failed turn is counted under (mika#2522 R-3).
///
/// `ResolvedBudgetRecord::model` is empty when the provider itself could not be
/// parsed (`unknown_provider`, mika#2328). That **degrades** the line to
/// `unknown` rather than suppressing it — the `repo=unknown` motif of mika#2496:
/// a turn lost on a model we cannot name is still a turn lost, and dropping the
/// row would silently shrink the very count AC2 exists to produce.
///
/// It is also never an empty `target_key`: an empty group in the operator's
/// `GROUP BY` says nothing about why it is empty, while `unknown` names the one
/// cause it can have.
fn audit_model_label(record_model: &str) -> &str {
    let trimmed = record_model.trim();
    if trimmed.is_empty() {
        "unknown"
    } else {
        trimmed
    }
}

/// Count a failed turn, on the two surfaces one fact owes (mika#2522 R-3).
///
/// One fact, one site, two surfaces: the class the client needs in order to
/// decide (stamped by [`stamp_turn_failure_class`]) and the event plus audit row
/// an operator needs in order to answer *which model loses turns, and on which
/// class*. Writing them from one place is what guarantees they cannot diverge.
///
/// The class is taken as an argument rather than recomputed: it was read off the
/// error's **variant** in [`A2aTurnFailure::from_anyhow`], which is the last
/// point where the variant exists. Reclassifying here would mean classifying a
/// rendered sentence.
async fn report_turn_failure(
    agent_state: &Arc<AgentState>,
    task_id: &str,
    session_id: &str,
    failure: &A2aTurnFailure,
) {
    let class = &failure.class;

    // The model is *resolved*, never guessed: `effective_model` is `None` on
    // this branch (the turn produced no `AgentOutput`), so it comes off the
    // record frozen at `init_agent` — the same value `llm_budget_resolved`
    // reports (mika#2328).
    let model = audit_model_label(&agent_state.budget_record.model);

    warn!(
        event = "a2a_turn_failed",
        agent_id = %agent_state.db.agent_id(),
        task_id = %task_id,
        session_id = %session_id,
        model = %model,
        error_class = %class,
        "a2a turn failed — attesting the class to the caller (mika#2522)"
    );

    if let Err(e) = agent_state
        .db
        .log_audit_event(
            session_id,
            A2A_TURN_FAILED_TOOL,
            model,
            None,
            Some(class.as_ref()),
            Some(&format!(
                "task={task_id} class={class} error={}",
                mika_common::text::safe_truncate(&failure.message, 500)
            )),
            Some(task_id),
        )
        .await
    {
        warn!(error = %e, task_id = %task_id, "a2a_turn_failed_audit_failed");
    }
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

/// The `audit_events.tool_name` under which a failed A2A turn is counted
/// (mika#2522 R-3).
///
/// **SOLE WRITER** — [`report_turn_failure`] is the only site that writes this
/// name, in the log and in `audit_events`, and a source scan
/// (`canonical_tokens::tests::mika2522_the_turn_failure_name_has_a_single_writer`)
/// refuses a second. That is what makes
/// `SELECT target_key, after_value, count(*) … GROUP BY 1, 2` an exact answer to
/// *which model loses turns, and on which class* rather than a number two
/// writers can disagree about.
const A2A_TURN_FAILED_TOOL: &str = "a2a_turn_failed";

/// What the agent loop left in `handle_message_send`'s hand.
///
/// Named rather than an `Option<Option<String>>` because the three states are
/// semantically distinct and the nesting reads as a mistake: the loop failing, the
/// loop producing nothing, and the loop producing text are three different things
/// to do with a rebuilt Task.
enum TurnText {
    /// The loop finished. `Some` when it produced text of its own.
    Produced(Option<String>),
    /// The loop failed; the task is `failed` and there is no answer to serve —
    /// but there *is* something to say about why (mika#2522).
    ///
    /// The class travels **inside** the variant rather than beside it in a
    /// parallel local: the `match` on `turn_text` below is the only place that
    /// knows a turn failed, and carrying the class here is what stops a later
    /// editor from handling that branch without it.
    LoopFailed {
        class: std::borrow::Cow<'static, str>,
    },
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

    // mika#2304 D2: resolve the caller's model **before** the lock and before the
    // task row exists. A declared override this agent cannot serve is refused as
    // a JSON-RPC error naming the model and the provider — the caller reads a
    // sentence, not a task in state `failed` whose reason is only in the log. It
    // also costs no queue place and no database write.
    //
    // The `Arc` is bound here so it outlives the turn: `AgentParams.llm` is a
    // `&dyn` and `make_provider_for` returns an `Arc`.
    let caller_model = match resolve_caller_model_override(agent_state, &params, &task_id) {
        Ok(p) => p,
        Err(err) => {
            return Json(JsonRpcResponse::error(request.id.clone(), err)).into_response();
        }
    };

    // mika#1951: refused here for the same three reasons as the override above —
    // before the lock, before the task row, and as a sentence rather than a
    // `failed` task. A malformed isolation request must cost the caller an error
    // it can read, not a turn it will misread.
    let session_isolated = match requested_session_isolation(&params) {
        Ok(v) => v,
        Err(err) => {
            warn!(
                event = "a2a_session_isolation_refused",
                agent = %agent_state.db.agent_id(),
                task_id = %task_id,
                port = "send",
                "caller declared `{SESSION_ISOLATED_KEY}` with a non-boolean value — refusing the turn (mika#1951)"
            );
            return Json(JsonRpcResponse::error(request.id.clone(), err)).into_response();
        }
    };

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
        // Process synchronously: transition to working, run agent loop, return completed task.
        //
        // mika#2379: from here the turn runs inside this handler's future, which
        // hyper drops if the caller hangs up. The guard closes the row if that
        // happens before the terminal write below.
        let mut turn_guard = TurnGuard::arm(&agent_state.db, &task_id, "send");
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

        // mika#2304: the model that served, kept for the same reason the text is
        // — it exists only in this process's hand, and the Task rebuilt from the
        // database does not carry it.
        let mut effective_model: Option<String> = None;
        // mika#1883: same trajectory, same reason — the per-turn total lives
        // only in this process's hand.
        let mut run_usage: Option<mika_common::llm::LlmUsage> = None;

        let turn_text = match run_a2a_agent(
            state,
            agent_state,
            &session_id,
            &input_text,
            &task_id,
            None,
            &only_skills,
            caller_model.as_ref(),
            session_isolated,
        )
        .await
        {
            Ok(turn) => {
                turn_guard.settle(
                    agent_state
                        .db
                        .a2a_update_task_state(&task_id, "completed")
                        .await,
                );

                info!(task_id = %task_id, "A2A task completed via agent loop");
                effective_model = turn.effective_model;
                run_usage = turn.run_usage;
                TurnText::Produced(turn.text)
            }
            Err(e) => {
                error!(error = %e, task_id = %task_id, "A2A agent loop failed");
                turn_guard.settle(
                    agent_state
                        .db
                        .a2a_update_task_state(&task_id, "failed")
                        .await,
                );
                // mika#2522: the class was read off the variant in
                // `run_a2a_agent` and travels on the failure. Count it here —
                // `agent_state` and `session_id` are in hand — and carry it to
                // the stamp below.
                report_turn_failure(agent_state, &task_id, &session_id, &e).await;
                TurnText::LoopFailed { class: e.class }
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
                //
                // mika#2522: it does now carry **why**. Serving the state alone
                // made the client class every `failed` as `Contract`, so a turn
                // an OpenRouter cut had just killed was declared un-retryable
                // and the groom above it died with it.
                match &turn_text {
                    TurnText::Produced(text) => {
                        ensure_send_task_carries_text(
                            agent_state,
                            &mut task,
                            text.as_deref(),
                            &session_id,
                        )
                        .await;
                    }
                    TurnText::LoopFailed { class } => {
                        stamp_turn_failure_class(&mut task, class);
                    }
                }
                // mika#2304 D3: the mika#2270 intervention point — `task` is
                // already `mut` here and `a2a_build_task` is upstream, so nothing
                // downstream can overwrite the field.
                stamp_effective_model(&mut task, effective_model.as_deref());
                // mika#1951 U3: same intervention point, same reason. The value
                // is the one this handler *resolved and passed in*, not the flag
                // the caller sent — and the two are the same only because the
                // refusal above made every other reading impossible.
                stamp_session_isolation(&mut task, session_isolated);
                // mika#1883: same intervention point, third field. Silent when
                // the turn produced no readable usage — see `stamp_run_usage`
                // for why this one, unlike its two siblings, is conditional.
                stamp_run_usage(&mut task, run_usage.as_ref());
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

    // mika#2304: same key, same refusal, on both ports — `message/stream` shares
    // `MessageSendParams` with `message/send`, and a field that meant different
    // things on two endpoints of the same protocol would be worse than no field.
    // Resolved before the task row and before the stream opens, so the refusal is
    // still a plain JSON-RPC error rather than a `failed` frame on an open
    // stream.
    //
    // NOTE: this port does **not** stamp the attestation. It serves events, not
    // a rebuilt `Task`, and mika#2304 scopes the attestation to synchronous
    // `message/send` — the path both doors of `mika ask` take. A stream caller
    // lands in the honest "not attested" population, which the client renders as
    // an absent model rather than a local guess.
    let caller_model = match resolve_caller_model_override(agent_state, &params, &task_id) {
        Ok(p) => p,
        Err(err) => {
            return Json(JsonRpcResponse::error(request.id.clone(), err)).into_response();
        }
    };

    // mika#1951: the *request* half is honoured on this port — an isolated
    // stream is a legitimate thing to ask for, and refusing a malformed value
    // must still happen before the stream opens (mid-SSE there is no way left to
    // say "your call was wrong"). The *attestation* half is not, for the reason
    // stated above for the model: this port serves events, not a rebuilt `Task`.
    let session_isolated = match requested_session_isolation(&params) {
        Ok(v) => v,
        Err(err) => {
            warn!(
                event = "a2a_session_isolation_refused",
                agent = %agent_state.db.agent_id(),
                task_id = %task_id,
                port = "stream",
                "caller declared `{SESSION_ISOLATED_KEY}` with a non-boolean value — refusing the turn (mika#1951)"
            );
            return Json(JsonRpcResponse::error(request.id.clone(), err)).into_response();
        }
    };

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
        // mika#2379: a disconnect does not cancel this task, but a panic ends it
        // without a terminal write. Armed first so every path below is covered.
        let mut turn_guard = TurnGuard::arm(&agent_state_clone.db, &task_id_clone, "stream");
        let turn = StreamTurn {
            state: state_clone,
            agent_state: agent_state_clone,
            session_id,
            input_text,
            task_id: task_id_clone,
            context_id,
            tx,
            only_skills,
            caller_model,
            session_isolated,
        };

        // The kill-switch path already holds the lock — it was taken in the
        // handler, at the same point it always was — so there is nothing here to
        // wait for and nothing to abandon.
        let slot = match entry {
            StreamLockEntry::Held(guard) => {
                run_a2a_stream_turn(guard, turn, turn_guard).await;
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
                turn_guard.settle(turn
                    .agent_state
                    .db
                    .a2a_update_task_state(&turn.task_id, "canceled")
                    .await);
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
                turn_guard.settle(
                    turn.agent_state
                        .db
                        .a2a_update_task_state(&turn.task_id, "failed")
                        .await,
                );
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

        run_a2a_stream_turn(guard, turn, turn_guard).await;
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
    /// The caller's model, already resolved and refused-or-accepted in the
    /// handler (mika#2304). Same reason as `only_skills` for carrying it: the
    /// params do not survive the spawn — and the refusal must happen before the
    /// stream opens, so it cannot be deferred here either.
    caller_model: Option<Arc<dyn mika_common::llm::LlmProvider>>,
    /// The caller's session-isolation request (mika#1951), already validated in
    /// the handler. Carried for the same reason as its two neighbours: `params`
    /// does not survive the spawn, and the refusal must land before the stream
    /// opens — a caller cannot be told mid-SSE that its request was malformed.
    session_isolated: bool,
}

/// Run the streaming turn. The guard is taken by value and dropped with this
/// future, so the agent lock is held for exactly the turn's lifetime.
async fn run_a2a_stream_turn(
    _lock_guard: OwnedMutexGuard<()>,
    turn: StreamTurn,
    mut turn_guard: TurnGuard,
) {
    let StreamTurn {
        state,
        agent_state,
        session_id,
        input_text,
        task_id,
        context_id,
        tx,
        only_skills,
        caller_model,
        session_isolated,
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
        caller_model.as_ref(),
        session_isolated,
    )
    .await
    {
        Ok(A2aTurn {
            text: response_text,
            ..
        }) => {
            // mika#2270: the literal is shared with `message/send`, which now
            // serves the same one for a turn that produced no text. Two ports
            // answering the same situation identically is a property worth being
            // able to check.
            let text = response_text.unwrap_or_else(|| COMPLETED_WITHOUT_TEXT.to_string());
            let response_message = agent_text_message(&task_id, context_id.clone(), text);

            turn_guard.settle(
                agent_state
                    .db
                    .a2a_update_task_state(&task_id, "completed")
                    .await,
            );

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
            turn_guard.settle(
                agent_state
                    .db
                    .a2a_update_task_state(&task_id, "failed")
                    .await,
            );

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
        // The boundary comes from `mika_common::source_guard` (mika#2398):
        // splitting on the first `\n#[cfg(test)]\n` was blind to a module-level
        // helper, to a single-line item, and to this file being read as a whole.
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let source = scanner.production_of(&scanner.src_root().join("server/a2a.rs"));

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

    // ================== mika#1951 — the session-isolation lever ==================

    fn with_isolation(value: serde_json::Value) -> MessageSendParams {
        params_with_metadata(Some(HashMap::from([(
            SESSION_ISOLATED_KEY.to_string(),
            value,
        )])))
    }

    /// **Absence is soft.** No metadata, an empty map, the key absent, `null` —
    /// all read as "no restriction", so an older caller and a newer one produce
    /// the same turn. That is the property the three sister keys hold, and
    /// losing it would fail turns from every client that predates this key.
    #[test]
    fn mika1951_an_absent_isolation_request_is_no_restriction() {
        assert!(!requested_session_isolation(&params_with_metadata(None)).unwrap());
        assert!(!requested_session_isolation(&params_with_metadata(Some(HashMap::new()))).unwrap());
        assert!(!requested_session_isolation(&with_isolation(serde_json::Value::Null)).unwrap());
        // A declared `false` is honoured literally — same turn, said explicitly.
        assert!(!requested_session_isolation(&with_isolation(serde_json::json!(false))).unwrap());
        assert!(requested_session_isolation(&with_isolation(serde_json::json!(true))).unwrap());
    }

    /// **Applying it is fail-closed.** A present, non-boolean value refuses the
    /// turn; it is never read as `false`.
    ///
    /// `"true"` and `1` are the shapes a hand-written client is most likely to
    /// send, and they are exactly the ones that must not pass: a bench that asked
    /// for isolation and silently did not get it produces plausible, contaminated
    /// measurements — which is mika#1951 itself, not a nuance of it.
    #[test]
    fn mika1951_a_malformed_isolation_request_refuses_the_turn() {
        for value in [
            serde_json::json!("true"),
            serde_json::json!("yes"),
            serde_json::json!(1),
            serde_json::json!(0),
            serde_json::json!([true]),
            serde_json::json!({ "isolated": true }),
            serde_json::json!(""),
        ] {
            let err = requested_session_isolation(&with_isolation(value.clone()))
                .expect_err("a non-boolean must refuse the turn, never read as false");
            assert_eq!(err.code, INVALID_PARAMS, "wrong code for {value}");
            let message = err.message.clone();
            assert!(
                message.contains(SESSION_ISOLATED_KEY),
                "the refusal must name the key an operator has to fix: {message}"
            );
        }
    }

    /// The refusal names the shape it received and never echoes the value.
    ///
    /// It lands in a JSON-RPC error that may be logged; the type is what the
    /// operator needs to repair the call, and it cannot carry a payload.
    #[test]
    fn mika1951_the_refusal_names_the_shape_without_echoing_the_value() {
        let err = requested_session_isolation(&with_isolation(serde_json::json!("hunter2")))
            .expect_err("a string must refuse");
        assert!(err.message.contains("a string"), "{}", err.message);
        assert!(
            !err.message.contains("hunter2"),
            "the refusal must not echo the caller's value: {}",
            err.message
        );
    }

    /// **U3 — the attestation is written on every synchronous turn**, isolated
    /// or not.
    ///
    /// Absence must mean "this server did not attest", which it cannot if the
    /// server only writes the field when the answer is `true`: the client would
    /// then be unable to tell a pre-mika#1951 binary from a turn that ran
    /// unisolated, and printing its own flag in that gap is the false green
    /// mika#2304 measured on the model field.
    #[test]
    fn mika1951_the_attestation_is_written_for_both_verdicts() {
        for ran_isolated in [true, false] {
            let mut task = completed_task(Some(agent_reply("ok")));
            stamp_session_isolation(&mut task, ran_isolated);
            assert_eq!(
                mika_a2a::params::attested_session_isolation(&task),
                Some(ran_isolated),
                "the server must attest what it did, including when it did nothing"
            );
        }
    }

    /// The attestation is additive: stamping it does not disturb a model
    /// attestation already on the Task.
    ///
    /// The two are written back to back at the same intervention point, on a
    /// `metadata` map that `a2a_build_task` may already have populated. A
    /// `metadata = Some(map)` assignment instead of an insert would silently drop
    /// the other field, and each field's own test would still pass.
    #[test]
    fn mika1951_the_two_attestations_coexist() {
        let mut task = completed_task(Some(agent_reply("ok")));
        stamp_effective_model(&mut task, Some("openrouter/z-ai/glm-5.3"));
        stamp_session_isolation(&mut task, true);
        assert_eq!(
            mika_a2a::params::attested_model(&task),
            Some("openrouter/z-ai/glm-5.3")
        );
        assert_eq!(
            mika_a2a::params::attested_session_isolation(&task),
            Some(true)
        );
    }

    /// **The bool reaches the loop, and it is read at exactly two sites.**
    ///
    /// The plan's Definition of Done says `AgentParams.session_isolated` is read
    /// at the scoped-session resolution and the summary gate *and nowhere else*.
    /// A third reader would not make any decision wrong on its own — it would
    /// widen what "isolated" silently means, which no behavioural test can see.
    /// Hence a lexical guard, on production source only.
    #[test]
    fn mika1951_the_isolation_flag_has_exactly_two_readers_in_the_loop() {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let source = scanner.production_of(&scanner.src_root().join("agent_loop/mod.rs"));
        let reads = source.matches("params.session_isolated").count();
        assert_eq!(
            reads, 2,
            "expected exactly two read sites (the scope resolution and the summary \
             gate), found {reads} — a third would widen what isolation means \
             without failing any assertion about either channel"
        );
    }

    // ===================== mika#2304 — the model override =====================

    fn openrouter_settings(api_key: Option<&str>) -> mika_common::config::Settings {
        let mut settings = mika_common::config::Settings::test_defaults();
        settings.llm_provider = mika_common::llm::ProviderKind::OpenRouter;
        settings.openrouter_api_key = api_key.map(secrecy::SecretString::from);
        settings
    }

    /// **T3 / AC4 — reading the key is fail-soft.**
    ///
    /// Every malformed shape means "no override", never an error. A caller from
    /// an older or a newer protocol version must not be able to fail a turn with
    /// a field this server is free to ignore — the same contract
    /// `requested_only_skills` carries one ticket earlier.
    #[test]
    fn mika2304_every_unreadable_override_shape_means_no_override() {
        assert_eq!(
            requested_model_override(&params_with_metadata(None)),
            None,
            "no metadata at all"
        );
        for shape in [
            serde_json::json!({}),
            serde_json::json!({ MODEL_OVERRIDE_KEY: serde_json::Value::Null }),
            serde_json::json!({ MODEL_OVERRIDE_KEY: 42 }),
            serde_json::json!({ MODEL_OVERRIDE_KEY: ["sonnet"] }),
            serde_json::json!({ MODEL_OVERRIDE_KEY: { "model": "sonnet" } }),
            serde_json::json!({ MODEL_OVERRIDE_KEY: "" }),
            serde_json::json!({ MODEL_OVERRIDE_KEY: "   " }),
            // Another key's payload must not be mistaken for this one.
            serde_json::json!({ ONLY_SKILLS_KEY: ["mika-arch-groom-ticket"] }),
        ] {
            let serde_json::Value::Object(map) = shape.clone() else {
                unreachable!()
            };
            let params = params_with_metadata(Some(map.into_iter().collect()));
            assert_eq!(
                requested_model_override(&params),
                None,
                "shape {shape} must degrade to no override"
            );
        }
    }

    /// The readable case, and it comes back **raw**: the server owns alias
    /// resolution and prefix stripping (mika#1591 depends on *this* agent's
    /// provider), so the reader must not pre-chew the string.
    #[test]
    fn mika2304_a_declared_override_is_read_verbatim() {
        let params = params_with_metadata(Some(HashMap::from([(
            MODEL_OVERRIDE_KEY.to_string(),
            serde_json::Value::String("  moonshotai/kimi-k2.5  ".to_string()),
        )])));
        assert_eq!(
            requested_model_override(&params),
            Some("moonshotai/kimi-k2.5"),
            "only surrounding whitespace is removed"
        );
    }

    /// **T4 / AC3 — applying it is fail-CLOSED.**
    ///
    /// The core of mika#2304. A declared override the agent cannot serve refuses;
    /// it is never degraded to "no override". The chosen cause is a provider with
    /// no API key, which is the only inapplicability detectable without a network
    /// call — an id the provider does not know is not checkable at any layer and
    /// fails, already fail-closed, at the provider's own 400/404.
    #[test]
    fn mika2304_an_override_the_agent_cannot_serve_refuses_the_turn() {
        // `unwrap_err` is unavailable here: `Arc<dyn LlmProvider>` is not `Debug`.
        let Err(err) = caller_model_provider(&openrouter_settings(None), "moonshotai/kimi-k2.5")
        else {
            panic!("an override with no API key must not be honoured");
        };
        assert!(err.contains("openrouter"), "names the provider: {err}");
        assert!(
            err.contains("moonshotai/kimi-k2.5"),
            "names the model: {err}"
        );
    }

    /// **T4's negative control**, and the assertion that would go red the day
    /// someone aligned this key on `only_skills`' fail-soft policy.
    ///
    /// The refusal must be *caused by the key*: with one configured, the same
    /// request succeeds and lands on the caller's model. Without this half, a
    /// function that refused everything would pass the test above.
    #[test]
    fn mika2304_the_refusal_is_caused_by_the_key_not_by_the_override() {
        let provider = caller_model_provider(
            &openrouter_settings(Some("sk-or-xxx")),
            "moonshotai/kimi-k2.5",
        )
        .expect("with a key configured the override must be honoured");
        assert_eq!(provider.provider_name(), "openrouter");
        assert_eq!(
            provider.model_name(),
            "moonshotai/kimi-k2.5",
            "an honoured override must reach the provider — falling back to the \
             configured model here is exactly the silent no-op mika#2304 closes"
        );
    }

    /// **T4's structural half — FD1b.** `make_provider_for` routes the ten
    /// OpenAI-compatible variants (OpenRouter included, the rail the founding
    /// measurement ran on) to `OpenAiCompatibleProvider::new`, which returns no
    /// `Result` and never consults `api_key`. So the key check must be *posed*
    /// before construction, never hoped for from it. Inverting the two lines
    /// leaves a built provider, an absent key, and a turn that departs anyway —
    /// a defect no behavioural test on this rail can see, because the refusal
    /// simply stops happening.
    #[test]
    fn mika2304_the_key_is_checked_before_the_provider_is_built() {
        let source = include_str!("a2a.rs");
        let body = source
            .split_once("fn caller_model_provider(")
            .expect("caller_model_provider must exist")
            .1;
        let check = body
            .find("resolve_model_override(")
            .expect("the key check must be in this function");
        let build = body
            .find("make_provider_for(")
            .expect("the construction must be in this function");
        assert!(
            check < build,
            "mika#2304 FD1b: `resolve_model_override` (which checks the API key) \
             must precede `make_provider_for`. `OpenAiCompatibleProvider::new` \
             succeeds with no key, so a construction-first order writes the \
             fail-closed policy in the plan and leaves it out of the binary."
        );
        // And nothing may turn the refusal back into an absence.
        for degrader in [".ok()", "unwrap_or", "unwrap_or_default", "unwrap_or_else"] {
            assert!(
                !body[..build + 200].contains(degrader),
                "`{degrader}` in `caller_model_provider` would degrade a refusal \
                 into 'no override' — the silent no-op that IS the defect (D2)"
            );
        }
    }

    /// **T10 — the attestation follows the provider that SERVED.**
    ///
    /// Structural, and the reason is worth stating rather than hiding: the
    /// discriminating behavioural case needs a turn whose per-skill `[llm]`
    /// override resolves to a *different* provider, and `resolve_skill_llm_override`
    /// builds that one through `Settings::make_provider_for` — a real HTTP
    /// provider that `MockLlmProvider` cannot stand in for. Such a turn fails at
    /// the first call and produces no `AgentOutput` to inspect, so the end-to-end
    /// assertion is unreachable with the eval harness as it stands.
    ///
    /// What *is* checkable is the wiring, and it is where the defect would live:
    /// the attestation must be taken on `effective_llm` (post-override) and never
    /// on `llm` (the provider handed in). A draft of this ticket placed it in this
    /// very file, on `agent_state.llm` — it would have shipped a field asserting a
    /// model that did not run, wearing the authority of a server attestation.
    #[test]
    fn mika2304_the_attestation_is_taken_on_the_serving_provider() {
        // Needles assembled with `concat!` so this file does not contain the
        // strings it asserts on — otherwise the scan would match itself.
        const ON_SERVING_PROVIDER: &str = concat!("attest_", "effective_model", "(effective_llm)");
        const FORWARDED: &str = concat!("effective_model", ": output.effective_model");
        const ANY_ATTESTATION_SITE: &str = concat!("attest_", "effective_model");

        let loop_source = include_str!("../agent_loop/mod.rs");
        assert!(
            loop_source.contains(ON_SERVING_PROVIDER),
            "mika#2304 E7/FD5: the attestation must be computed from `effective_llm`, \
             the provider that serves the turn after any per-skill `[llm]` override — \
             not from `llm`, which is only the one handed in."
        );
        // The server side must read it back, never recompute one of its own.
        // Scoped to the production half of the file: the T11 round-trip test
        // below legitimately calls the formatter to walk the chain end to end,
        // and it is the *shipped* code that must not have a second site.
        let source = include_str!("a2a.rs");
        let production = source
            .split_once("#[cfg(test)]")
            .expect("this module has a test section")
            .0;
        assert!(
            production.contains(FORWARDED),
            "this module must forward the loop's attestation verbatim; recomputing \
             it here would reintroduce the entry-site reading (E7)"
        );
        assert!(
            !production.contains(ANY_ATTESTATION_SITE),
            "no second attestation site: this module does not know the effective provider"
        );
    }

    /// The attestation is stamped where mika#2270 already intervenes, and it
    /// never overwrites a metadata map the Task already carried.
    #[test]
    fn mika2304_stamping_preserves_other_metadata_and_skips_when_absent() {
        let mut task = completed_task(Some(agent_reply("ok")));
        task.metadata = Some(HashMap::from([(
            "kept".to_string(),
            serde_json::Value::String("value".to_string()),
        )]));

        stamp_effective_model(&mut task, Some("openrouter/moonshotai/kimi-k2.5"));
        let metadata = task.metadata.clone().expect("metadata should be present");
        assert_eq!(metadata.len(), 2);
        assert_eq!(
            metadata.get(EFFECTIVE_MODEL_KEY).and_then(|v| v.as_str()),
            Some("openrouter/moonshotai/kimi-k2.5")
        );
        assert_eq!(metadata.get("kept").and_then(|v| v.as_str()), Some("value"));

        // No attestation → nothing written, not an empty string and not a null:
        // the client must be able to read absence as "this server did not say".
        let mut untouched = completed_task(Some(agent_reply("ok")));
        stamp_effective_model(&mut untouched, None);
        assert!(untouched.metadata.is_none());
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2522 — a turn killed by the transport is not a refusal by the server.
    // ─────────────────────────────────────────────────────────────────────

    /// **V1.** The class comes from the error's **variant**, on every class.
    ///
    /// `run_a2a_agent` used to end on `Err(e.to_string())`, which is where the
    /// variant died: after it, the only way to classify was a `contains()` on a
    /// rendered sentence — the thing mika#2179 and mika#2289 both refuse in as
    /// many words. So the reading happens while `e` is still an `anyhow::Error`,
    /// and this test is what pins that it reads the variant rather than the text.
    ///
    /// The first case is the founding incident's own error, verbatim from
    /// `mika-common`'s `INCIDENT_TRANSPORT_TIMEOUT`: the `body read failed
    /// mid-stream` shape that killed five grooms of mika#2515 on 2026-09-24.
    #[test]
    fn mika2522_the_failure_class_is_read_from_the_error_variant() {
        use mika_common::llm::LlmError;

        let cases: Vec<(anyhow::Error, &str)> = vec![
            (
                anyhow::Error::new(LlmError::Transport(
                    "failed to read response body: error decoding response body: \
                     request or response body error: operation timed out"
                        .into(),
                )),
                "transport_timeout",
            ),
            (
                anyhow::Error::new(LlmError::Transport(
                    "failed to read response body: unexpected EOF during chunked body read".into(),
                )),
                "transport",
            ),
            // AC3's population — none of these may read as transport, because
            // none of them is repaired by sending the same brief again.
            (
                anyhow::Error::new(LlmError::ParseError("bad json".into())),
                "parse",
            ),
            (
                anyhow::Error::new(LlmError::ProviderError("upstream said no".into())),
                "provider",
            ),
            (
                anyhow::Error::new(LlmError::HttpError {
                    status: 400,
                    message: "bad request".into(),
                    retryable: false,
                }),
                "http_400",
            ),
            // Not an LLM failure at all: `other` is a statement, not a fallback.
            (anyhow::anyhow!("the database went away"), "other"),
        ];

        for (err, expected) in cases {
            let rendered = err.to_string();
            let failure = A2aTurnFailure::from_anyhow(&err);
            assert_eq!(
                failure.class, expected,
                "class for {rendered:?} should be read off the variant"
            );
            // The sentence is unchanged: every existing `%e` site — the
            // `message/stream` error arm included — must print what it printed
            // before this ticket.
            assert_eq!(failure.to_string(), rendered);
        }
    }

    /// **V1.** A wrapped cause is still found: the whole `anyhow` chain is
    /// walked.
    ///
    /// The engine adds context as the error climbs, so the `LlmError` is rarely
    /// the outermost layer by the time it reaches this module. A classifier that
    /// only inspected the top would answer `other` on the exact population this
    /// ticket exists for, and the assertion above would still pass.
    #[test]
    fn mika2522_a_wrapped_transport_failure_is_still_transport() {
        use anyhow::Context;
        use mika_common::llm::LlmError;

        let err = Err::<(), _>(LlmError::Transport("connection reset".into()))
            .context("LLM call failed at step 3")
            .context("agent loop failed")
            .expect_err("built as an error");

        assert_eq!(A2aTurnFailure::from_anyhow(&err).class, "transport");
    }

    /// **V1.** The class crosses the frontier and reads back through the one
    /// decoder.
    ///
    /// The full server half of mika#2522: variant → class → Task metadata →
    /// `attested_turn_failure_class`. Asserting the stamp against the raw map
    /// would leave the reader untested, and the reader is what the client calls.
    #[test]
    fn mika2522_the_attested_class_reads_back_through_the_shared_reader() {
        use mika_common::llm::LlmError;

        let err = anyhow::Error::new(LlmError::Transport("connection reset".into()));
        let failure = A2aTurnFailure::from_anyhow(&err);

        let mut task = completed_task(None);
        task.metadata = Some(HashMap::from([(
            "kept".to_string(),
            serde_json::Value::String("value".to_string()),
        )]));
        stamp_turn_failure_class(&mut task, &failure.class);

        assert_eq!(
            mika_a2a::params::attested_turn_failure_class(&task),
            Some("transport")
        );
        // The stamp joins the map, it does not replace it — same contract as its
        // three siblings.
        let metadata = task.metadata.clone().expect("metadata present");
        assert_eq!(metadata.len(), 2);
        assert_eq!(metadata.get("kept").and_then(|v| v.as_str()), Some("value"));
    }

    /// **V1, negative control.** A turn that did not fail attests nothing.
    ///
    /// This is what lets the client read absence as *"this server said nothing"*
    /// rather than as `contract` — the asymmetry `stamp_turn_failure_class`'s doc
    /// comment states, and the reason a binary predating mika#2522 keeps its old
    /// behaviour exactly.
    #[test]
    fn mika2522_a_successful_turn_carries_no_failure_class() {
        let mut task = completed_task(Some(agent_reply("here is your answer")));
        // The success path stamps the other three and never this one.
        stamp_effective_model(&mut task, Some("openrouter/moonshotai/kimi-k2.5"));
        stamp_session_isolation(&mut task, false);

        assert_eq!(mika_a2a::params::attested_turn_failure_class(&task), None);
        assert!(
            !task
                .metadata
                .as_ref()
                .expect("the two siblings wrote a map")
                .contains_key(TURN_FAILURE_CLASS_KEY),
            "the failure class must be absent from a successful turn, not empty"
        );
    }

    /// **V4.** The audit line is degraded by an unreadable model, never
    /// suppressed by one.
    ///
    /// The `target_key` this produces is what an operator groups by. An empty
    /// key would make the count of turns lost on an unnameable model
    /// indistinguishable from an absent group — so the whole population AC2
    /// measures would be short by exactly the rows hardest to explain.
    #[test]
    fn mika2522_an_unreadable_model_degrades_the_line_and_never_drops_it() {
        assert_eq!(
            audit_model_label("openrouter/moonshotai/kimi-k2.5"),
            "openrouter/moonshotai/kimi-k2.5"
        );
        // `ResolvedBudgetRecord::model` is `""` when the provider itself could
        // not be parsed (mika#2328's `unknown_provider`).
        assert_eq!(audit_model_label(""), "unknown");
        assert_eq!(audit_model_label("   "), "unknown");
        // Whitespace around a real value is trimmed rather than carried into the
        // group key: `" kimi"` and `"kimi"` must not become two populations.
        assert_eq!(audit_model_label("  zai/glm-5.2  "), "zai/glm-5.2");
    }

    /// **V1, structural.** The stamp has exactly one call site, and it is the
    /// failure branch.
    ///
    /// No behavioural test in this module can see the composition: the `match`
    /// on `turn_text` is inside an async handler that needs a live server. What
    /// a scan *can* see is that nobody stamps a class anywhere else — and a
    /// second site, on the success branch or elsewhere, would make the absence
    /// of the key stop meaning "this server said nothing", which is the whole
    /// contract the client rests on.
    #[test]
    fn mika2522_the_failure_class_is_stamped_from_one_branch_only() {
        const CALL_SITE: &str = concat!("stamp_turn_failure", "_class(&mut task");
        // Scoped to the shipped half of the file, like its mika#2304 neighbour:
        // the round-trip test above legitimately calls the stamp to walk the
        // chain, and it is the production code that must have one site.
        let production = include_str!("a2a.rs")
            .split_once("#[cfg(test)]")
            .expect("this module has a test section")
            .0;

        let sites = production.matches(CALL_SITE).count();
        assert_eq!(
            sites, 1,
            "mika#2522 — expected exactly one call site for the failure-class \
             stamp, found {sites}. A second one would make an absent key stop \
             meaning `this server attested nothing`."
        );

        // And that site is inside the `LoopFailed` arm. Anchored on the arm's
        // own pattern so a stamp moved to the `Produced` arm goes red.
        const FAILURE_ARM: &str = concat!(
            "TurnText::LoopFailed { class } => {\n",
            "                        stamp_turn_failure",
            "_class(&mut task, class);\n"
        );
        assert!(
            production.contains(FAILURE_ARM),
            "mika#2522 — the stamp must sit in the `LoopFailed` arm of the \
             `turn_text` match; it is what ties the class to the failure that \
             produced it"
        );
    }

    /// **Both ports refuse the same way (D2, AC3).** `message/send` and
    /// `message/stream` share `MessageSendParams`; a key honoured on one and
    /// ignored on the other would mean two different things on two endpoints of
    /// the same protocol. Structural because standing both handlers up needs a
    /// live server, and the property is about the *number of call sites*, which a
    /// behavioural test on either one cannot see.
    #[test]
    fn mika2304_both_ports_resolve_the_override_before_creating_a_task() {
        // `concat!` again: the needles must not be found in this test's own body.
        const RESOLVE_SITE: &str = concat!("resolve_caller_model_override", "(agent_state");
        // Anchored on the override resolution so the count is this key's refusal
        // alone: the same `return` also answers a malformed
        // `mika.session_isolated` (mika#1951), which is a different refusal.
        const REFUSAL_SITE: &str = concat!(
            "resolve_caller_model_override",
            "(agent_state, &params, &task_id) {\n",
            "        Ok(p) => p,\n",
            "        Err(err) => {\n",
            "            return Json(JsonRpcResponse::error(request.id.clone(), ",
            "err)).into_response();"
        );

        let source = include_str!("a2a.rs");
        assert_eq!(
            source.matches(RESOLVE_SITE).count(),
            2,
            "both `message/send` and `message/stream` must resolve (and be able to \
             refuse) the caller's model — on the same key, with the same policy"
        );
        // The refusal must reach the caller as a JSON-RPC error. Returning it as a
        // `failed` task would leave the reason in the server log only, which is
        // the shape the founding ticket had to work around by reading bodies.
        assert_eq!(
            source.matches(REFUSAL_SITE).count(),
            2,
            "each port must answer a refused override with a JSON-RPC error naming it"
        );
    }

    /// **T11 — propagation and attestation describe the same model.**
    ///
    /// T1…T6 each check one half from its own side, and none of them can see a
    /// wiring where both halves work while naming two different models — which
    /// is the shape that would put the operator back where mika#2304 found them,
    /// reading an authoritative field about a turn that ran elsewhere.
    ///
    /// The chain is walked on the real types, minus HTTP: request metadata →
    /// server read → provider → attestation → `Task.metadata` → client read. The
    /// client's two ends are the crate-shared constants, so this test and
    /// `remote_ask`'s `send_params_carry_the_model_override_unresolved` /
    /// `verbose_reports_the_attested_model_on_both_formats` meet on the same two
    /// keys — which is exactly why those keys live in `mika-a2a` (D6).
    ///
    /// Standing the two handlers up would need a live server and a reachable
    /// provider; the same boundary `a2a_caller_session_correlation.rs` draws for
    /// mika#2070, and for the same reason.
    #[test]
    fn mika2304_the_propagated_model_and_the_attested_model_are_the_same() {
        // 1. What the CLI puts on the wire (raw — the server owns resolution).
        let params = params_with_metadata(Some(HashMap::from([(
            MODEL_OVERRIDE_KEY.to_string(),
            serde_json::Value::String("moonshotai/kimi-k2.5".to_string()),
        )])));

        // 2. What this server reads back.
        let requested = requested_model_override(&params).expect("the key must be read");

        // 3. The provider the turn will run under.
        let settings = openrouter_settings(Some("sk-or-xxx"));
        let Ok(provider) = caller_model_provider(&settings, requested) else {
            panic!("a configured provider must honour the caller's model");
        };

        // 4. What the loop attests, taken on the provider that serves.
        let attested = crate::agent_loop::attest_effective_model(provider.as_ref());

        // 5. What the caller receives.
        let mut task = completed_task(Some(agent_reply("ok")));
        stamp_effective_model(&mut task, Some(&attested));

        // 6. What the client reads, through the shared reader.
        assert_eq!(
            mika_a2a::attested_model(&task),
            Some("openrouter/moonshotai/kimi-k2.5"),
            "the model the caller asked for, the model that served, and the model \
             attested back must be one model — two of them agreeing is not enough"
        );
        assert!(
            attested.ends_with(requested),
            "the attestation must carry the requested id, not a configured one: \
             requested {requested}, attested {attested}"
        );
    }

    // --- mika#2379: an A2A turn always ends in a terminal state ---------------

    async fn guard_db() -> crate::async_db::AsyncDatabase {
        let db = crate::async_db::AsyncDatabase::new_with_agent(
            crate::db::Database::open_in_memory().unwrap(),
            "mika",
        );
        db.a2a_create_task("t1", None, None).await.unwrap();
        db
    }

    async fn row(db: &crate::async_db::AsyncDatabase) -> (String, Option<String>) {
        db.with_db(|d| {
            Ok(d.conn.query_row(
                "SELECT t.status, t.result FROM tasks t
                 JOIN a2a_task_map m ON m.task_id = t.id WHERE m.a2a_task_id = 't1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap()
    }

    /// The guard's write is spawned from `Drop`; give it a bounded chance to land.
    async fn settled_row(db: &crate::async_db::AsyncDatabase) -> (String, Option<String>) {
        for _ in 0..200 {
            let r = row(db).await;
            if r.0 != "in_progress" && r.0 != "pending" {
                return r;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        row(db).await
    }

    #[tokio::test]
    async fn mika2379_an_armed_guard_dropped_cancels_the_row() {
        let db = guard_db().await;
        db.a2a_update_task_state("t1", "working").await.unwrap();

        drop(TurnGuard::arm(&db, "t1", "send"));

        let (status, result) = settled_row(&db).await;
        assert_eq!(status, "cancelled");
        assert_eq!(
            result.as_deref(),
            Some(crate::a2a_db::a2a_close_reason_metadata(TURN_ABANDONED_REASON).as_str())
        );
    }

    #[tokio::test]
    async fn mika2379_a_disarmed_guard_leaves_the_terminal_write_alone() {
        let db = guard_db().await;
        db.a2a_update_task_state("t1", "working").await.unwrap();
        let mut guard = TurnGuard::arm(&db, "t1", "send");
        db.a2a_update_task_state("t1", "completed").await.unwrap();
        guard.disarm();
        drop(guard);

        // Nothing was spawned; a short wait proves no late write lands either.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(row(&db).await.0, "completed");
    }

    #[tokio::test]
    async fn mika2379_a_guard_dropped_after_a_terminal_write_does_not_overwrite_it() {
        let db = guard_db().await;
        let guard = TurnGuard::arm(&db, "t1", "send");
        db.a2a_update_task_state("t1", "failed").await.unwrap();
        drop(guard); // still armed: the race the conditional write exists for

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(row(&db).await, ("failed".to_string(), None));
    }

    /// The disconnect shape: hyper drops the handler future while it is parked on
    /// the agent turn. Here the turn is a future that never resolves.
    #[tokio::test]
    async fn mika2379_dropping_the_turn_future_mid_await_cancels_the_row() {
        let db = guard_db().await;
        db.a2a_update_task_state("t1", "working").await.unwrap();
        let turn_db = db.clone();
        let turn = async move {
            let mut guard = TurnGuard::arm(&turn_db, "t1", "send");
            std::future::pending::<()>().await;
            guard.disarm();
        };
        // The caller's budget runs out: `timeout` drops the parked future, which
        // is exactly what hyper does to the handler when the client hangs up.
        let outcome = tokio::time::timeout(std::time::Duration::from_millis(20), turn).await;
        assert!(
            outcome.is_err(),
            "the turn must still be parked when dropped"
        );

        assert_eq!(settled_row(&db).await.0, "cancelled");
    }

    /// A terminal write that fails leaves the guard armed, so the row is still
    /// closed rather than left `in_progress` until the next restart.
    #[tokio::test]
    async fn mika2379_a_failed_terminal_write_keeps_the_guard_armed() {
        let db = guard_db().await;
        db.a2a_update_task_state("t1", "working").await.unwrap();
        let mut guard = TurnGuard::arm(&db, "t1", "send");
        guard.settle(Err(anyhow::anyhow!("database is locked")));
        drop(guard);

        assert_eq!(settled_row(&db).await.0, "cancelled");
    }

    #[tokio::test]
    async fn mika2379_a_landed_terminal_write_disarms_the_guard() {
        let db = guard_db().await;
        db.a2a_update_task_state("t1", "working").await.unwrap();
        let mut guard = TurnGuard::arm(&db, "t1", "send");
        guard.settle(db.a2a_update_task_state("t1", "completed").await);
        drop(guard);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(row(&db).await.0, "completed");
    }

    #[tokio::test]
    async fn mika2379_a_panicking_stream_turn_cancels_the_row() {
        let db = guard_db().await;
        db.a2a_update_task_state("t1", "working").await.unwrap();
        let turn_db = db.clone();
        let joined = tokio::spawn(async move {
            let _guard = TurnGuard::arm(&turn_db, "t1", "stream");
            panic!("simulated turn panic");
        })
        .await;
        assert!(joined.unwrap_err().is_panic());

        assert_eq!(settled_row(&db).await.0, "cancelled");
    }
}
