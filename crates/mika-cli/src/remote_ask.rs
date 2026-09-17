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
pub use mika_a2a::{CALLER_SESSION_ID_KEY, ONLY_SKILLS_KEY};
use uuid::Uuid;

/// Output format selector. Mirrors `crate::cli::OutputFormat` to keep the
/// remote-mode dispatch decoupled from the binary's clap layer (so integration
/// tests in `tests/` can call this directly without pulling in the full CLI).
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Text,
    Json,
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
/// The two metadata keys are independent and either may be absent. When both
/// are, `metadata` itself stays absent so the serialized body is byte-identical
/// to the pre-mika#2070 shape — the property that makes a caller declaring
/// nothing indistinguishable from a caller that predates these keys.
fn build_send_params(
    message: &str,
    caller_session_id: Option<&str>,
    context_id: &str,
    only_skills: &[String],
) -> MessageSendParams {
    let mut fields = std::collections::HashMap::new();
    if let Some(sid) = caller_session_id {
        fields.insert(
            CALLER_SESSION_ID_KEY.to_string(),
            serde_json::Value::String(sid.to_string()),
        );
    }
    if !only_skills.is_empty() {
        fields.insert(
            ONLY_SKILLS_KEY.to_string(),
            serde_json::Value::Array(
                only_skills
                    .iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect(),
            ),
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
        ))
        .await
    {
        Ok(task) => task,
        Err(A2aError::InvalidJsonRpc(msg)) => anyhow::bail!("remote error: {msg}"),
        Err(A2aError::ClientError(e)) => {
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
                other => anyhow::bail!(transport_error_message(
                    failure,
                    url,
                    client.timeout(),
                    &context_id,
                    other.as_ref(),
                )),
            }
        }
        Err(A2aError::SerializationError(e)) => anyhow::bail!("serialization error: {e}"),
        Err(A2aError::InvalidStateTransition { from, to }) => {
            anyhow::bail!("invalid state transition from {from} to {to}")
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
    match task.status.state {
        TaskState::Completed | TaskState::InputRequired | TaskState::AuthRequired => {}
        TaskState::Failed | TaskState::Canceled | TaskState::Rejected => {
            anyhow::bail!(
                "remote task {} ended in state '{}'",
                task.id,
                task.status.state
            );
        }
        TaskState::Submitted | TaskState::Working | TaskState::Unknown => {
            anyhow::bail!(
                "remote task {} is still in state '{}' — sync dispatch expected a terminal state",
                task.id,
                task.status.state
            );
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
) -> Result<String> {
    // Fail-fast URL validation. A2aClient itself doesn't pre-parse, so an invalid
    // URL would surface as a reqwest send error — a less actionable message.
    reqwest::Url::parse(remote_url)
        .with_context(|| format!("invalid --remote URL: {remote_url}"))?;

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
    let task = send_message_to_agent(message, remote_url, None, &[]).await?;
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
) -> Result<()> {
    let output = dispatch_remote(message, remote_url, format, verbose).await?;
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
fn render(task: &Task, format: OutputFormat, verbose: bool) -> Result<String> {
    let rendered = render_task_parts(task)?;
    Ok(match format {
        OutputFormat::Text => {
            if verbose {
                format!("{rendered}\n\nremote_task_id: {}", task.id)
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
                response["metadata"] = serde_json::json!({
                    "remote_task_id": task.id,
                });
            }
            serde_json::to_string(&response)?
        }
    })
}

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
        let err = dispatch_remote("hi", "not-a-url", OutputFormat::Text, false)
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
        assert_eq!(s, "ok\n\nremote_task_id: task-test");
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
        let params = build_send_params("hello", Some("rt005-c1-r7"), "ctx-1", &[]);
        let metadata = params.metadata.expect("metadata should be present");
        assert_eq!(metadata.len(), 1);
        assert_eq!(
            metadata.get(CALLER_SESSION_ID_KEY).and_then(|v| v.as_str()),
            Some("rt005-c1-r7")
        );
    }

    #[test]
    fn send_params_without_a_session_serialize_without_metadata() {
        let params = build_send_params("hello", None, "ctx-1", &[]);
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
        let params = build_send_params("hello", None, "ctx-1", &only);
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
        let params = build_send_params("hello", Some("sess-7"), "ctx-1", &only);
        let metadata = params.metadata.expect("metadata should be present");
        assert_eq!(metadata.len(), 2);
        assert_eq!(
            metadata.get(CALLER_SESSION_ID_KEY).and_then(|v| v.as_str()),
            Some("sess-7")
        );
        assert!(metadata.contains_key(ONLY_SKILLS_KEY));
    }

    #[test]
    fn an_empty_restriction_leaves_the_key_absent() {
        // R3: a caller that declares nothing must produce the pre-mika#2363 body.
        // An empty array on the wire would be a different statement — and one the
        // server would have to decide the meaning of.
        let params = build_send_params("hello", Some("sess-7"), "ctx-1", &[]);
        let body = serde_json::to_value(&params).unwrap();
        assert!(
            body["metadata"].get(ONLY_SKILLS_KEY).is_none(),
            "unexpected only_skills key in {body}"
        );
    }
}
