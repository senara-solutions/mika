//! Supervisor for the TUI agent-worker `tokio::spawn`.
//!
//! The TUI runs the agent loop locally in a worker task. Without supervision,
//! a panic inside that task silently kills the receiver: subsequent UI sends
//! succeed (mpsc::SendError only fires when the receiver is *dropped*, not when
//! the task owning it panics mid-iteration) and the user sees no feedback.
//! See mika#1149 for the full bug class and forensic context.
//!
//! [`supervise`] awaits the worker's [`JoinHandle`] and reports any unexpected
//! exit — panic *or* premature clean exit — to the caller via a closure.
//! The caller decides how to surface the failure (e.g., send an
//! `AgentResponse::WorkerCrashed { reason }` variant up the agent channel).
//!
//! Two failure modes are reported:
//! - [`JoinError`] from a panic — `is_panic()` true, reason captures the panic
//!   payload via best-effort downcast.
//! - `Ok(())` — the worker's `while let` loop returned without an operator
//!   shutdown signal (e.g., `user_rx` closed because senders were dropped).
//!
//! A shared [`AtomicBool`] (`shutdown_initiated`) lets the operator suppress
//! the false-positive on clean shutdown paths (Quit, agent-switch, /restart):
//! set it to `true` before dropping senders or sending [`AgentRequest::Quit`],
//! and the supervisor exits silently when the worker resolves.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::task::JoinHandle;

/// Failure reported by the supervisor when the worker exits unexpectedly.
pub struct WorkerFailure {
    /// Human-readable reason. Prefixes:
    /// - `"worker panicked: <payload>"` on `JoinError::is_panic()`.
    /// - `"worker task error: <err>"` on other `JoinError` shapes (e.g.,
    ///   cancellation via `JoinHandle::abort`).
    /// - `"worker exited before session closed"` on premature `Ok(())`.
    pub reason: String,
}

/// Await the worker handle and forward any unexpected exit to `on_failure`.
///
/// When `shutdown_initiated` is `true` at the moment the handle resolves,
/// the supervisor returns silently — the worker exited as the operator asked.
/// Otherwise both `Err(JoinError)` and `Ok(())` invoke `on_failure` with a
/// reason string the caller can surface.
pub async fn supervise<F>(
    worker_handle: JoinHandle<()>,
    shutdown_initiated: Arc<AtomicBool>,
    on_failure: F,
) where
    F: FnOnce(WorkerFailure),
{
    let join_result = worker_handle.await;

    if shutdown_initiated.load(Ordering::Acquire) {
        return;
    }

    let reason = match join_result {
        Err(join_err) if join_err.is_panic() => {
            let payload = join_err.into_panic();
            let panic_msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            format!("worker panicked: {panic_msg}")
        }
        Err(join_err) => format!("worker task error: {join_err}"),
        Ok(()) => "worker exited before session closed".to_string(),
    };

    on_failure(WorkerFailure { reason });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    #[tokio::test]
    async fn shutdown_initiated_silences_clean_exit() {
        let worker = tokio::spawn(async {});
        let shutdown = Arc::new(AtomicBool::new(true));
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let sink = captured.clone();

        supervise(worker, shutdown, move |failure| {
            *sink.lock().unwrap() = Some(failure.reason);
        })
        .await;

        assert!(captured.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn premature_clean_exit_is_reported_with_reason() {
        let worker = tokio::spawn(async {});
        let shutdown = Arc::new(AtomicBool::new(false));
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let sink = captured.clone();

        supervise(worker, shutdown, move |failure| {
            *sink.lock().unwrap() = Some(failure.reason);
        })
        .await;

        let reason = captured.lock().unwrap().clone().expect("reason captured");
        assert!(
            reason.contains("exited before session closed"),
            "unexpected reason: {reason}",
        );
    }

    #[tokio::test]
    async fn panic_payload_is_extracted_into_reason() {
        let worker = tokio::spawn(async {
            panic!("simulated worker panic");
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let sink = captured.clone();

        supervise(worker, shutdown, move |failure| {
            *sink.lock().unwrap() = Some(failure.reason);
        })
        .await;

        let reason = captured.lock().unwrap().clone().expect("reason captured");
        assert!(
            reason.contains("panicked"),
            "missing panic marker: {reason}"
        );
        assert!(
            reason.contains("simulated worker panic"),
            "missing panic payload: {reason}",
        );
    }

    #[tokio::test]
    async fn aborted_worker_is_reported() {
        let worker = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let handle_for_abort = worker.abort_handle();
        let shutdown = Arc::new(AtomicBool::new(false));
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let sink = captured.clone();

        handle_for_abort.abort();

        supervise(worker, shutdown, move |failure| {
            *sink.lock().unwrap() = Some(failure.reason);
        })
        .await;

        let reason = captured.lock().unwrap().clone().expect("reason captured");
        assert!(
            reason.contains("worker task error"),
            "unexpected reason: {reason}",
        );
    }
}
