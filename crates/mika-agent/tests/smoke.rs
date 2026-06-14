//! End-to-end smoke tests for the mika-spirit binary.
//!
//! These tests spawn the actual server binary and verify it handles
//! HTTP requests correctly. They require the binary to be built first
//! (`cargo build --bin mika-spirit`).
//!
//! Marked `#[ignore]` — run explicitly with:
//! ```
//! cargo test --test smoke -- --ignored
//! ```

use std::process::{Child, Command};
use std::time::Duration;

/// Spawn the mika-spirit binary on the given port, returning the child process.
fn spawn_server(port: u16) -> Child {
    let binary = env!("CARGO_BIN_EXE_mika-spirit");
    Command::new(binary)
        .env("MIKA_ANTHROPIC_API_KEY", "sk-ant-test-dummy-key-for-smoke")
        .env("MIKA_INTERNAL_TOKEN", "aa".repeat(32)) // 64 hex chars
        .env("MIKA_ROUTING_URL", "http://localhost:19999")
        .env("MIKA_SPIRIT_PORT", port.to_string())
        .env("MIKA_HOME", format!("/tmp/mika-smoke-test-{port}"))
        .env("MIKA_DISABLE_BUNDLED_SKILLS", "true")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn mika-spirit binary")
}

/// Wait for the server to respond on the health endpoint.
async fn wait_for_health(port: u16, timeout: Duration) -> bool {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/health");
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 503 => {
                return true;
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// Find a free TCP port by binding to port 0.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Clean up the temporary home directory.
fn cleanup(port: u16) {
    let path = format!("/tmp/mika-smoke-test-{port}");
    let _ = std::fs::remove_dir_all(&path);
}

#[tokio::test]
#[ignore] // Requires built binary; run with: cargo test --test smoke -- --ignored
async fn health_endpoint_responds() {
    let port = free_port();
    let mut child = spawn_server(port);

    let healthy = wait_for_health(port, Duration::from_secs(15)).await;

    child.kill().ok();
    child.wait().ok();
    cleanup(port);

    assert!(healthy, "server did not respond on /health within 15s");
}

#[tokio::test]
#[ignore]
async fn message_endpoint_requires_auth() {
    let port = free_port();
    let mut child = spawn_server(port);

    if !wait_for_health(port, Duration::from_secs(15)).await {
        child.kill().ok();
        child.wait().ok();
        cleanup(port);
        panic!("server did not start");
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/message"))
        .header("Content-Type", "application/json")
        .body(r#"{"text":"hello","channel_type":"cli"}"#)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status(),
        401,
        "unauthenticated POST /message should return 401"
    );

    child.kill().ok();
    child.wait().ok();
    cleanup(port);
}
