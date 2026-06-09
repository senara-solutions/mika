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

use anyhow::{Context, Result};
use mika_a2a::client::A2aClient;
use mika_a2a::{A2aError, Message, MessageSendParams, Part, Role, Task, TaskState};
use uuid::Uuid;

/// Output format selector. Mirrors `crate::cli::OutputFormat` to keep the
/// remote-mode dispatch decoupled from the binary's clap layer (so integration
/// tests in `tests/` can call this directly without pulling in the full CLI).
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Text,
    Json,
}

/// Render an A2A `Task`'s text content to a flat string using the same three-tier
/// extraction strategy as the in-process `a2a_call` builtin tool
/// (`crates/mika-agent/src/tools/a2a_call.rs`): artifacts → agent-role history →
/// status-message fallback. The A2A spec carries completed-task output in
/// artifacts, so reading only `status.message` would silently render empty for
/// spec-conformant remote agents.
///
/// Non-text parts surface as placeholder strings (`[file: <name>]`, `[data]`);
/// multi-text parts are joined with blank-line separators to preserve paragraph
/// boundaries from the remote agent.
pub fn render_task_parts(task: &Task) -> String {
    let mut parts_text: Vec<String> = Vec::new();

    // Tier 1: artifacts
    if let Some(artifacts) = &task.artifacts {
        for artifact in artifacts {
            for part in &artifact.parts {
                push_rendered_part(part, &mut parts_text);
            }
        }
    }

    // Tier 2: agent-role messages in history
    if let Some(history) = &task.history {
        for msg in history {
            if msg.role == Role::Agent {
                for part in &msg.parts {
                    push_rendered_part(part, &mut parts_text);
                }
            }
        }
    }

    // Tier 3: status.message fallback (only if tiers 1+2 produced nothing)
    if parts_text.is_empty()
        && let Some(msg) = task.status.message.as_ref()
    {
        for part in &msg.parts {
            push_rendered_part(part, &mut parts_text);
        }
    }

    parts_text.join("\n\n")
}

fn push_rendered_part(part: &Part, out: &mut Vec<String>) {
    match part {
        Part::Text { text, .. } => out.push(text.clone()),
        Part::File { file, .. } => {
            let name = file.name.as_deref().unwrap_or("unnamed");
            out.push(format!("[file: {name}]"));
        }
        Part::Data { .. } => out.push("[data]".to_string()),
    }
}

/// Build a `MessageSendParams` containing the user's single text prompt.
fn build_send_params(message: &str) -> MessageSendParams {
    MessageSendParams {
        message: Message {
            message_id: Uuid::new_v4().to_string(),
            role: Role::User,
            parts: vec![Part::Text {
                text: message.to_string(),
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
        metadata: None,
    }
}

/// Dispatch a `mika ask` invocation in remote mode and return the rendered output.
///
/// Validates `remote_url`, sends the prompt via `A2aClient::send_message`, and
/// renders the returned `Task` to a string in the requested format. Returns the
/// rendered string rather than printing it so integration tests can assert on the
/// output without capturing stdout. `run_remote` wraps this with a print.
///
/// Errors are surfaced as `anyhow` failures with single-line prefixes per
/// `A2aError` variant so the caller can `eprintln!("Error: {e}")` and exit non-zero.
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

    // Filter out an empty MIKA_INTERNAL_TOKEN — set-but-empty (common with a
    // misconfigured .env) would otherwise forward `Authorization: Bearer `, which
    // the gateway 401s with no diagnostic hint. Treating empty as unset surfaces
    // the same 401 but at least matches the no-auth diagnostic shape.
    // Note on secrets discipline: `mika/CLAUDE.md` mandates `secrecy::SecretString`
    // at the `Settings` accessor boundary, with downstream types using plain `String`.
    // `A2aClient::new` takes `Option<String>` — its API IS the downstream boundary,
    // so a plain-String read here is consistent with the convention. `A2aClient`
    // does not derive `Debug`, so accidental log leakage is constrained.
    let auth_token = std::env::var("MIKA_INTERNAL_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    let client = A2aClient::new(remote_url, auth_token);

    let task = match client.send_message(build_send_params(message)).await {
        Ok(task) => task,
        Err(A2aError::InvalidJsonRpc(msg)) => anyhow::bail!("remote error: {msg}"),
        Err(A2aError::ClientError(e)) => anyhow::bail!("connection error: {e}"),
        Err(A2aError::SerializationError(e)) => anyhow::bail!("serialization error: {e}"),
        Err(A2aError::InvalidStateTransition { from, to }) => {
            anyhow::bail!("invalid state transition from {from} to {to}")
        }
    };

    // Inspect the terminal Task state. Per A2A v0.3 §6, a synchronous
    // `message/send` returns a Task in one of: Completed, Failed, Canceled,
    // Rejected, InputRequired, AuthRequired (terminal-or-pending). Submitted /
    // Working are async-dispatch states the gateway should not return here.
    //
    // Render output for Completed and the pending-input states (the latter
    // carry meaningful text content the user needs to see). Surface terminal-bad
    // states and async-in-progress states as errors so the shell exit code
    // matches the local `mika ask` contract.
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

fn render(task: &Task, format: OutputFormat, verbose: bool) -> Result<String> {
    let rendered = render_task_parts(task);
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
        assert_eq!(render_task_parts(&task), "hello world");
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
        assert_eq!(render_task_parts(&task), "artifact-text");
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
        assert_eq!(render_task_parts(&task), "history-text");
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
        assert_eq!(render_task_parts(&task), "status-fallback-text");
    }

    #[test]
    fn render_empty_message_emits_empty_string() {
        let mut task = task_with_text("ignored");
        task.status.message = None;
        assert_eq!(render_task_parts(&task), "");
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
        assert_eq!(render_task_parts(&task), "[file: foo.txt]");
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
        assert_eq!(render_task_parts(&task), "[file: unnamed]");
    }

    #[test]
    fn render_data_part_emits_placeholder() {
        let mut task = task_with_text("");
        task.status.message.as_mut().unwrap().parts = vec![Part::Data {
            data: serde_json::json!({"k": "v"}),
            metadata: None,
        }];
        assert_eq!(render_task_parts(&task), "[data]");
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
        assert_eq!(render_task_parts(&task), "see file:\n\n[file: a.txt]");
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
}
