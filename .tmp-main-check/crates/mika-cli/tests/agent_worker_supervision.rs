//! Integration tests for the TUI agent-worker supervisor (mika#1149).
//!
//! These exercise the supervisor primitive (`mika_cli::supervision::supervise`)
//! against the two failure shapes the production wiring depends on:
//!
//! 1. **Panic path** — a `#[tokio::main]` worker task that panics inside its
//!    body. `JoinHandle::await` resolves as `Err(JoinError::is_panic)` and the
//!    supervisor surfaces a `worker panicked: <payload>` reason.
//! 2. **Premature clean exit** — a worker task that returns `Ok(())` while the
//!    session is still active (mirrors the production case where the worker's
//!    `while let Some(req) = user_rx.recv().await` loop exits because all
//!    senders were dropped without an operator-driven shutdown). The
//!    supervisor surfaces a `worker exited before session closed` reason.
//!
//! Both shapes must be observable so the TUI can re-render with a visible
//! error state and arm `/restart`. Silent-drop regressions on either branch
//! would re-introduce the mika#850 P0 failure mode.
//!
//! We test the supervisor primitive directly rather than spinning up the
//! whole `spawn_agent_worker` stack: the contract under test is
//! "what does the supervisor do when its worker handle resolves," and the
//! integration with `run_agent` (a `#[cfg(test)]`-gated `PanicTool` would just
//! exercise tokio's panic propagation, which is already well-covered upstream).
//! The supervisor's behaviour is the load-bearing piece — if it stays correct,
//! any future worker body that panics or exits cleanly is automatically
//! surfaced.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use mika_cli::supervision::{WorkerFailure, supervise};

/// Returns a closure that writes the failure reason into `slot`. Lets each
/// test capture the supervisor's report without channel plumbing.
fn record_reason(slot: &Arc<Mutex<Option<String>>>) -> impl FnOnce(WorkerFailure) + Send + 'static {
    let slot = slot.clone();
    move |failure: WorkerFailure| {
        *slot.lock().unwrap() = Some(failure.reason);
    }
}

#[tokio::test]
async fn worker_panic_surfaces_as_worker_crashed_reason() {
    // Mirrors the production failure: a tool execution panics inside the
    // worker's `tokio::spawn`; supervisor must surface it.
    let worker = tokio::spawn(async {
        panic!("simulated tool panic");
    });
    let shutdown = Arc::new(AtomicBool::new(false));
    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    tokio::time::timeout(
        Duration::from_secs(2),
        supervise(worker, shutdown, record_reason(&captured)),
    )
    .await
    .expect("supervisor did not resolve within 2s");

    let reason = captured
        .lock()
        .unwrap()
        .clone()
        .expect("supervisor must report a reason on worker panic");
    assert!(
        reason.contains("panicked"),
        "expected panic marker in reason: {reason}"
    );
    assert!(
        reason.contains("simulated tool panic"),
        "expected panic payload in reason: {reason}"
    );
}

#[tokio::test]
async fn worker_premature_clean_exit_surfaces_as_worker_crashed_reason() {
    // Production analog: the worker's `while let Some(req) = user_rx.recv().await`
    // loop exits because all `agent_tx` senders were dropped (e.g., a refactor
    // accidentally drops them before issuing `AgentRequest::Quit`). The session
    // is still active from the user's perspective.
    let worker = tokio::spawn(async {
        // Simulate the loop running briefly, then exiting cleanly.
        tokio::time::sleep(Duration::from_millis(10)).await;
    });
    let shutdown = Arc::new(AtomicBool::new(false));
    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    tokio::time::timeout(
        Duration::from_secs(2),
        supervise(worker, shutdown, record_reason(&captured)),
    )
    .await
    .expect("supervisor did not resolve within 2s");

    let reason = captured
        .lock()
        .unwrap()
        .clone()
        .expect("supervisor must report a reason on premature clean exit");
    assert!(
        reason.contains("exited before session closed"),
        "expected premature-exit marker in reason: {reason}"
    );
}

#[tokio::test]
async fn operator_initiated_shutdown_silences_supervisor() {
    // Production analog: the chat loop sets shutdown_initiated = true before
    // sending AgentRequest::Quit. The worker then exits Ok(()), but that's
    // expected — supervisor must not surface a false-positive crash.
    let worker = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(10)).await;
    });
    let shutdown = Arc::new(AtomicBool::new(true));
    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    tokio::time::timeout(
        Duration::from_secs(2),
        supervise(worker, shutdown, record_reason(&captured)),
    )
    .await
    .expect("supervisor did not resolve within 2s");

    assert!(
        captured.lock().unwrap().is_none(),
        "supervisor must stay silent on operator-initiated shutdown"
    );
}
