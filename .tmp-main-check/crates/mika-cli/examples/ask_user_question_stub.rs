//! AskUserQuestion TUI-side stub consumer (mika#1734 AC5).
//!
//! One-file demo that (a) subscribes to the permissions SSE stream,
//! (b) logs any `ask_user_question` frame it receives, and (c) POSTs a
//! canned reply that picks the FIRST option of every question. TUI's
//! actual rendering lands in mika#1727; this stub exists so integrators
//! and tests can wire against a working consumer.
//!
//! Usage:
//! ```bash
//! MIKA_INTERNAL_TOKEN=<token> \
//!   cargo run --example ask_user_question_stub -- \
//!     --spirit-url http://localhost:8080
//! ```
//!
//! The stub also logs `permission_request` frames but does NOT POST a
//! decision — that surface belongs to the actual TUI.

use std::collections::HashMap;
use std::env;

use clap::Parser;
use futures_util::TryStreamExt;
use mika_agent::server::permissions_stream::{PermissionAnswerRequest, PermissionStreamFrame};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::io::StreamReader;

#[derive(Parser, Debug)]
#[command(about = "AskUserQuestion TUI-side stub consumer (mika#1734)")]
struct Args {
    /// Base URL of the mika-spirit server (defaults to $MIKA_SPIRIT_URL or
    /// `http://localhost:8080`).
    #[arg(long)]
    spirit_url: Option<String>,

    /// Log frames without POSTing answers.
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let spirit_url = args
        .spirit_url
        .or_else(|| env::var("MIKA_SPIRIT_URL").ok())
        .unwrap_or_else(|| "http://localhost:8080".to_string());
    let token = env::var("MIKA_INTERNAL_TOKEN")
        .or_else(|_| env::var("MIKA_DASHBOARD_TOKEN"))
        .map_err(|_| {
            anyhow::anyhow!("MIKA_INTERNAL_TOKEN (or MIKA_DASHBOARD_TOKEN) must be set for auth")
        })?;

    println!("subscribing to {spirit_url}/api/v1/dashboard/permissions/stream");

    let client = reqwest::Client::builder().build()?;
    let resp = client
        .get(format!("{spirit_url}/api/v1/dashboard/permissions/stream"))
        .bearer_auth(&token)
        .send()
        .await?
        .error_for_status()?;

    // Adapt reqwest's byte stream to `AsyncRead` for line-based SSE parsing.
    let stream = resp
        .bytes_stream()
        .map_err(|e| std::io::Error::other(e.to_string()));
    let mut lines = BufReader::new(StreamReader::new(stream)).lines();

    // SSE data lines look like `data: <json>` separated by blank lines.
    while let Some(line) = lines.next_line().await? {
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() {
            continue;
        }
        let frame: PermissionStreamFrame = match serde_json::from_str(payload) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("(stub) unparseable frame: {e}: {payload}");
                continue;
            }
        };
        match frame {
            PermissionStreamFrame::AskUserQuestion {
                request_id,
                questions,
            } => {
                println!("[ask_user_question] request_id={request_id}");
                for (idx, q) in questions.iter().enumerate() {
                    let opts = q
                        .options
                        .iter()
                        .map(|o| o.label.as_str())
                        .collect::<Vec<_>>()
                        .join(" | ");
                    println!(
                        "  [{idx}] {} — options: {opts} (multiSelect={})",
                        q.question, q.multi_select
                    );
                }
                if args.dry_run {
                    continue;
                }
                // Canned reply: first option of each question.
                let answers: HashMap<String, String> = questions
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, q)| {
                        q.options
                            .first()
                            .map(|opt| (idx.to_string(), opt.label.clone()))
                    })
                    .collect();
                let body = AnswerBody { answers };
                let resp = client
                    .post(format!(
                        "{spirit_url}/api/v1/dashboard/permissions/{request_id}/answer"
                    ))
                    .bearer_auth(&token)
                    .json(&body)
                    .send()
                    .await?;
                println!("  answered: HTTP {}", resp.status());
            }
            PermissionStreamFrame::PermissionRequest {
                request_id,
                tool_name,
                args_summary,
                classifier_verdict,
                held_reason,
            } => {
                println!(
                    "[permission_request] request_id={request_id} tool={tool_name} verdict={classifier_verdict:?} args={args_summary} reason={held_reason} (stub does not decide)"
                );
            }
            PermissionStreamFrame::OverflowMarker { dropped_count } => {
                eprintln!("[overflow] {dropped_count} frames dropped");
            }
        }
    }

    Ok(())
}

/// Serialize-side mirror of `PermissionAnswerRequest`. The server-side
/// type is `Deserialize`-only; the `_shape_check` const function below
/// stops compiling if the mirrored shape drifts.
#[derive(serde::Serialize)]
struct AnswerBody {
    answers: HashMap<String, String>,
}

#[allow(dead_code)]
const _SHAPE_CHECK: fn(PermissionAnswerRequest) -> AnswerBody = |req| AnswerBody {
    answers: req.answers,
};
