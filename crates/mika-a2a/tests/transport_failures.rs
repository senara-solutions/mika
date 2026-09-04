//! End-to-end transport-failure tests for `A2aClient` (mika#2036).
//!
//! These exercise the real socket path rather than a mock: the founding defect
//! was that `reqwest::Client::new()` carried no timeout at all, and that every
//! resulting failure rendered the same sentence. A test that stubs the transport
//! cannot falsify either claim, so each case here drives a real
//! `tokio::net::TcpListener` and asserts on what the client actually observed.

use std::time::Duration;

use mika_a2a::client::{A2aClient, DEFAULT_TIMEOUT};
use mika_a2a::error::TransportFailure;
use mika_a2a::{A2aError, Message, MessageSendParams, Part, Role};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

/// The endpoint path carries an accent on purpose: our nominal population of
/// agent names, plan titles and paths is French, and an ASCII-only fixture does
/// not test the population we actually run.
const AGENT_PATH: &str = "/a2a/mika-arch/révision-de-plan";

fn params(text: &str) -> MessageSendParams {
    MessageSendParams {
        message: Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: Role::User,
            parts: vec![Part::Text {
                text: text.to_string(),
                metadata: None,
            }],
            context_id: Some("ctx-révision-1".to_string()),
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

fn classify(err: A2aError) -> TransportFailure {
    match err {
        A2aError::ClientError(e) => TransportFailure::classify(&e),
        other => panic!("expected a transport failure, got: {other}"),
    }
}

/// AC2, end to end: a server that accepts the connection and then says nothing
/// must be abandoned on the client's own budget.
///
/// Before mika#2036 this test could not terminate — `reqwest::Client::new()`
/// applies no request timeout, so the client waited on the socket indefinitely.
/// Passing it is the proof that a budget now exists and is *enforced*, not
/// merely stored on the struct.
#[tokio::test]
async fn a_silent_server_is_abandoned_on_the_clients_budget() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Accept and hold. Never write a byte, never close.
    let held = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(stream);
    });

    let client = A2aClient::with_timeout(
        format!("http://{addr}{AGENT_PATH}"),
        None,
        Duration::from_secs(1),
    );

    let started = std::time::Instant::now();
    let err = client
        .send_message(params("révision du plan"))
        .await
        .expect_err("a silent server must not be waited on forever");
    let waited = started.elapsed();

    assert_eq!(
        classify(err),
        TransportFailure::TimedOut,
        "a silent server is a timeout, not an unreachable host"
    );
    assert!(
        waited < Duration::from_secs(10),
        "the client waited {waited:?} — its own budget was not enforced"
    );

    held.abort();
}

/// AC3's anti-vacuity clause at the transport layer: a refused port must
/// classify as [`TransportFailure::Unreachable`], the single variant that
/// forbids a recovery attempt. Misclassifying it would send a caller hunting for
/// a task the server never created.
#[tokio::test]
async fn a_refused_port_is_unreachable_and_forbids_recovery() {
    // Bind then drop: the port is known-free and nothing is listening on it.
    let addr = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    };

    let client = A2aClient::new(format!("http://{addr}{AGENT_PATH}"), None);
    let failure = classify(
        client
            .send_message(params("révision du plan"))
            .await
            .expect_err("nothing is listening on this port"),
    );

    assert_eq!(failure, TransportFailure::Unreachable);
    assert!(
        !failure.request_was_sent(),
        "an unreachable server must never trigger a recovery read"
    );
}

/// A socket dropped *after* the request was read is not unreachable: the bytes
/// left, so work may exist on the other side. This is the shape of the founding
/// incident, and the one case where recovery is warranted.
#[tokio::test]
async fn a_socket_dropped_after_the_request_lands_allows_recovery() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        // Read what the client sent, then hang up without answering.
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await;
        drop(stream);
    });

    let client = A2aClient::new(format!("http://{addr}{AGENT_PATH}"), None);
    let failure = classify(
        client
            .send_message(params("révision du plan"))
            .await
            .expect_err("the server hung up without answering"),
    );

    assert_ne!(
        failure,
        TransportFailure::Unreachable,
        "the request was read by the server; calling it unreachable hides the generated answer"
    );
    assert!(
        failure.request_was_sent(),
        "{failure:?} must permit a recovery read"
    );

    server.await.unwrap();
}

/// AC2: `new` keeps its signature and carries the measured default.
#[test]
fn the_default_client_carries_the_measured_budget() {
    let client = A2aClient::new("http://127.0.0.1:9/a2a/mika-arch", None);
    assert_eq!(client.timeout(), DEFAULT_TIMEOUT);
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(300));
}
