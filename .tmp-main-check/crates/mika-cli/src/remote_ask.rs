//! Remote-mode dispatch for `mika ask`.
//!
//! When `--remote <URL>` (or `MIKA_REMOTE_AGENT_URL`) is set, `mika ask` bypasses
//! the local in-process agent loop and dispatches the user's prompt to a remote
//! Mika agent via the A2A protocol (see `mika-a2a`). The remote URL points at the
//! gateway's per-agent A2A proxy endpoint, e.g.,
//! `https://gw.example.com/a2a/{customer_id}/{agent_name}`.
//!
//! Authentication is bearer-token via the existing `MIKA_INTERNAL_TOKEN` env var,
//! matching the gateway's internal-token contract.
//!
//! See `docs/plans/2026-06-09-003-feat-ascension-architecture-first-slice-cli-plan.md`
//! and the ascension architecture brainstorm (`docs/brainstorms/2026-06-09-...`) for
//! the broader context — local↔cloud Mika portability, R1 daily-use unblock slice.

use std::time::Duration;

use anyhow::{Context, Result};
use mika_a2a::client::{A2aClient, RECOVERY_TIMEOUT};
use mika_a2a::error::TransportFailure;
pub use mika_a2a::render::{EmptyKind, TaskRenderEmpty};
use mika_a2a::{A2aError, Message, MessageSendParams, Part, Role, Task, TaskState};
pub use mika_a2a::{
    CALLER_SESSION_ID_KEY, EFFECTIVE_MODEL_KEY, MODEL_OVERRIDE_KEY, ONLY_SKILLS_KEY, RUN_USAGE_KEY,
    RunUsage, SESSION_ISOLATED_APPLIED_KEY, SESSION_ISOLATED_KEY, TURN_FAILURE_CLASS_KEY,
    attested_model, attested_run_usage, attested_session_isolation, attested_turn_failure_class,
};
use uuid::Uuid;

/// Output format selector. Mirrors `crate::cli::OutputFormat` to keep the
/// remote-mode dispatch decoupled from the binary's clap layer (so integration
/// tests in `tests/` can call this directly without pulling in the full CLI).
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Text,
    Json,
}

/// Exit code for a transport-class failure — `EX_TEMPFAIL` from `sysexits.h`,
/// "temporary failure, retry later" (mika#2278).
///
/// It collides with nothing: this CLI emits only `0` and `1` of its own, plus
/// clap's own codes for an argument error. The other `mika ask` callers under
/// `skills/bundled/**` are all `--task-complete`, a path that returns *before*
/// the A2A send and therefore can never produce this code.
pub const EXIT_TRANSPORT_FAILURE: i32 = 75;

/// Whether a `mika ask` failure is worth attempting again (mika#2278).
///
/// # The class comes from the variant, never from the message text
///
/// The caller that needs this distinction is a shell loop, and the cheapest
/// thing it could have done is `grep` the error text for "Retry." — which would
/// turn a sentence written for a human into a wire format. The house has ruled
/// on this twice already: mika#2179 takes its error classes from the `LlmError`
/// *variant* via `downcast_ref` and never from a substring of the rendered
/// message, and mika#2291 triggers its fallback on the HTTP *status* and never
/// on a substring of Telegram's `description`. So the class travels as a typed
/// marker on the error ([`TransportClass`]) and surfaces as a dedicated process
/// exit code; not one byte of any message changes.
///
/// # Which way the doubtful cases lean, and why
///
/// A false `Transport` costs one architect turn paid twice — a few minutes and
/// a few cents. A false `Contract` costs the whole pass, the dispatch slot, and
/// one point of the re-drive budget; three of those abandon a healthy ticket in
/// `operator-review` (mika#2020). The costs are not of the same order, so the
/// transport family leans entirely towards `Transport` — bounded to a single
/// retry, like every budget in this codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Transient: the exchange failed on the way, and the same request may well
    /// succeed. Surfaces as [`EXIT_TRANSPORT_FAILURE`].
    Transport,
    /// Definitive: a broken contract, a usage error, or a turn the server ran
    /// and refused. Re-sending the same request does not change the answer.
    Contract,
}

/// A failure the caller may retry, carried as a type on the `anyhow` chain.
///
/// The marker *is* the error: its `Display` is the operator-facing message, so
/// attaching the class costs the printed text nothing. Readers ask
/// [`is_transport_failure`], never a string comparison.
#[derive(Debug)]
pub struct TransportClass {
    message: String,
}

impl TransportClass {
    /// Wrap an already-composed operator-facing message as transport-class.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TransportClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TransportClass {}

/// Build a failure carrying its class, with `message` as the visible text.
///
/// Both arms render identically under `{e}` and `{e:#}`; only the type differs.
fn classified(class: FailureClass, message: String) -> anyhow::Error {
    match class {
        FailureClass::Transport => anyhow::Error::new(TransportClass::new(message)),
        FailureClass::Contract => anyhow::anyhow!("{message}"),
    }
}

/// Whether this failure is transport-class.
///
/// Walks the whole `anyhow` cause chain rather than inspecting only the
/// outermost error: `wrap_send_error` re-composes the message at the CLI
/// boundary, and a future context layer must not silently demote the class.
pub fn is_transport_failure(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.is::<TransportClass>())
}

/// The process exit code a failed `mika ask` must leave behind.
///
/// Fail-safe in the house direction (mika#2278 D4): anything not *positively*
/// known to be transport exits `1`. A shell wrapper retries on `75` only, so an
/// unforeseen failure mode becomes a loud single failure rather than a silent
/// retry loop.
pub fn exit_code_for(err: &anyhow::Error) -> i32 {
    if is_transport_failure(err) {
        EXIT_TRANSPORT_FAILURE
    } else {
        1
    }
}

/// Class of an [`A2aError`] raised by `message/send`.
///
/// Exhaustive on purpose, with no `_` arm: a new variant must be classified by
/// whoever adds it, not silently absorbed into the safe-looking default.
fn a2a_error_class(err: &A2aError) -> FailureClass {
    match err {
        // Everything reqwest could not complete: connection refused (the
        // restart this ticket is about — the port simply does not answer),
        // timeout, non-2xx status, unreadable body, socket closed mid-flight.
        A2aError::ClientError(_) => FailureClass::Transport,
        // A malformed envelope, a body that would not serialize, a state
        // machine that refused a transition. None of these is repaired by
        // sending the same thing again.
        A2aError::InvalidJsonRpc(_)
        | A2aError::SerializationError(_)
        | A2aError::InvalidStateTransition { .. } => FailureClass::Contract,
    }
}

/// Whether a server-attested failure class is one a second attempt may clear
/// (mika#2522).
///
/// The two spellings are **imported** from `mika-common`, never retyped: they
/// are the wire format `llm_call_attempt`, `audit_events.callback_delivery_failed`
/// (mika#2179) and `qa_deadline_verdict` already share, and a copy here would be
/// the population split `error_class`'s own doc comment exists to prevent.
fn is_transport_class(class: &str) -> bool {
    use mika_common::llm::error::error_class;
    class == error_class::TRANSPORT || class == error_class::TRANSPORT_TIMEOUT
}

/// Class of a `Task` state that a synchronous `message/send` should not have
/// returned, or returned as a refusal.
///
/// Exhaustive for the same reason as [`a2a_error_class`].
///
/// `attested_class` is the server's own reading of *why* a `failed` turn failed
/// (mika#2522), taken off the Task via
/// [`mika_a2a::params::attested_turn_failure_class`]. `None` means the server
/// attested nothing.
fn terminal_state_class(state: TaskState, attested_class: Option<&str>) -> FailureClass {
    match state {
        // Async-dispatch states the server owes us no answer in: per A2A v0.3
        // §6 a synchronous send returns a terminal-or-pending state, so seeing
        // one of these is an anomaly of the server's own making — exactly the
        // kind a second attempt clears.
        TaskState::Submitted | TaskState::Working | TaskState::Unknown => FailureClass::Transport,

        // mika#2522 — `failed` has two causes of opposite natures, and the
        // server is the only party that can tell them apart.
        //
        // The comment this line replaces read: "A turn the server actually ran
        // and ended without an answer. Re-sending the same brief does not change
        // that verdict." True of a reasoned refusal; **false of a turn a
        // transport cut killed** — nothing was refused, and `LlmError::is_retryable`
        // says the opposite of that verdict. Measured 2026-09-24: 11 OpenRouter
        // `body read failed mid-stream` cuts, 5 grooms of mika#2515 lost, every
        // one of them exiting 1 so `_arch_ask_with_retry` (which replays on 75
        // alone) never armed. That case is the twin of the exception the
        // parenthesis below already named.
        //
        // Fail-closed: ONLY a positively attested transport class flips. An
        // absent key (a server predating mika#2522, `message/stream`,
        // `returnImmediately`), an unreadable one, and any non-transport class
        // all stay `Contract` — byte for byte the behaviour before this change.
        TaskState::Failed => match attested_class {
            Some(c) if is_transport_class(c) => FailureClass::Transport,
            _ => FailureClass::Contract,
        },

        // A cancellation and a refusal are decisions, not accidents, so the
        // attestation is deliberately not consulted here. (The *other* `failed`
        // — the one `startup_recovery` writes on a turn a restart killed — still
        // never reaches this site: its exchange died at transport and is read
        // through `Recovery::Ended` inside the `ClientError` arm above.)
        TaskState::Canceled | TaskState::Rejected => FailureClass::Contract,

        // Not failures at all; listed so that adding a state cannot compile
        // without a decision being taken about it.
        TaskState::Completed | TaskState::InputRequired | TaskState::AuthRequired => {
            FailureClass::Contract
        }
    }
}

/// Render an A2A `Task`'s text content, or report that there is none to render.
///
/// Thin CLI-facing name for [`mika_a2a::render::render_task_text`]. The reading
/// itself lives in the protocol crate because mika-spirit's `message/send` needs
/// the *same* verdict this side reaches: the server's mika#2270 net fires exactly
/// when this function would fail, so the two cannot drift into disagreeing.
///
/// # The three tiers are one query, not three sources (mika#2270)
///
/// The doc comment this replaces promised defence in depth — artifacts, then
/// agent-role history, then `status.message`, "so reading only `status.message`
/// would silently render empty". Against a lost turn that promise is empty, and
/// believing it is what sent mika#2270's investigation at this function instead
/// of upstream:
///
/// * `Database::a2a_insert_artifact` has no production caller, so a Task built by
///   mika-spirit never carries artifacts (tier 1 survives only for `--remote`,
///   where a spec-conformant server may populate it);
/// * `a2a_build_task` derives `status.message` from the last agent-role message
///   of `history`, and `history` is `a2a_get_messages`' result — so tiers 2 and 3
///   fail **together**, on one query.
///
/// A renderer cannot rescue what the Task does not carry. What it can do is
/// refuse to answer an empty string, which is why this returns a `Result`: an
/// empty rendering used to be indistinguishable from an agent with nothing to
/// say, at exit 0. See [`mika_a2a::render`] for the full reasoning.
pub fn render_task_parts(task: &Task) -> Result<String, TaskRenderEmpty> {
    mika_a2a::render::render_task_text(task)
}

/// Build a `MessageSendParams` containing the user's single text prompt.
///
/// `caller_session_id` is the sender's own session id, carried in request
/// metadata under [`CALLER_SESSION_ID_KEY`]. `None` leaves `metadata` absent so
/// the serialized body is byte-identical to the pre-mika#2070 shape.
///
/// `context_id` is the caller's own handle on this exchange (mika#2036). The
/// server persists it beside the task it mints, which makes it the one name the
/// caller can still use to find its answer if the response never arrives.
///
/// `only_skills` names the skills the turn should keep, under
/// [`ONLY_SKILLS_KEY`] (mika#2363). Empty leaves the key absent.
///
/// `model_override` is the model id **as the operator typed it**, under
/// [`MODEL_OVERRIDE_KEY`] (mika#2304). Deliberately raw: alias resolution and
/// prefix stripping depend on the *executing* agent's `llm_provider`
/// (mika#1591), which on `--remote` is not this machine's. Resolving here would
/// send an id resolved against the wrong provider — a second false green,
/// quieter than the first.
///
/// `session_isolated` asks the server to run this one turn under a
/// session-scoped conversation window with no compaction summary, under
/// [`SESSION_ISOLATED_KEY`] (mika#1951). `false` leaves the key **absent**
/// rather than posting `false`: absent and `false` mean the same thing to the
/// server, and absent is the shape that keeps a non-declaring caller
/// byte-identical to one that predates the key.
///
/// The four metadata keys are independent and any may be absent. When all are,
/// `metadata` itself stays absent so the serialized body is byte-identical to
/// the pre-mika#2070 shape — the property that makes a caller declaring nothing
/// indistinguishable from a caller that predates these keys.
///
/// **This is the single site where `mika ask`'s request metadata is built, and
/// that is what makes the isolation flag reach both doors.** Since mika#1727 the
/// default path is an A2A client too, so it and `--remote` both arrive here
/// through [`send_message_to_agent`]. mika#2304 had to repair exactly that half
/// after a fix aimed only at `--remote`; posting the key here gives it to both,
/// and `mika1951_both_ask_doors_post_the_isolation_key_through_one_site` asserts
/// it rather than assuming it.
fn build_send_params(
    message: &str,
    caller_session_id: Option<&str>,
    context_id: &str,
    only_skills: &[String],
    model_override: Option<&str>,
    session_isolated: bool,
) -> MessageSendParams {
    let mut fields = std::collections::HashMap::new();
    if let Some(sid) = caller_session_id {
        fields.insert(
            CALLER_SESSION_ID_KEY.to_string(),
            serde_json::Value::String(sid.to_string()),
        );
    }
    if !only_skills.is_empty() {
        // A `&[String]` serializes to a JSON array of strings, which is exactly
        // the shape the server reads.
        fields.insert(ONLY_SKILLS_KEY.to_string(), serde_json::json!(only_skills));
    }
    if let Some(model) = model_override {
        fields.insert(
            MODEL_OVERRIDE_KEY.to_string(),
            serde_json::Value::String(model.to_string()),
        );
    }
    if session_isolated {
        // A JSON bool, which is the one shape the server reads; anything else
        // fails the request rather than degrading to "not isolated" (mika#1951).
        fields.insert(
            SESSION_ISOLATED_KEY.to_string(),
            serde_json::Value::Bool(true),
        );
    }
    let metadata = if fields.is_empty() {
        None
    } else {
        Some(fields)
    };
    MessageSendParams {
        message: Message {
            message_id: Uuid::new_v4().to_string(),
            role: Role::User,
            parts: vec![Part::Text {
                text: message.to_string(),
                metadata: None,
            }],
            context_id: Some(context_id.to_string()),
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

/// What a recovery read found on the server after a failed exchange.
///
/// These variants are the distinction the founding incident could not make. On
/// 2026-09-04 a scripted caller retried `mika ask --agent mika-qa` eight times
/// over twenty minutes with a growing backoff and could not tell "the agent is
/// busy, retry" from "your answer exists and was dropped on the way back" — and
/// only the second of those justifies an escalation.
#[derive(Debug)]
enum Recovery {
    /// The generation finished and is being returned. It was produced, then
    /// lost at transport — the whole point of mika#2036.
    Recovered(Box<Task>),
    /// A task exists but has not finished. The answer does not exist *yet*;
    /// retrying is the right move.
    StillRunning { task_id: String, state: TaskState },
    /// A task exists and ended without a usable answer.
    Ended { task_id: String, state: TaskState },
    /// The server holds no task under this context. Either the request never
    /// landed, or it was refused before a task was created — a busy agent
    /// refuses at the lock, before `a2a_create_task`.
    NoTask,
    /// The recovery read itself failed, so whether an answer exists is unknown.
    Unavailable(String),
}

/// Ask the server what became of the exchange named by `context_id`.
///
/// Uses [`RECOVERY_TIMEOUT`] rather than the send budget: this is a database
/// read, and a caller already past one failure should not wait another five
/// minutes to learn whether its answer survived.
async fn recover_by_context(url: &str, auth_token: Option<String>, context_id: &str) -> Recovery {
    let client = A2aClient::with_timeout(url, auth_token, RECOVERY_TIMEOUT);
    match client.get_task(context_id, None).await {
        Ok(Some(task)) => match task.status.state {
            TaskState::Completed | TaskState::InputRequired | TaskState::AuthRequired => {
                Recovery::Recovered(Box::new(task))
            }
            TaskState::Submitted | TaskState::Working | TaskState::Unknown => {
                Recovery::StillRunning {
                    task_id: task.id,
                    state: task.status.state,
                }
            }
            TaskState::Failed | TaskState::Canceled | TaskState::Rejected => Recovery::Ended {
                task_id: task.id,
                state: task.status.state,
            },
        },
        Ok(None) => Recovery::NoTask,
        Err(e) => Recovery::Unavailable(e.to_string()),
    }
}

/// Build the operator-facing message for an exchange that failed and could not
/// be recovered.
///
/// Kept pure so every branch can be asserted without a network. The point of
/// mika#2036 is that these sentences *differ*; a test that cannot compare them
/// cannot defend the difference.
///
/// `recovery` is `None` when no recovery was attempted — which happens only
/// when the request never reached the server, and is itself information the
/// caller needs.
fn transport_error_message(
    failure: TransportFailure,
    url: &str,
    timeout: Duration,
    context_id: &str,
    recovery: Option<&Recovery>,
) -> String {
    let head = failure.describe(url, timeout);
    let tail = match recovery {
        None => "the request never left this client, so no answer exists to reclaim".to_string(),
        Some(Recovery::NoTask) => format!(
            "the server holds no task for context {context_id} — the request did not land, or was              refused before work started (a busy agent refuses at the lock). Retry."
        ),
        Some(Recovery::StillRunning { task_id, state }) => format!(
            "the server is still working on it (task {task_id}, state '{state}', context              {context_id}) — the answer does not exist yet. Retry."
        ),
        Some(Recovery::Ended { task_id, state }) => format!(
            "the server's task {task_id} ended in state '{state}' (context {context_id}) — it will              not produce an answer."
        ),
        Some(Recovery::Unavailable(why)) => format!(
            "an answer may exist server-side but the recovery read failed ({why}) — look it up              with tasks/get on context {context_id}"
        ),
        // Never reached: a recovered task is returned, not reported as an error.
        Some(Recovery::Recovered(task)) => {
            format!("recovered task {} (context {context_id})", task.id)
        }
    };
    format!("{head}; {tail}")
}

/// Send a single user message to an agent's A2A endpoint via `message/send` and
/// return the terminal `Task`.
///
/// Shared core for both the `--remote` cloud path (`dispatch_remote`) and the
/// local mika-spirit thin-client path (`commands::ask`, mika#1727). Handles auth,
/// dispatch, and terminal-state validation; rendering is left to the caller so
/// each surface can apply its own output shape.
///
/// Auth is bearer-token via `MIKA_INTERNAL_TOKEN`. An empty value is treated as
/// unset — set-but-empty (common with a misconfigured `.env`) would otherwise
/// forward `Authorization: Bearer `, which the server 401s with no diagnostic
/// hint; treating empty as unset surfaces the same 401 but matches the no-auth
/// diagnostic shape. Per `mika/CLAUDE.md`, `secrecy::SecretString` guards the
/// `Settings` accessor boundary; `A2aClient::new` takes `Option<String>` as the
/// downstream boundary, so a plain-String read here is consistent, and
/// `A2aClient` does not derive `Debug`, so accidental log leakage is constrained.
///
/// The caller is responsible for URL validation with a surface-appropriate error
/// message (e.g. `--remote` vs. the local spirit endpoint).
pub async fn send_message_to_agent(
    message: &str,
    url: &str,
    caller_session_id: Option<&str>,
    only_skills: &[String],
    model_override: Option<&str>,
    session_isolated: bool,
) -> Result<Task> {
    let auth_token = std::env::var("MIKA_INTERNAL_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    let client = A2aClient::new(url, auth_token.clone());

    // The caller's own handle on this exchange. Minted before the send so it
    // survives the send failing: the task id is server-side and comes back only
    // in the envelope that may be lost (mika#2036).
    let context_id = Uuid::new_v4().to_string();

    let task = match client
        .send_message(build_send_params(
            message,
            caller_session_id,
            &context_id,
            only_skills,
            model_override,
            session_isolated,
        ))
        .await
    {
        Ok(task) => task,
        // The class is read off the variant *before* destructuring (mika#2278):
        // every arm below then carries it without re-deciding, so the exit code
        // and the message can never disagree about what happened.
        Err(err) => {
            let class = a2a_error_class(&err);
            match err {
                A2aError::InvalidJsonRpc(msg) => {
                    return Err(classified(class, format!("remote error: {msg}")));
                }
                A2aError::ClientError(e) => {
                    let failure = TransportFailure::classify(&e);
                    // Only attempt recovery when the request actually left. A server
                    // that was never reached cannot hold a task, and asking it for one
                    // would be a phantom recovery.
                    let recovery = if failure.request_was_sent() {
                        Some(recover_by_context(url, auth_token, &context_id).await)
                    } else {
                        None
                    };
                    match recovery {
                        Some(Recovery::Recovered(recovered)) => {
                            tracing::warn!(
                                context_id = %context_id,
                                task_id = %recovered.id,
                                failure = ?failure,
                                "reclaimed a generated A2A response after a transport failure"
                            );
                            *recovered
                        }
                        other => {
                            return Err(classified(
                                class,
                                transport_error_message(
                                    failure,
                                    url,
                                    client.timeout(),
                                    &context_id,
                                    other.as_ref(),
                                ),
                            ));
                        }
                    }
                }
                A2aError::SerializationError(e) => {
                    return Err(classified(class, format!("serialization error: {e}")));
                }
                A2aError::InvalidStateTransition { from, to } => {
                    return Err(classified(
                        class,
                        format!("invalid state transition from {from} to {to}"),
                    ));
                }
            }
        }
    };

    // Inspect the terminal Task state. Per A2A v0.3 §6, a synchronous
    // `message/send` returns a Task in one of: Completed, Failed, Canceled,
    // Rejected, InputRequired, AuthRequired (terminal-or-pending). Submitted /
    // Working are async-dispatch states the server should not return here.
    //
    // Completed and the pending-input states carry meaningful text the user needs
    // to see. Surface terminal-bad and async-in-progress states as errors so the
    // shell exit code matches the local `mika ask` contract.
    // mika#2522: the server's own reading of why a `failed` turn failed. Read
    // once, here, through the single decoder — `terminal_state_class` decides on
    // the key, and the operator-facing sentence below merely echoes it.
    let attested_class = attested_turn_failure_class(&task).map(str::to_string);

    match task.status.state {
        TaskState::Completed | TaskState::InputRequired | TaskState::AuthRequired => {}
        state @ (TaskState::Failed | TaskState::Canceled | TaskState::Rejected) => {
            // Naming the class is what makes the client's decision readable
            // without opening the server's log: `… ended in state 'failed'
            // (transport)` says why the retry armed, and the bare form says the
            // server attested nothing. The decision still reads the key, never
            // this sentence (mika#2179, mika#2291).
            let attribution = match attested_class.as_deref() {
                Some(c) => format!(" ({c})"),
                None => String::new(),
            };
            return Err(classified(
                terminal_state_class(state, attested_class.as_deref()),
                format!(
                    "remote task {} ended in state '{}'{attribution}",
                    task.id, state
                ),
            ));
        }
        state @ (TaskState::Submitted | TaskState::Working | TaskState::Unknown) => {
            return Err(classified(
                terminal_state_class(state, attested_class.as_deref()),
                format!(
                    "remote task {} is still in state '{state}' — sync dispatch expected a terminal state",
                    task.id
                ),
            ));
        }
    }

    Ok(task)
}

/// Dispatch a `mika ask` invocation in remote mode and return the rendered output.
///
/// Validates `remote_url`, sends the prompt via `send_message_to_agent`, and
/// renders the returned `Task` to a string in the requested format. Returns the
/// rendered string rather than printing it so integration tests can assert on the
/// output without capturing stdout. `run_remote` wraps this with a print.
///
/// Errors are surfaced as `anyhow` failures with single-line prefixes per
/// `A2aError` variant so the caller can `eprintln!("Error: {e}")` and exit
/// non-zero. A transport failure additionally reports what became of the work
/// on the other side — see [`transport_error_message`].
pub async fn dispatch_remote(
    message: &str,
    remote_url: &str,
    format: OutputFormat,
    verbose: bool,
    model_override: Option<&str>,
    session_isolated: bool,
) -> Result<String> {
    // Fail-fast URL validation. A2aClient itself doesn't pre-parse, so an invalid
    // URL would surface as a reqwest send error — a less actionable message.
    reqwest::Url::parse(remote_url)
        .with_context(|| format!("invalid --remote URL: {remote_url}"))?;

    // Two of the four `mika.*` keys are deliberately NOT sent on this path; the
    // other two are (mika#2304, mika#1951). What separates them is what each one
    // *names*.
    //
    // `--remote` sends no caller session id (mika#2070). The local bookkeeping
    // session lives in this machine's database; a remote agent normally holds a
    // different one and would refuse the id. A single-host deployment where the
    // gateway proxies back to this same spirit is the exception — correlation
    // would work there — but `--remote` is not the measured path, so we do not
    // count on it.
    // `--remote` sends no skill restriction either (mika#2363): `--only-skill`
    // is refused in team mode but accepted with `--remote`, and a remote agent's
    // skill names are not this machine's to guess. The flag is honoured on the
    // local spirit path, which is the one `_arch_ask` uses.
    //
    // Both refusals rest on the same property: each key names a **local
    // reference the caller would be guessing** — a row of this machine's
    // database, a name in this machine's skill registry. A model id is not one.
    // It is a string the operator typed, resolved against the *executing*
    // agent's provider (which is why it travels raw), and whose validity is a
    // property of that remote agent, not an inference of this one. It is also
    // what mika#2304 is about by name — the ticket's title says `--remote`.
    //
    // The fail-closed policy is what makes the asymmetry safe: an id the remote
    // cannot serve fails the request with a sentence naming the model and the
    // provider — the ticket's own fallback option ("faire échouer avec un
    // message clair"), reached without giving up the capability.
    //
    // `--isolated` travels for the same reason (mika#1951): it names nothing
    // local. It is a property the caller asks of the turn — read your history
    // session-scoped, inject no summary — and it is meaningful against any
    // agent, on any host, without this machine knowing that agent's identity.
    // It is also strictly restrictive by the shape of its wire value, so
    // sending it to a remote agent cannot widen anything there.
    let task = send_message_to_agent(
        message,
        remote_url,
        None,
        &[],
        model_override,
        session_isolated,
    )
    .await?;
    render(&task, format, verbose)
}

/// Run remote dispatch and write the result to stdout.
///
/// Thin binary-side wrapper around `dispatch_remote`. Tests should target
/// `dispatch_remote` directly to avoid stdout-capture complications.
pub async fn run_remote(
    message: &str,
    remote_url: &str,
    format: OutputFormat,
    verbose: bool,
    model_override: Option<&str>,
    session_isolated: bool,
) -> Result<()> {
    let output = dispatch_remote(
        message,
        remote_url,
        format,
        verbose,
        model_override,
        session_isolated,
    )
    .await?;
    println!("{output}");
    Ok(())
}

/// Format a `Task` for stdout, or fail naming what was inspected (mika#2270).
///
/// The failure is propagated bare, with no `with_context` on top: the outer CLI
/// printer shows only the top layer of the chain (mika#1985), so wrapping it
/// would hide the slice census that is the whole point of failing here. Both
/// formats share this one gate, which is why `--format json` cannot emit
/// `"content": ""` while `--format text` errors, or the reverse.
/// mika#2304: under `--verbose` this path now also reports the model the server
/// attested. It reports **nothing** when the server attested nothing — a remote
/// agent running a binary older than mika#2304 lands there, and it is exactly the
/// population where printing anything local would be a lie. Note this is an
/// *addition* on this path, not the repair of a false statement: `--remote` never
/// displayed a model at all. The measured false green of the founding ticket is
/// the local path's (`commands::ask`), and over-attributing it here would
/// misdescribe the fix.
fn render(task: &Task, format: OutputFormat, verbose: bool) -> Result<String> {
    let rendered = render_task_parts(task)?;
    let model = attested_model(task);
    // mika#1951: the isolation reported is the server's, read the same way the
    // model is, and for the same reason. The flag this process was passed is
    // never consulted here — a spirit that ignored the key would otherwise be
    // reported as isolated, which is the false green that makes a contaminated
    // bench look valid.
    let isolated = attested_session_isolation(task);
    // mika#1883: the per-turn token total, read through the same `mika-a2a`
    // function `commands::ask` calls. Two surfaces answering the same question
    // with two readers is the divergence `attested_model`'s doc comment was
    // written to prevent, and a second decoding site is refused by the source
    // scan `mika1883_both_client_surfaces_read_the_one_reader`.
    let tokens = attested_run_usage(task);
    Ok(match format {
        OutputFormat::Text => {
            if verbose {
                let mut out = format!("{rendered}\n\nremote_task_id: {}", task.id);
                match model {
                    Some(m) => out.push_str(&format!("\nmodel: {m}")),
                    None => {
                        out.push('\n');
                        out.push_str(NO_ATTESTATION_LINE);
                    }
                }
                match isolated {
                    Some(v) => out.push_str(&format!("\nisolated: {v}")),
                    None => {
                        out.push('\n');
                        out.push_str(NO_ISOLATION_ATTESTATION_LINE);
                    }
                }
                // Unlike `model:` and `isolated:` one field up, absence prints
                // **nothing** rather than a "(not attested)" line. Those two
                // answer a flag the caller passed, so silence would leave the
                // operator unable to tell "not attested" from "the trailer
                // changed shape"; this one is a measurement nobody requested,
                // and a line announcing the absence of a number nobody asked
                // for is noise on every turn a server predating the key serves.
                if let Some(u) = tokens {
                    out.push_str(&format!("\ntokens.input: {}", u.input));
                    out.push_str(&format!("\ntokens.output: {}", u.output));
                    if let Some(v) = u.cache_read {
                        out.push_str(&format!("\ntokens.cache_read: {v}"));
                    }
                    if let Some(v) = u.cache_write {
                        out.push_str(&format!("\ntokens.cache_write: {v}"));
                    }
                }
                out
            } else {
                rendered
            }
        }
        OutputFormat::Json => {
            let mut response = serde_json::json!({
                "role": "assistant",
                "content": rendered,
            });
            if verbose {
                let mut metadata = serde_json::json!({
                    "remote_task_id": task.id,
                });
                // Absent, never null and never a local value: an absent key is
                // the honest encoding of "this server did not say".
                if let Some(m) = model {
                    metadata["model"] = serde_json::Value::String(m.to_string());
                }
                if let Some(v) = isolated {
                    metadata["isolated"] = serde_json::Value::Bool(v);
                }
                // mika#1883: absent key, never a null and never a zeroed
                // object — the same encoding of "this server did not say" the
                // two fields above use.
                if let Some(u) = tokens {
                    let mut t = serde_json::json!({ "input": u.input, "output": u.output });
                    if let Some(v) = u.cache_read {
                        t["cache_read"] = serde_json::Value::from(v);
                    }
                    if let Some(v) = u.cache_write {
                        t["cache_write"] = serde_json::Value::from(v);
                    }
                    metadata["tokens"] = t;
                }
                response["metadata"] = metadata;
            }
            serde_json::to_string(&response)?
        }
    })
}

/// What text mode says when the server attested no model (mika#2304, D3).
///
/// A named constant because both `--remote` and `commands::ask` must say the
/// same thing: the two surfaces answer the same question, and two wordings would
/// read as two different situations.
pub const NO_ATTESTATION_LINE: &str = "model: (not attested by the server)";

/// What text mode says when the server attested no session isolation
/// (mika#1951, U3).
///
/// The sibling of [`NO_ATTESTATION_LINE`], named for the same reason: both
/// `--remote` and `commands::ask` must say the same thing, and two wordings
/// would read as two different situations.
///
/// **It is what a caller sees against a server that predates the key** — the
/// population where echoing the local `--isolated` flag would assert an
/// isolation that did not happen. Absence is rendered as absence; it is never a
/// fallback to what this process asked for.
pub const NO_ISOLATION_ATTESTATION_LINE: &str = "isolated: (not attested by the server)";

#[cfg(test)]
mod tests {
    use super::*;
    use mika_a2a::{TaskState, TaskStatus};

    fn task_with_text(text: &str) -> Task {
        Task {
            id: "task-test".to_string(),
            context_id: None,
            status: TaskStatus {
                state: TaskState::Completed,
                message: Some(Message {
                    message_id: "msg-1".to_string(),
                    role: Role::Agent,
                    parts: vec![Part::Text {
                        text: text.to_string(),
                        metadata: None,
                    }],
                    context_id: None,
                    task_id: None,
                    metadata: None,
                    reference_task_ids: None,
                    extensions: None,
                    kind: "message".to_string(),
                }),
                timestamp: None,
            },
            artifacts: None,
            history: None,
            metadata: None,
            kind: "task".to_string(),
        }
    }

    #[test]
    fn render_text_part_emits_text_verbatim() {
        let task = task_with_text("hello world");
        assert_eq!(render_task_parts(&task).unwrap(), "hello world");
    }

    #[test]
    fn render_prefers_artifacts_over_status_message() {
        let mut task = task_with_text("status-fallback-text");
        task.artifacts = Some(vec![mika_a2a::Artifact {
            artifact_id: "art-1".to_string(),
            name: None,
            description: None,
            parts: vec![Part::Text {
                text: "artifact-text".to_string(),
                metadata: None,
            }],
            metadata: None,
            extensions: None,
        }]);
        assert_eq!(render_task_parts(&task).unwrap(), "artifact-text");
    }

    #[test]
    fn render_prefers_agent_history_over_status_message_when_no_artifacts() {
        let mut task = task_with_text("status-fallback-text");
        task.history = Some(vec![Message {
            message_id: "history-msg-1".to_string(),
            role: Role::Agent,
            parts: vec![Part::Text {
                text: "history-text".to_string(),
                metadata: None,
            }],
            context_id: None,
            task_id: None,
            metadata: None,
            reference_task_ids: None,
            extensions: None,
            kind: "message".to_string(),
        }]);
        assert_eq!(render_task_parts(&task).unwrap(), "history-text");
    }

    #[test]
    fn render_ignores_user_role_history_messages() {
        let mut task = task_with_text("status-fallback-text");
        task.history = Some(vec![Message {
            message_id: "history-msg-1".to_string(),
            role: Role::User,
            parts: vec![Part::Text {
                text: "user-prompt".to_string(),
                metadata: None,
            }],
            context_id: None,
            task_id: None,
            metadata: None,
            reference_task_ids: None,
            extensions: None,
            kind: "message".to_string(),
        }]);
        // User-role history is skipped; falls through to status.message
        assert_eq!(render_task_parts(&task).unwrap(), "status-fallback-text");
    }

    // --- mika#2270 (gate #2264): no completed turn renders to the empty string --

    /// **AC4, negative 1 — content living only in `role: User`.**
    ///
    /// This is the shape the ticket describes: a `completed` Task whose content
    /// sits in a slice the renderer does not read. Before mika#2270 it rendered
    /// `""` and `mika ask` exited 0 with `.content` absent — a probe that lies
    /// without failing.
    #[test]
    fn content_only_in_user_role_history_fails_the_render() {
        let mut task = task_with_text("ignored");
        task.status.message = None;
        task.history = Some(vec![Message {
            message_id: "history-msg-1".to_string(),
            role: Role::User,
            parts: vec![Part::Text {
                text: "Disposition: READY".to_string(),
                metadata: None,
            }],
            context_id: None,
            task_id: None,
            metadata: None,
            reference_task_ids: None,
            extensions: None,
            kind: "message".to_string(),
        }]);

        let empty = render_task_parts(&task).expect_err("a completed turn must not render empty");
        assert_eq!(empty.kind, EmptyKind::SlicesUnreadable);
        assert_eq!(empty.history, 1);
        assert_eq!(empty.agent_history, 0);
    }

    /// **AC4, negative 2 — an artifact whose `parts` are empty.**
    #[test]
    fn an_artifact_with_no_parts_fails_the_render() {
        let mut task = task_with_text("ignored");
        task.status.message = None;
        task.artifacts = Some(vec![mika_a2a::Artifact {
            artifact_id: "art-1".to_string(),
            name: None,
            description: None,
            parts: vec![],
            metadata: None,
            extensions: None,
        }]);

        let empty = render_task_parts(&task).expect_err("a completed turn must not render empty");
        assert_eq!(empty.kind, EmptyKind::SlicesUnreadable);
        assert_eq!(empty.artifacts, 1);
        assert_eq!(empty.artifact_parts, 0);
    }

    /// **AC4, negative 3 — an agent message carrying no part at all.**
    ///
    /// Distinct from negative 2 on purpose: the slice the mika-spirit path
    /// actually populates is `status.message`, so a renderer fixed only for
    /// artifacts would still be blind here.
    #[test]
    fn an_agent_message_with_no_textual_part_fails_the_render() {
        let mut task = task_with_text("ignored");
        task.status.message.as_mut().unwrap().parts = vec![];

        let empty = render_task_parts(&task).expect_err("a completed turn must not render empty");
        assert_eq!(empty.kind, EmptyKind::SlicesUnreadable);
        assert!(empty.status_message);
        assert_eq!(empty.status_message_parts, 0);
    }

    /// **AC5 — neither output format can answer an empty body at exit 0.**
    ///
    /// `render` is the single gate both formats pass through, so this asserts the
    /// property for `--format json` and `--format text` at once, and would fail
    /// the day one of them grew its own fallback.
    #[test]
    fn both_output_formats_fail_and_name_the_handles() {
        let mut task = task_with_text("ignored");
        task.status.message = None;
        task.context_id = Some("ctx-2270".to_string());

        for format in [OutputFormat::Json, OutputFormat::Text] {
            for verbose in [false, true] {
                let err = render(&task, format, verbose)
                    .expect_err("an empty task must never render at exit 0");
                let chain = format!("{err:#}");
                assert!(chain.contains("task-test"), "no task id: {chain}");
                assert!(chain.contains("ctx-2270"), "no context id: {chain}");
                assert!(chain.contains("status.message"), "no slice census: {chain}");
            }
        }
    }

    #[test]
    fn render_file_part_emits_placeholder_with_name() {
        let mut task = task_with_text("");
        task.status.message.as_mut().unwrap().parts = vec![Part::File {
            file: mika_a2a::FileContent {
                name: Some("foo.txt".to_string()),
                mime_type: None,
                bytes: None,
                url: None,
            },
            metadata: None,
        }];
        assert_eq!(render_task_parts(&task).unwrap(), "[file: foo.txt]");
    }

    #[test]
    fn render_file_part_without_name_emits_unnamed_placeholder() {
        let mut task = task_with_text("");
        task.status.message.as_mut().unwrap().parts = vec![Part::File {
            file: mika_a2a::FileContent {
                name: None,
                mime_type: None,
                bytes: None,
                url: None,
            },
            metadata: None,
        }];
        assert_eq!(render_task_parts(&task).unwrap(), "[file: unnamed]");
    }

    #[test]
    fn render_data_part_emits_placeholder() {
        let mut task = task_with_text("");
        task.status.message.as_mut().unwrap().parts = vec![Part::Data {
            data: serde_json::json!({"k": "v"}),
            metadata: None,
        }];
        assert_eq!(render_task_parts(&task).unwrap(), "[data]");
    }

    #[test]
    fn render_mixed_parts_joins_with_blank_lines() {
        let mut task = task_with_text("");
        task.status.message.as_mut().unwrap().parts = vec![
            Part::Text {
                text: "see file:".to_string(),
                metadata: None,
            },
            Part::File {
                file: mika_a2a::FileContent {
                    name: Some("a.txt".to_string()),
                    mime_type: None,
                    bytes: None,
                    url: None,
                },
                metadata: None,
            },
        ];
        // Parts are joined with blank-line separators to preserve agent paragraph
        // boundaries; mirrors a2a_call's render contract.
        assert_eq!(
            render_task_parts(&task).unwrap(),
            "see file:\n\n[file: a.txt]"
        );
    }

    #[tokio::test]
    async fn invalid_url_fails_fast_with_clear_error() {
        let err = dispatch_remote("hi", "not-a-url", OutputFormat::Text, false, None, false)
            .await
            .expect_err("should fail on invalid URL");
        let chain = format!("{err:#}");
        assert!(
            chain.contains("invalid --remote URL"),
            "unexpected error: {chain}"
        );
    }

    #[test]
    fn render_text_without_verbose_emits_rendered_only() {
        let task = task_with_text("ok");
        let s = render(&task, OutputFormat::Text, false).unwrap();
        assert_eq!(s, "ok");
    }

    #[test]
    fn render_text_with_verbose_appends_task_id_trailer() {
        let task = task_with_text("ok");
        let s = render(&task, OutputFormat::Text, true).unwrap();
        // mika#2304: the verbose trailer always carries a model line; with no
        // server attestation it says so rather than printing a local value.
        // mika#1951 adds the isolation line under the same rule — a bare `Task`
        // attests neither, so both lines state the absence.
        assert_eq!(
            s,
            format!(
                "ok\n\nremote_task_id: task-test\n{NO_ATTESTATION_LINE}\n\
                 {NO_ISOLATION_ATTESTATION_LINE}"
            )
        );
    }

    #[test]
    fn render_json_without_verbose_omits_metadata() {
        let task = task_with_text("hi");
        let s = render(&task, OutputFormat::Json, false).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["content"], "hi");
        assert!(v.get("metadata").is_none());
    }

    #[test]
    fn render_json_with_verbose_includes_remote_task_id_metadata() {
        let task = task_with_text("hi");
        let s = render(&task, OutputFormat::Json, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["content"], "hi");
        assert_eq!(v["metadata"]["remote_task_id"], "task-test");
    }

    // --- mika#2036: the error tells the truth about itself ---------------------

    const URL: &str = "http://127.0.0.1:8080/a2a/mika-arch/révision-de-plan";
    const CTX: &str = "ctx-révision-2026-09-04";

    fn every_outcome() -> Vec<(&'static str, Option<Recovery>)> {
        vec![
            ("not sent", None),
            ("no task", Some(Recovery::NoTask)),
            (
                "still running",
                Some(Recovery::StillRunning {
                    task_id: "tâche-1".to_string(),
                    state: TaskState::Working,
                }),
            ),
            (
                "ended",
                Some(Recovery::Ended {
                    task_id: "tâche-1".to_string(),
                    state: TaskState::Failed,
                }),
            ),
            (
                "unavailable",
                Some(Recovery::Unavailable("connexion refusée".to_string())),
            ),
        ]
    }

    /// **AC1 + the 2026-09-04 instance.** Every outcome must render its own
    /// sentence. The founding defect was not a missing message but a *shared*
    /// one: a caller could not tell "busy, retry" from "your answer exists and
    /// was lost", and only the second justifies an escalation. Comparing every
    /// pair fails the moment two of them collapse again.
    #[test]
    fn each_outcome_reads_differently_from_every_other() {
        let outcomes = every_outcome();
        let rendered: Vec<(&str, String)> = outcomes
            .iter()
            .map(|(label, rec)| {
                (
                    *label,
                    transport_error_message(
                        TransportFailure::Interrupted,
                        URL,
                        Duration::from_secs(300),
                        CTX,
                        rec.as_ref(),
                    ),
                )
            })
            .collect();

        for (i, (label_a, a)) in rendered.iter().enumerate() {
            for (label_b, b) in rendered.iter().skip(i + 1) {
                assert_ne!(a, b, "'{label_a}' and '{label_b}' render the same sentence");
            }
        }
    }

    /// **AC4.** Whatever happened, the caller must be told where to look. The
    /// context id is the only handle it holds — the task id is server-minted and
    /// travels back in the envelope that was lost.
    #[test]
    fn every_recovered_outcome_names_the_handle_to_look_it_up_with() {
        for (label, rec) in every_outcome() {
            let text = transport_error_message(
                TransportFailure::Interrupted,
                URL,
                Duration::from_secs(300),
                CTX,
                rec.as_ref(),
            );
            assert!(
                text.contains(URL),
                "'{label}' does not name the endpoint: {text}"
            );
            if rec.is_some() {
                assert!(
                    text.contains(CTX),
                    "'{label}' does not name the context to look it up with: {text}"
                );
            }
        }
    }

    /// The message must carry the *reason* on top of the outcome: a timeout and
    /// an interrupted exchange lead to the same "still running" verdict but are
    /// not the same event, and the timeout must name the budget it spent.
    #[test]
    fn the_reason_survives_alongside_the_outcome() {
        let outcome = Recovery::StillRunning {
            task_id: "tâche-1".to_string(),
            state: TaskState::Working,
        };
        let timed_out = transport_error_message(
            TransportFailure::TimedOut,
            URL,
            Duration::from_secs(300),
            CTX,
            Some(&outcome),
        );
        let interrupted = transport_error_message(
            TransportFailure::Interrupted,
            URL,
            Duration::from_secs(300),
            CTX,
            Some(&outcome),
        );

        assert!(timed_out.contains("300s"), "budget missing: {timed_out}");
        assert_ne!(
            timed_out, interrupted,
            "the same outcome after different failures must not read identically"
        );
    }

    /// A caller that never reached the server must be told exactly that, and
    /// must not be pointed at a lookup that cannot succeed.
    #[test]
    fn an_unreachable_server_does_not_send_the_caller_hunting() {
        let text = transport_error_message(
            TransportFailure::Unreachable,
            URL,
            Duration::from_secs(300),
            CTX,
            None,
        );
        assert!(text.contains("never left this client"), "got: {text}");
        assert!(
            !text.contains("tasks/get"),
            "an unreachable server holds nothing to look up: {text}"
        );
    }

    // --- mika#2070: caller session id on the wire ------------------------------

    #[test]
    fn send_params_carry_the_caller_session_id() {
        let params = build_send_params("hello", Some("rt005-c1-r7"), "ctx-1", &[], None, false);
        let metadata = params.metadata.expect("metadata should be present");
        assert_eq!(metadata.len(), 1);
        assert_eq!(
            metadata.get(CALLER_SESSION_ID_KEY).and_then(|v| v.as_str()),
            Some("rt005-c1-r7")
        );
    }

    #[test]
    fn send_params_without_a_session_serialize_without_metadata() {
        let params = build_send_params("hello", None, "ctx-1", &[], None, false);
        assert!(params.metadata.is_none());
        // The pre-mika#2070 body shape is preserved byte-for-byte: `metadata` is
        // `skip_serializing_if = "Option::is_none"`, so the key must be absent
        // rather than null. Servers that never read it see no change at all.
        let body = serde_json::to_value(&params).unwrap();
        assert!(
            body.get("metadata").is_none(),
            "unexpected metadata key in {body}"
        );
    }

    // --- mika#2363: the skill restriction on the wire --------------------------

    #[test]
    fn send_params_carry_only_skills_as_an_array_of_strings() {
        let only = vec!["mika-arch-groom-ticket".to_string()];
        let params = build_send_params("hello", None, "ctx-1", &only, None, false);
        let body = serde_json::to_value(&params).unwrap();
        assert_eq!(
            body["metadata"][ONLY_SKILLS_KEY],
            serde_json::json!(["mika-arch-groom-ticket"]),
            "the server reads an array of strings; any other shape degrades to \
             no restriction and the flag would be a silent no-op: {body}"
        );
    }

    #[test]
    fn the_two_metadata_keys_are_independent() {
        // Both halves present is the `_arch_ask` shape: a session to continue and
        // a pass to declare. Neither key may shadow the other.
        let only = vec!["mika-arch-second-review".to_string()];
        let params = build_send_params("hello", Some("sess-7"), "ctx-1", &only, None, false);
        let metadata = params.metadata.expect("metadata should be present");
        assert_eq!(metadata.len(), 2);
        assert_eq!(
            metadata.get(CALLER_SESSION_ID_KEY).and_then(|v| v.as_str()),
            Some("sess-7")
        );
        assert!(metadata.contains_key(ONLY_SKILLS_KEY));
    }

    // --- mika#2278: the class is a variant, and it reaches the exit code ------

    /// **AC2, the whole point of the ticket.** The classification is decided by
    /// the error *variant*. Every assertion below compares `FailureClass`
    /// values — never a substring of a message — because a test that matched on
    /// the text would re-introduce exactly the coupling D1 refuses, and would
    /// then certify it.
    #[tokio::test]
    async fn the_a2a_variants_split_transport_from_contract() {
        // A transport failure: the only shape `reqwest` produces that we can
        // build without a network is a URL that cannot be parsed into a
        // request, which `classify` reads as `Unreachable` — the restart case.
        let client_error = reqwest::Client::new()
            .get("http://")
            .build()
            .expect_err("an authority-less URL cannot be built into a request");
        assert_eq!(
            a2a_error_class(&A2aError::ClientError(client_error)),
            FailureClass::Transport,
            "a failed exchange is the retryable family — a restarting server is \
             its canonical member"
        );

        // The negative controls. Without these, a classifier that answered
        // `Transport` unconditionally would pass the assertion above.
        assert_eq!(
            a2a_error_class(&A2aError::InvalidJsonRpc("bad envelope".into())),
            FailureClass::Contract
        );
        let serde_error = serde_json::from_str::<serde_json::Value>("{")
            .expect_err("truncated JSON must not parse");
        assert_eq!(
            a2a_error_class(&A2aError::SerializationError(serde_error)),
            FailureClass::Contract
        );
        assert_eq!(
            a2a_error_class(&A2aError::InvalidStateTransition {
                from: TaskState::Completed,
                to: TaskState::Working,
            }),
            FailureClass::Contract
        );
    }

    /// The state machine's half of the same split (plan step 3).
    ///
    /// Every case passes `None` — no attestation — so this test doubles as
    /// mika#2522's **non-regression control**: a server predating that key
    /// produces exactly the classes it produced before.
    #[test]
    fn only_the_async_dispatch_states_are_retryable() {
        for state in [TaskState::Submitted, TaskState::Working, TaskState::Unknown] {
            assert_eq!(
                terminal_state_class(state, None),
                FailureClass::Transport,
                "{state} is a state a synchronous send should never return"
            );
        }
        for state in [
            TaskState::Failed,
            TaskState::Canceled,
            TaskState::Rejected,
            TaskState::Completed,
            TaskState::InputRequired,
            TaskState::AuthRequired,
        ] {
            assert_eq!(
                terminal_state_class(state, None),
                FailureClass::Contract,
                "{state} is a verdict the server reached, not an accident on the way"
            );
        }
    }

    /// **mika#2522 V2.** The R-2 table, line by line.
    ///
    /// Only a positively attested transport class flips `failed` to
    /// `Transport`; every other reading — including the unreadable ones the
    /// decoder turns into `None` — stays `Contract`.
    #[test]
    fn mika2522_a_failed_turn_is_retryable_only_on_an_attested_transport_class() {
        for class in ["transport", "transport_timeout"] {
            assert_eq!(
                terminal_state_class(TaskState::Failed, Some(class)),
                FailureClass::Transport,
                "a turn killed by {class} was not refused — re-sending the same \
                 brief is exactly what `is_retryable` says to do"
            );
        }

        // The negative controls. Without them, a reader that answered
        // `Transport` on any non-`None` class would pass the loop above.
        for class in [
            "parse",
            "provider",
            "http_400",
            "http_429",
            "unsupported",
            "other",
            // Neither a substring nor a prefix of a transport class counts: the
            // comparison is equality on the shared constants, not a `contains`.
            "transporter",
            "TRANSPORT",
            "",
        ] {
            assert_eq!(
                terminal_state_class(TaskState::Failed, Some(class)),
                FailureClass::Contract,
                "{class:?} is not a transport class and must not arm a retry"
            );
        }
    }

    /// **mika#2522 V2.** `Canceled` and `Rejected` do not consult the
    /// attestation, and a test must say so.
    ///
    /// Without this, a later editor wiring those two arms to the class would see
    /// nothing go red — and a cancellation replayed is a cancellation ignored.
    #[test]
    fn mika2522_a_cancellation_is_a_decision_whatever_the_attestation_says() {
        for state in [TaskState::Canceled, TaskState::Rejected] {
            assert_eq!(
                terminal_state_class(state, Some("transport")),
                FailureClass::Contract,
                "{state} is a decision, not an accident on the way"
            );
        }
    }

    /// **mika#2522.** The two transport spellings come from `mika-common`, and
    /// the predicate agrees with them.
    ///
    /// Pins the import rather than a pair of local literals: a copy here would
    /// split the population that `llm_call_attempt` and
    /// `audit_events.callback_delivery_failed` already count together.
    #[test]
    fn mika2522_the_transport_spellings_are_the_shared_wire_format() {
        use mika_common::llm::error::error_class;
        assert!(is_transport_class(error_class::TRANSPORT));
        assert!(is_transport_class(error_class::TRANSPORT_TIMEOUT));
        assert!(!is_transport_class(error_class::PARSE));
        assert!(!is_transport_class(error_class::PROVIDER));
        assert!(!is_transport_class(error_class::OTHER));
        assert!(!is_transport_class(&error_class::http(429)));
    }

    /// **AC1.** The class survives the trip to the process exit code, and the
    /// two families land on different codes.
    #[test]
    fn the_class_reaches_the_exit_code() {
        let transport = classified(FailureClass::Transport, "server restarting".into());
        let contract = classified(
            FailureClass::Contract,
            "session belongs to someone else".into(),
        );

        assert!(is_transport_failure(&transport));
        assert!(!is_transport_failure(&contract));
        assert_eq!(exit_code_for(&transport), EXIT_TRANSPORT_FAILURE);
        assert_eq!(exit_code_for(&contract), 1);
        // The retry budget keys on this literal; a silent drift would disarm
        // `_arch_ask_with_retry` without failing anything else.
        assert_eq!(EXIT_TRANSPORT_FAILURE, 75);
    }

    /// **D1, said as an assertion.** Attaching the class must not move a single
    /// byte of what the operator reads.
    #[test]
    fn carrying_the_class_does_not_change_the_message() {
        const TEXT: &str = "unreachable: no request reached http://127.0.0.1:8080/a2a/mika-arch";
        for class in [FailureClass::Transport, FailureClass::Contract] {
            let err = classified(class, TEXT.to_string());
            assert_eq!(format!("{err}"), TEXT, "{class:?} altered the plain form");
            assert_eq!(
                format!("{err:#}"),
                TEXT,
                "{class:?} altered the chained form"
            );
        }
    }

    /// **D4, fail-safe.** An error carrying no class at all — every failure
    /// raised before the send, and anything a future path forgets to classify —
    /// is definitive. The inverse default would turn an unforeseen failure mode
    /// into a silent retry loop.
    #[test]
    fn an_unclassified_failure_is_never_retryable() {
        let plain = anyhow::anyhow!("--session-id value must not be empty");
        assert!(!is_transport_failure(&plain));
        assert_eq!(exit_code_for(&plain), 1);
    }

    /// The class must survive a context layer. `wrap_send_error` re-poses it
    /// deliberately (it flattens the chain for mika#1985), but an ordinary
    /// `.context()` added anywhere on this path must not demote it either.
    #[test]
    fn a_context_layer_does_not_demote_the_class() {
        let wrapped = classified(FailureClass::Transport, "interrupted after send".into())
            .context("mika ask to http://127.0.0.1:8080/a2a/mika-arch failed");
        assert_eq!(exit_code_for(&wrapped), EXIT_TRANSPORT_FAILURE);
    }

    #[test]
    fn an_empty_restriction_leaves_the_key_absent() {
        // R3: a caller that declares nothing must produce the pre-mika#2363 body.
        // An empty array on the wire would be a different statement — and one the
        // server would have to decide the meaning of.
        let params = build_send_params("hello", Some("sess-7"), "ctx-1", &[], None, false);
        let body = serde_json::to_value(&params).unwrap();
        assert!(
            body["metadata"].get(ONLY_SKILLS_KEY).is_none(),
            "unexpected only_skills key in {body}"
        );
    }

    // --- mika#2304: the model override on the wire ----------------------------

    /// **T1.** The key travels, and it travels **raw**. Resolving locally would
    /// produce, on `--remote`, an id resolved against the wrong provider — a
    /// second false green, quieter than the one the ticket measured.
    #[test]
    fn send_params_carry_the_model_override_unresolved() {
        // `sonnet` is an alias and `moonshotai/kimi-k2.5` is vendor-prefixed:
        // between them they cover both transformations the server owns.
        for raw in ["sonnet", "moonshotai/kimi-k2.5"] {
            let params = build_send_params("hello", None, "ctx-1", &[], Some(raw), false);
            let body = serde_json::to_value(&params).unwrap();
            assert_eq!(
                body["metadata"][MODEL_OVERRIDE_KEY],
                serde_json::json!(raw),
                "the server resolves aliases and prefixes against the *executing* \
                 provider (mika#1591); this side must not pre-resolve: {body}"
            );
        }
    }

    /// **T1, second half.** With all three keys present, none shadows another.
    /// Same shape as `the_two_metadata_keys_are_independent` one ticket earlier.
    #[test]
    fn the_three_metadata_keys_are_independent() {
        let only = vec!["mika-arch-second-review".to_string()];
        let params = build_send_params(
            "hello",
            Some("sess-7"),
            "ctx-1",
            &only,
            Some("moonshotai/kimi-k2.5"),
            false,
        );
        let metadata = params.metadata.expect("metadata should be present");
        assert_eq!(metadata.len(), 3);
        assert_eq!(
            metadata.get(CALLER_SESSION_ID_KEY).and_then(|v| v.as_str()),
            Some("sess-7")
        );
        assert!(metadata.contains_key(ONLY_SKILLS_KEY));
        assert_eq!(
            metadata.get(MODEL_OVERRIDE_KEY).and_then(|v| v.as_str()),
            Some("moonshotai/kimi-k2.5")
        );
    }

    /// **T2 / AC5.** A caller declaring none of the three keys produces the
    /// pre-mika#2070 body, byte for byte: `metadata` absent, not `null`.
    #[test]
    fn declaring_no_key_at_all_leaves_metadata_absent() {
        let params = build_send_params("hello", None, "ctx-1", &[], None, false);
        let body = serde_json::to_value(&params).unwrap();
        assert!(
            body.get("metadata").is_none(),
            "unexpected metadata key in {body}"
        );
    }

    /// A model override alone must not drag the other two keys along.
    #[test]
    fn a_model_override_alone_carries_only_its_own_key() {
        let params = build_send_params("hello", None, "ctx-1", &[], Some("sonnet"), false);
        let metadata = params.metadata.expect("metadata should be present");
        assert_eq!(metadata.len(), 1);
        assert!(metadata.contains_key(MODEL_OVERRIDE_KEY));
    }

    // --- mika#2304 T6: the CLI never shows a model it was not given -----------

    fn task_attesting(model: Option<&str>) -> Task {
        let mut task = task_with_text("ok");
        if let Some(m) = model {
            task.metadata = Some(std::collections::HashMap::from([(
                EFFECTIVE_MODEL_KEY.to_string(),
                serde_json::Value::String(m.to_string()),
            )]));
        }
        task
    }

    /// **T6, positive.** An attested model is what both formats report.
    #[test]
    fn verbose_reports_the_attested_model_on_both_formats() {
        let task = task_attesting(Some("openrouter/moonshotai/kimi-k2.5"));

        let text = render(&task, OutputFormat::Text, true).unwrap();
        assert!(
            text.contains("model: openrouter/moonshotai/kimi-k2.5"),
            "text trailer missing the attestation: {text}"
        );

        let json = render(&task, OutputFormat::Json, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["metadata"]["model"], "openrouter/moonshotai/kimi-k2.5");
    }

    /// **T6, the one that matters.** With no attestation the requested model
    /// must appear **nowhere** in the output. A server older than mika#2304
    /// lands here, and printing a local value would be the very lie the
    /// attestation exists to remove — asserted on the whole rendered string, not
    /// just on the field, so a second display path cannot leak it either.
    #[test]
    fn without_an_attestation_no_model_is_shown_anywhere() {
        let task = task_attesting(None);

        let text = render(&task, OutputFormat::Text, true).unwrap();
        assert!(
            !text.contains("kimi"),
            "a model leaked into an unattested render: {text}"
        );
        assert!(
            text.contains(NO_ATTESTATION_LINE),
            "the absence must be stated, not silently omitted: {text}"
        );

        let json = render(&task, OutputFormat::Json, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            v["metadata"].get("model").is_none(),
            "absent, never null and never local: {v}"
        );
    }

    /// Without `--verbose` nothing changes on either format — the mika#2304
    /// trailer is verbose-gated like every other runtime field.
    #[test]
    fn a_non_verbose_render_is_untouched_by_the_attestation() {
        let task = task_attesting(Some("openrouter/moonshotai/kimi-k2.5"));
        assert_eq!(render(&task, OutputFormat::Text, false).unwrap(), "ok");
        let json = render(&task, OutputFormat::Json, false).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("metadata").is_none(), "unexpected metadata in {v}");
    }

    // --- mika#1951 U2: the isolation lever on the wire -------------------------

    /// **U2, the wire shape.** The server reads a JSON **bool**; any other shape
    /// fails the request rather than degrading to "not isolated". A string
    /// `"true"` here would turn the flag into a refusal, and a number into a
    /// refusal too — both of which are better than a silent no-op, but neither
    /// is the flag working.
    #[test]
    fn mika1951_send_params_carry_the_isolation_flag_as_a_json_bool() {
        let params = build_send_params("hello", None, "ctx-1", &[], None, true);
        let body = serde_json::to_value(&params).unwrap();
        assert_eq!(
            body["metadata"][SESSION_ISOLATED_KEY],
            serde_json::Value::Bool(true),
            "the server reads a bool; any other shape fails the request: {body}"
        );
    }

    /// **U2, the absent half.** `--isolated` unset leaves the key **absent**, not
    /// `false`. Both mean the same thing to the server, but only absence keeps a
    /// non-declaring caller byte-identical to one that predates the key — the
    /// property the three sister keys already have.
    #[test]
    fn mika1951_an_unisolated_call_leaves_the_key_absent_entirely() {
        let params = build_send_params("hello", None, "ctx-1", &[], None, false);
        let body = serde_json::to_value(&params).unwrap();
        assert!(
            body.get("metadata").is_none(),
            "an unisolated call must not post a `false`: {body}"
        );

        // And it must not appear beside a sister key either — a `false` riding
        // along on an otherwise-populated envelope would be just as much a
        // change of shape, only harder to see.
        let params = build_send_params("hello", Some("sess-7"), "ctx-1", &[], None, false);
        let metadata = params.metadata.expect("metadata should be present");
        assert!(
            !metadata.contains_key(SESSION_ISOLATED_KEY),
            "unexpected isolation key in {metadata:?}"
        );
    }

    /// **U2, independence.** With all four keys present none shadows another.
    /// The direct continuation of `the_three_metadata_keys_are_independent`.
    #[test]
    fn mika1951_the_four_metadata_keys_are_independent() {
        let only = vec!["mika-arch-second-review".to_string()];
        let params = build_send_params(
            "hello",
            Some("sess-7"),
            "ctx-1",
            &only,
            Some("moonshotai/kimi-k2.5"),
            true,
        );
        let metadata = params.metadata.expect("metadata should be present");
        assert_eq!(metadata.len(), 4);
        assert_eq!(
            metadata.get(CALLER_SESSION_ID_KEY).and_then(|v| v.as_str()),
            Some("sess-7")
        );
        assert!(metadata.contains_key(ONLY_SKILLS_KEY));
        assert_eq!(
            metadata.get(MODEL_OVERRIDE_KEY).and_then(|v| v.as_str()),
            Some("moonshotai/kimi-k2.5")
        );
        assert_eq!(
            metadata.get(SESSION_ISOLATED_KEY).and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    /// An isolation request alone must not drag the other three keys along.
    #[test]
    fn mika1951_the_isolation_flag_alone_carries_only_its_own_key() {
        let params = build_send_params("hello", None, "ctx-1", &[], None, true);
        let metadata = params.metadata.expect("metadata should be present");
        assert_eq!(metadata.len(), 1);
        assert!(metadata.contains_key(SESSION_ISOLATED_KEY));
    }

    /// **Both doors post the key, and it is asserted rather than assumed.**
    ///
    /// The plan says so in as many words, because mika#2304 had to be repaired
    /// once for exactly this: a fix aimed at `--remote` left the default path —
    /// an A2A client too since mika#1727 — silently dropping the flag. The
    /// property that prevents the recurrence is structural: `build_send_params`
    /// is called from **one** place, `send_message_to_agent`, which both doors
    /// reach. A behavioural test cannot see a second construction site; it would
    /// stay green while a new door posted no key at all.
    #[test]
    fn mika1951_both_ask_doors_post_the_isolation_key_through_one_site() {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let source = scanner.production_of(&scanner.src_root().join("remote_ask.rs"));

        // One definition, one call. The definition line is `fn build_send_params(`;
        // the single call is inside `send_message_to_agent`.
        let definitions = source.matches("fn build_send_params(").count();
        let calls = source.matches("build_send_params(").count() - definitions;
        assert_eq!(definitions, 1, "found {definitions} definitions");
        assert_eq!(
            calls, 1,
            "expected exactly one production call site (inside \
             `send_message_to_agent`, which both `mika ask` doors reach), found \
             {calls} — a second one is a door that can drift out of sync, which \
             is the mika#2304 recurrence this guard exists to refuse"
        );

        // And that single site is inside `send_message_to_agent`: a call moved
        // into some other helper would keep the count at one while breaking the
        // property the count stands for.
        let sender = source
            .split_once("pub async fn send_message_to_agent(")
            .map(|(_, rest)| rest)
            .expect("send_message_to_agent must exist in production source");
        assert!(
            sender
                .split_once("\npub ")
                .map_or(sender, |(body, _)| body)
                .contains("build_send_params("),
            "the one call site must live inside `send_message_to_agent`"
        );
    }

    // --- mika#1951 U3: the CLI never claims an isolation it was not told ------

    fn task_attesting_isolation(isolated: Option<bool>) -> Task {
        let mut task = task_with_text("ok");
        if let Some(v) = isolated {
            task.metadata = Some(std::collections::HashMap::from([(
                SESSION_ISOLATED_APPLIED_KEY.to_string(),
                serde_json::Value::Bool(v),
            )]));
        }
        task
    }

    /// **U3, positive — and both values matter.** `true` is the bench's green;
    /// `false` is the answer a server gives when it understood the question and
    /// did not isolate. Rendering only the first would leave `false` and "not
    /// attested" indistinguishable, which is the conflation this whole unit
    /// exists to remove.
    #[test]
    fn mika1951_verbose_reports_the_attested_isolation_on_both_formats() {
        for attested in [true, false] {
            let task = task_attesting_isolation(Some(attested));

            let text = render(&task, OutputFormat::Text, true).unwrap();
            assert!(
                text.contains(&format!("isolated: {attested}")),
                "text trailer missing the attestation: {text}"
            );

            let json = render(&task, OutputFormat::Json, true).unwrap();
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(
                v["metadata"]["isolated"],
                serde_json::Value::Bool(attested),
                "json envelope missing the attestation: {v}"
            );
        }
    }

    /// **U3, the one that matters.** A server that attested nothing must produce
    /// no isolation claim at all — the population being every mika-spirit older
    /// than mika#1951, where echoing the local flag would assert an isolation
    /// that did not happen and make a contaminated bench read as valid.
    #[test]
    fn mika1951_without_an_attestation_no_isolation_is_claimed() {
        let task = task_attesting_isolation(None);

        let text = render(&task, OutputFormat::Text, true).unwrap();
        assert!(
            text.contains(NO_ISOLATION_ATTESTATION_LINE),
            "the absence must be stated, not silently omitted: {text}"
        );
        assert!(
            !text.contains("isolated: true") && !text.contains("isolated: false"),
            "an isolation verdict leaked into an unattested render: {text}"
        );

        let json = render(&task, OutputFormat::Json, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            v["metadata"].get("isolated").is_none(),
            "absent, never null and never the local flag: {v}"
        );
    }

    /// The rendering reads the Task and nothing else — `render` is not even
    /// handed the flag this process was passed, which is what makes "the CLI
    /// cannot echo its own request" a property of the signature rather than of
    /// a reviewer's attention.
    #[test]
    fn mika1951_a_non_verbose_render_is_untouched_by_the_isolation_attestation() {
        let task = task_attesting_isolation(Some(true));
        assert_eq!(render(&task, OutputFormat::Text, false).unwrap(), "ok");
        let json = render(&task, OutputFormat::Json, false).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("metadata").is_none(), "unexpected metadata in {v}");
    }

    /// The two attestations are read independently: a server that attests a
    /// model and not an isolation must produce exactly one of the two lines and
    /// the honest absence for the other. Neither may be inferred from the other.
    #[test]
    fn mika1951_the_two_attestations_do_not_shadow_each_other() {
        let mut task = task_with_text("ok");
        task.metadata = Some(std::collections::HashMap::from([(
            EFFECTIVE_MODEL_KEY.to_string(),
            serde_json::Value::String("openrouter/moonshotai/kimi-k2.5".to_string()),
        )]));

        let text = render(&task, OutputFormat::Text, true).unwrap();
        assert!(
            text.contains("model: openrouter/moonshotai/kimi-k2.5"),
            "the model attestation must survive: {text}"
        );
        assert!(
            text.contains(NO_ISOLATION_ATTESTATION_LINE),
            "a model attestation says nothing about isolation: {text}"
        );
    }

    // --- mika#1883: the per-turn token total ----------------------------------

    fn task_attesting_run_usage(usage: Option<serde_json::Value>) -> Task {
        let mut task = task_with_text("ok");
        if let Some(u) = usage {
            task.metadata = Some(std::collections::HashMap::from([(
                RUN_USAGE_KEY.to_string(),
                u,
            )]));
        }
        task
    }

    /// **AC1 / AC3, positive** — an attested total is reported on both formats,
    /// and the cache fields ride along when the provider reported them.
    #[test]
    fn mika1883_verbose_reports_the_attested_run_usage_on_both_formats() {
        let task = task_attesting_run_usage(Some(serde_json::json!({
            "input": 41_000, "output": 900, "cache_read": 38_000, "cache_write": 12,
        })));

        let text = render(&task, OutputFormat::Text, true).unwrap();
        for expected in [
            "tokens.input: 41000",
            "tokens.output: 900",
            "tokens.cache_read: 38000",
            "tokens.cache_write: 12",
        ] {
            assert!(text.contains(expected), "missing {expected} in {text}");
        }

        let json = render(&task, OutputFormat::Json, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["metadata"]["tokens"]["input"], 41_000);
        assert_eq!(v["metadata"]["tokens"]["output"], 900);
        assert_eq!(v["metadata"]["tokens"]["cache_read"], 38_000);
        assert_eq!(v["metadata"]["tokens"]["cache_write"], 12);
    }

    /// **AC2, and it is the assertion that carries the ticket's rule** — with no
    /// attestation, **no `tokens.` string appears anywhere** and the JSON key is
    /// absent.
    ///
    /// Asserted on the whole rendered output rather than on the field, so a
    /// second display path cannot leak a zero either. And deliberately **no**
    /// "(not attested)" line, unlike `model:` and `isolated:` one test up: those
    /// answer a flag the caller passed, where silence is ambiguous; this is a
    /// measurement nobody requested, where a line announcing an absent number is
    /// noise on every turn a pre-mika#1883 server serves.
    #[test]
    fn mika1883_without_an_attestation_no_token_count_is_shown_anywhere() {
        let task = task_attesting_run_usage(None);

        let text = render(&task, OutputFormat::Text, true).unwrap();
        assert!(
            !text.contains("tokens."),
            "an unattested render must show no token line at all — a zero is \
             indistinguishable from a real turn: {text}"
        );

        let json = render(&task, OutputFormat::Json, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            v["metadata"].get("tokens").is_none(),
            "absent, never null and never zeroed: {v}"
        );
    }

    /// **AC2, the partial shape** — a provider that reported no cache still
    /// attests its two totals, and the two absent fields stay absent rather than
    /// rendering as zeros.
    #[test]
    fn mika1883_absent_cache_fields_render_as_absent_not_zero() {
        let task = task_attesting_run_usage(Some(serde_json::json!({
            "input": 10, "output": 2,
        })));

        let text = render(&task, OutputFormat::Text, true).unwrap();
        assert!(text.contains("tokens.input: 10"));
        assert!(text.contains("tokens.output: 2"));
        assert!(
            !text.contains("tokens.cache_read") && !text.contains("tokens.cache_write"),
            "a cache field the provider never reported must not be printed as 0: {text}"
        );

        let json = render(&task, OutputFormat::Json, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["metadata"]["tokens"].get("cache_read").is_none());
        assert!(v["metadata"]["tokens"].get("cache_write").is_none());
    }

    /// **AC4** — without `--verbose` both formats are byte-identical to what
    /// they were before this field existed.
    #[test]
    fn mika1883_a_non_verbose_render_is_byte_identical() {
        let task = task_attesting_run_usage(Some(serde_json::json!({
            "input": 41_000, "output": 900,
        })));
        assert_eq!(render(&task, OutputFormat::Text, false).unwrap(), "ok");
        let json = render(&task, OutputFormat::Json, false).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("metadata").is_none(), "unexpected metadata in {v}");
        assert_eq!(v["content"], "ok");
    }

    /// **AC3** — both client surfaces decode the key through the one
    /// `mika-a2a` reader, and neither re-implements it.
    ///
    /// A second decoder would make no rendering *wrong* the day it is written;
    /// it would let `mika ask --verbose` and `mika ask --remote --verbose`
    /// answer the same question differently — the class `attested_model`'s doc
    /// comment was written to prevent, and the one
    /// `mika2220_no_local_reparse_of_the_llm_bodies_env_var` had to engrave once
    /// on the CLI/daemon split.
    ///
    /// Two halves: the wire spelling appears nowhere in this crate (a literal is
    /// how a second decoder starts), and both surfaces call the reader by name.
    #[test]
    fn mika1883_both_client_surfaces_read_the_one_reader() {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let src_root = scanner.src_root().to_path_buf();

        // The needle is the spelling **inside a string literal**, not the name:
        // the same discrimination mika#2220's guard makes, so a doc comment that
        // names the key in prose stays legal while a second decoder — which must
        // write the key as a literal to index the metadata map — does not. It is
        // assembled at runtime for that guard's other reason: a literal here
        // would match this file and fail forever.
        let wire_spelling = format!("\"mika.{}\"", "run_usage");
        let mut offenders: Vec<String> = Vec::new();
        let mut surfaces_reading: Vec<String> = Vec::new();

        scanner.for_each(|path, production| {
            let rel = path.strip_prefix(&src_root).unwrap_or(path).display();
            for (i, line) in production.lines().enumerate() {
                if line.contains(&wire_spelling) {
                    offenders.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                }
            }
            if production.contains("attested_run_usage(") {
                surfaces_reading.push(rel.to_string());
            }
        });

        assert!(
            offenders.is_empty(),
            "mika#1883 — the wire spelling is written here instead of being read \
             through `mika_a2a::params::attested_run_usage`. Two decoders is how \
             the two `mika ask` doors come to disagree:\n{}",
            offenders.join("\n")
        );

        // `commands/ask.rs` carries its directory on purpose: `remote_ask.rs`
        // also ends in `ask.rs`, so the bare suffix would let one surface
        // satisfy the assertion for both — the exact shape of a check that
        // passes while half the property is false.
        for surface in ["remote_ask.rs", "commands/ask.rs"] {
            assert!(
                surfaces_reading.iter().any(|s| s.ends_with(surface)),
                "{surface} must read the attestation through the shared reader; \
                 found only: {surfaces_reading:?}"
            );
        }
    }

    /// The paths this crate's tree sits under, resolved from its own manifest.
    ///
    /// `crates/mika-cli/..` rather than a repo-root walk: the two crates D2
    /// covers are named, and a wider sweep would be a guard whose scope nobody
    /// can state.
    fn mika2522_scanned_src_roots() -> Vec<std::path::PathBuf> {
        let crates_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/mika-cli has a parent")
            .to_path_buf();
        vec![
            crates_dir.join("mika-cli/src"),
            crates_dir.join("mika-a2a/src"),
        ]
    }

    /// **mika#2522 D2.** The failure-class attestation has exactly one reader.
    ///
    /// Outside `params.rs`, which defines it, the wire spelling must appear in
    /// no string literal across `mika-cli` and `mika-a2a`: a second decoder has
    /// to write the key as a literal to index the metadata map, and two decoders
    /// of one fact are two truths in waiting — the class
    /// `mika1883_both_client_surfaces_read_the_one_reader` above already holds
    /// for its own key.
    ///
    /// **Why a source scan and not a behavioural test.** A second reader makes
    /// **no decision wrong** the day it is written: the client still classes
    /// correctly, every assertion here stays green, and only the one-truth
    /// guarantee goes — in silence. That is the failure shape a behavioural test
    /// cannot see, and the only reason to write a scan.
    #[test]
    fn mika2522_the_attestation_has_a_single_reader() {
        // Assembled at runtime so this file does not accuse itself.
        let wire_spelling = format!("\"mika.{}\"", "turn_failure_class");
        let definition_site = "params.rs";

        let mut offenders: Vec<String> = Vec::new();
        let mut definition_carries_it = false;

        for root in mika2522_scanned_src_roots() {
            let scanner = mika_common::source_guard::ProductionScanner::new(&root);
            scanner.for_each(|path, production| {
                let is_definition = path
                    .file_name()
                    .is_some_and(|f| f == std::ffi::OsStr::new(definition_site));
                for (i, line) in production.lines().enumerate() {
                    if !line.contains(&wire_spelling) {
                        continue;
                    }
                    if is_definition {
                        definition_carries_it = true;
                    } else {
                        let rel = path.strip_prefix(&root).unwrap_or(path).display();
                        offenders.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                    }
                }
            });
        }

        // Anti-vacuity (the mika#2496 lesson): a scan aimed at a name nobody
        // writes verifies nothing and reads exactly like a clean scan.
        assert!(
            definition_carries_it,
            "mika#2522 — the wire spelling is written nowhere in {definition_site}: \
             this scan is aimed at a dead name and attests nothing"
        );

        assert!(
            offenders.is_empty(),
            "mika#2522 — the wire spelling is written here instead of being read \
             through `mika_a2a::params::attested_turn_failure_class`. Two decoders \
             is how a client comes to disagree with itself about whether a failed \
             turn may be retried:\n{}\n\n\
             RESOLUTION: remove the second reader. There is no allowlist to add \
             it to (mika#2201).",
            offenders.join("\n")
        );
    }
}
