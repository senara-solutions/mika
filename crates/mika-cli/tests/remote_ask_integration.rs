//! Integration tests for `mika_cli::remote_ask` against a mock A2A server.
//!
//! These tests stand up an axum server that mimics the gateway's A2A proxy
//! endpoint (JSON-RPC POST) and exercise the full client transport path —
//! request build, send_message, Task render — without depending on a live mika
//! deployment.
//!
//! Plan: `mika/docs/plans/2026-06-09-003-feat-ascension-architecture-first-slice-cli-plan.md`
//! Unit: U2 (Remote-mode dispatch via A2aClient).

use axum::{Json, Router, http::StatusCode, routing::post};
use mika_cli::remote_ask::{OutputFormat, dispatch_remote};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::net::TcpListener;

/// Captured request shape for assertions on what the client sent.
#[derive(Default)]
struct Capture {
    last_body: Option<Value>,
}

type SharedCapture = Arc<Mutex<Capture>>;

async fn spawn_mock<F>(handler: F) -> (SocketAddr, SharedCapture)
where
    F: Fn(SharedCapture, Json<Value>) -> (StatusCode, Json<Value>) + Clone + Send + Sync + 'static,
{
    let capture: SharedCapture = Arc::new(Mutex::new(Capture::default()));
    let cap_for_handler = capture.clone();
    let app = Router::new().route(
        "/a2a/{customer}/{agent}",
        post(move |body: Json<Value>| {
            let h = handler.clone();
            let cap = cap_for_handler.clone();
            async move { h(cap, body) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });
    (addr, capture)
}

fn ok_task_with_text(text: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "id": "task-mock-1",
            "status": {
                "state": "completed",
                "message": {
                    "messageId": "msg-agent-1",
                    "role": "agent",
                    "parts": [{"kind": "text", "text": text}],
                    "kind": "message",
                }
            },
            "kind": "task",
        }
    })
}

// Bearer-token forwarding is the A2A client's responsibility (covered there); we don't
// repeat it at the CLI layer because env-var mutation across parallel cargo tests races.

#[tokio::test]
async fn dispatch_returns_text_part_in_text_mode() {
    let (addr, _) =
        spawn_mock(|_cap, _body| (StatusCode::OK, Json(ok_task_with_text("hello world")))).await;

    let url = format!("http://{addr}/a2a/cust-1/mika-prime");
    let out = dispatch_remote("hi", &url, OutputFormat::Text, false)
        .await
        .expect("dispatch should succeed");
    assert_eq!(out, "hello world");
}

#[tokio::test]
async fn dispatch_sends_user_message_as_text_part_in_jsonrpc_request() {
    let (addr, capture) = spawn_mock(|cap, Json(body)| {
        cap.lock().unwrap().last_body = Some(body);
        (StatusCode::OK, Json(ok_task_with_text("reply")))
    })
    .await;

    let url = format!("http://{addr}/a2a/cust-3/mika-prime");
    let _ = dispatch_remote("what's on for today", &url, OutputFormat::Text, false)
        .await
        .expect("dispatch should succeed");

    let body = capture.lock().unwrap().last_body.clone().unwrap();
    assert_eq!(body["method"], "message/send");
    let parts = &body["params"]["message"]["parts"];
    assert!(parts.is_array(), "parts should be array, got {parts}");
    assert_eq!(parts[0]["kind"], "text");
    assert_eq!(parts[0]["text"], "what's on for today");
    assert_eq!(body["params"]["message"]["role"], "user");
}

#[tokio::test]
async fn dispatch_surfaces_jsonrpc_error_with_remote_prefix() {
    let (addr, _) = spawn_mock(|_cap, _body| {
        let err = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -32600, "message": "Invalid Request"}
        });
        (StatusCode::OK, Json(err))
    })
    .await;

    let url = format!("http://{addr}/a2a/cust-4/mika-prime");
    let err = dispatch_remote("hi", &url, OutputFormat::Text, false)
        .await
        .expect_err("should fail on JSON-RPC error response");
    let chain = format!("{err:#}");
    assert!(
        chain.contains("remote error:"),
        "expected 'remote error:' prefix, got: {chain}"
    );
    assert!(
        chain.contains("Invalid Request"),
        "expected server message in chain, got: {chain}"
    );
}

#[tokio::test]
async fn dispatch_surfaces_connection_error_for_dead_endpoint() {
    // Bind a listener, immediately drop it — the port is now closed.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let url = format!("http://{addr}/a2a/cust-5/mika-prime");
    let err = dispatch_remote("hi", &url, OutputFormat::Text, false)
        .await
        .expect_err("should fail when no listener accepts the connection");
    let chain = format!("{err:#}");
    assert!(
        chain.contains("connection error:"),
        "expected 'connection error:' prefix, got: {chain}"
    );
}

#[tokio::test]
async fn dispatch_renders_file_part_as_placeholder() {
    let (addr, _) = spawn_mock(|_cap, _body| {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "id": "task-mock-file",
                "status": {
                    "state": "completed",
                    "message": {
                        "messageId": "msg-agent-1",
                        "role": "agent",
                        "parts": [{
                            "kind": "file",
                            "file": {"name": "foo.txt"}
                        }],
                        "kind": "message",
                    }
                },
                "kind": "task",
            }
        });
        (StatusCode::OK, Json(response))
    })
    .await;

    let url = format!("http://{addr}/a2a/cust-6/mika-prime");
    let out = dispatch_remote("hi", &url, OutputFormat::Text, false)
        .await
        .expect("dispatch should succeed");
    assert_eq!(out, "[file: foo.txt]");
}
