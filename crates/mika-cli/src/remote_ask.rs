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
use mika_a2a::{A2aError, Message, MessageSendParams, Part, Role, Task};
use uuid::Uuid;

/// Output format selector. Mirrors `crate::cli::OutputFormat` to keep the
/// remote-mode dispatch decoupled from the binary's clap layer (so integration
/// tests in `tests/` can call this directly without pulling in the full CLI).
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Text,
    Json,
}

/// Render the agent's most-recent message parts on a `Task` to a flat string.
///
/// Text parts are emitted verbatim; non-text parts surface as placeholder strings
/// (`[file: <name>]`, `[data]`) — full multi-modal CLI rendering is deferred.
pub fn render_task_parts(task: &Task) -> String {
    let Some(msg) = task.status.message.as_ref() else {
        return String::new();
    };
    let mut out = String::new();
    for part in &msg.parts {
        match part {
            Part::Text { text, .. } => out.push_str(text),
            Part::File { file, .. } => {
                let name = file.name.as_deref().unwrap_or("unnamed");
                out.push_str(&format!("[file: {name}]"));
            }
            Part::Data { .. } => out.push_str("[data]"),
        }
    }
    out
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

    let auth_token = std::env::var("MIKA_INTERNAL_TOKEN").ok();
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
    fn render_mixed_parts_concatenates() {
        let mut task = task_with_text("");
        task.status.message.as_mut().unwrap().parts = vec![
            Part::Text {
                text: "see file: ".to_string(),
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
        assert_eq!(render_task_parts(&task), "see file: [file: a.txt]");
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
