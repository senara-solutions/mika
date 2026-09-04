//! mika#2036 — a generated response must survive a transport failure.
//!
//! The founding incident: `mika ask` reported `connection error` for exchanges
//! the server had *fully generated*. The answer existed, persisted, on the other
//! side of a socket that had already closed; the caller was told the server was
//! unreachable and threw the work away.
//!
//! These tests drive a raw `TcpListener` rather than an axum mock, because the
//! defect lives *below* the HTTP framing: the server must be able to read the
//! request and then hang up without answering, which a well-behaved HTTP mock
//! cannot do. The mock answers `tasks/get` normally, so a passing test proves
//! the caller reclaimed the answer through the recovery path and not by luck.
//!
//! Fixtures are accented throughout. This repo's nominal population of agent
//! names, plan titles and paths is French; an ASCII-only fixture would not test
//! the traffic we actually run.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use mika_cli::remote_ask::{render_task_parts, send_message_to_agent};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The verdict the founding incident lost and recovered by hand from the server
/// log. Accented, because a real one is.
const GENERATED_VERDICT: &str =
    "Disposition : ITERATE — le périmètre déclaré n'est pas tenu, réécris l'étape 3.";

/// What the mock server saw, so a test can assert the client used its *own*
/// handle for the recovery rather than inventing one.
#[derive(Default, Debug)]
struct Seen {
    /// `message.contextId` carried by the `message/send` the client sent.
    sent_context_id: Option<String>,
    /// The `id` the client asked `tasks/get` for.
    queried_id: Option<String>,
    tasks_get_calls: usize,
}

type SharedSeen = Arc<Mutex<Seen>>;

/// How the mock answers a `tasks/get` after having dropped the `message/send`.
#[derive(Clone, Copy)]
enum OnRecovery {
    /// The generation finished and is on disk — the founding-incident shape.
    Completed,
    /// The agent is still generating. The answer does not exist yet.
    StillWorking,
    /// No task under that context: the request never created one.
    NotFound,
}

/// Read one full HTTP request (headers + `Content-Length` body) off the socket.
async fn read_request(stream: &mut TcpStream) -> Option<Value> {
    let mut raw: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];

    loop {
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&buf[..n]);

        let text = String::from_utf8_lossy(&raw).to_string();
        let Some(head_end) = text.find("\r\n\r\n") else {
            continue;
        };
        let body_start = head_end + 4;
        let content_length = text[..head_end]
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse::<usize>().ok())?
            })
            .unwrap_or(0);

        if raw.len() >= body_start + content_length {
            return serde_json::from_slice(&raw[body_start..body_start + content_length]).ok();
        }
    }
}

async fn write_json(stream: &mut TcpStream, body: &Value) {
    let body = serde_json::to_vec(body).unwrap();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.flush().await;
}

fn task_json(id: &str, context_id: &str, state: &str, text: Option<&str>) -> Value {
    let mut status = serde_json::json!({ "state": state });
    if let Some(text) = text {
        status["message"] = serde_json::json!({
            "messageId": "msg-agent-récupéré",
            "role": "agent",
            "parts": [{"kind": "text", "text": text}],
            "kind": "message",
        });
    }
    serde_json::json!({
        "id": id,
        "contextId": context_id,
        "status": status,
        "kind": "task",
    })
}

/// A server that reads `message/send` and hangs up without answering — then
/// answers `tasks/get` according to `on_recovery`.
async fn spawn_dropping_server(on_recovery: OnRecovery) -> (SocketAddr, SharedSeen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: SharedSeen = Arc::new(Mutex::new(Seen::default()));
    let seen_for_server = seen.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let seen = seen_for_server.clone();
            tokio::spawn(async move {
                let Some(req) = read_request(&mut stream).await else {
                    return;
                };
                let method = req["method"].as_str().unwrap_or_default().to_string();

                if method == "message/send" {
                    let ctx = req["params"]["message"]["contextId"]
                        .as_str()
                        .map(str::to_string);
                    seen.lock().unwrap().sent_context_id = ctx;
                    // The generation completed server-side; the envelope is lost.
                    drop(stream);
                    return;
                }

                if method == "tasks/get" {
                    let id = req["params"]["id"].as_str().unwrap_or_default().to_string();
                    {
                        let mut s = seen.lock().unwrap();
                        s.queried_id = Some(id.clone());
                        s.tasks_get_calls += 1;
                    }
                    let body = match on_recovery {
                        OnRecovery::Completed => serde_json::json!({
                            "jsonrpc": "2.0", "id": 1,
                            "result": task_json("tâche-a2a-1", &id, "completed", Some(GENERATED_VERDICT)),
                        }),
                        OnRecovery::StillWorking => serde_json::json!({
                            "jsonrpc": "2.0", "id": 1,
                            "result": task_json("tâche-a2a-1", &id, "working", None),
                        }),
                        OnRecovery::NotFound => serde_json::json!({
                            "jsonrpc": "2.0", "id": 1,
                            "error": {"code": -32001, "message": "Task not found"},
                        }),
                    };
                    write_json(&mut stream, &body).await;
                }
            });
        }
    });

    (addr, seen)
}

fn endpoint(addr: SocketAddr) -> String {
    format!("http://{addr}/a2a/mika-arch/révision-de-plan")
}

/// **AC3.** The server generated the answer, then the socket died. The caller
/// must come back with the answer, not with an error.
///
/// Remove the recovery path and this test goes red: `send_message_to_agent`
/// returns `Err` and no text is produced.
#[tokio::test]
async fn a_generated_response_survives_a_dropped_socket() {
    let (addr, seen) = spawn_dropping_server(OnRecovery::Completed).await;

    let task = send_message_to_agent("relis ce plan, s'il te plaît", &endpoint(addr), None)
        .await
        .expect("the answer existed server-side and must be reclaimed");

    assert_eq!(
        render_task_parts(&task),
        GENERATED_VERDICT,
        "the reclaimed text must be the generation, verbatim and accented"
    );

    // The recovery used the caller's *own* handle. Without this the test would
    // pass on a server that answered any id at all.
    let seen = seen.lock().unwrap();
    let sent = seen
        .sent_context_id
        .as_deref()
        .expect("the client must send a context id it can correlate on");
    assert!(!sent.is_empty());
    assert_eq!(
        seen.queried_id.as_deref(),
        Some(sent),
        "recovery must query the context the caller itself sent"
    );
}

/// **AC3 anti-vacuity.** A server that was never reached must produce an error,
/// never a phantom recovery — and the recovery read must not even be attempted.
#[tokio::test]
async fn a_refused_port_errors_without_attempting_a_recovery() {
    // Bind then drop: known-free port, nothing listening.
    let addr = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    };

    let err = send_message_to_agent("relis ce plan", &endpoint(addr), None)
        .await
        .expect_err("nothing is listening; this must fail");
    let visible = format!("{err:#}");

    assert!(
        visible.contains("unreachable"),
        "a refused port must be named as unreachable; got: {visible}"
    );
    assert!(
        visible.contains("never left this client"),
        "the caller must be told no answer can exist; got: {visible}"
    );
}

/// **The distinction the 2026-09-04 instance asked for.** A caller must be able
/// to read "busy, retry" apart from "your answer exists and was lost". A
/// still-running task is the first: it must say so, must invite a retry, and
/// must NOT hand back the empty task as if it were an answer.
#[tokio::test]
async fn a_still_running_task_says_retry_rather_than_returning_nothing() {
    let (addr, _) = spawn_dropping_server(OnRecovery::StillWorking).await;

    let err = send_message_to_agent("relis ce plan", &endpoint(addr), None)
        .await
        .expect_err("an unfinished generation is not an answer");
    let visible = format!("{err:#}");

    assert!(
        visible.contains("still working"),
        "the caller must learn the work is in flight; got: {visible}"
    );
    assert!(
        visible.contains("does not exist yet") && visible.contains("Retry"),
        "the caller must be told to retry rather than escalate; got: {visible}"
    );
}

/// The other side of the same distinction: no task under the context means the
/// request never produced work — also a retry, but for a different reason, and
/// the message must not be the same sentence.
#[tokio::test]
async fn a_missing_task_is_reported_differently_from_one_in_flight() {
    let (addr_missing, _) = spawn_dropping_server(OnRecovery::NotFound).await;
    let (addr_running, _) = spawn_dropping_server(OnRecovery::StillWorking).await;

    let missing = format!(
        "{:#}",
        send_message_to_agent("relis ce plan", &endpoint(addr_missing), None)
            .await
            .expect_err("no task exists")
    );
    let running = format!(
        "{:#}",
        send_message_to_agent("relis ce plan", &endpoint(addr_running), None)
            .await
            .expect_err("the task has not finished")
    );

    assert!(
        missing.contains("holds no task"),
        "a missing task must be named as such; got: {missing}"
    );
    assert_ne!(
        missing, running,
        "'nothing was started' and 'still running' must not render the same sentence"
    );
}
