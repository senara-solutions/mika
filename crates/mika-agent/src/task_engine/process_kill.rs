//! Shared process termination helpers for task cancellation and orphan cleanup.
//!
//! Provides SIGTERM → grace period → SIGKILL with process-group semantics.
//!
//! **PID reuse risk:** There is a TOCTOU window between checking process liveness
//! and sending a signal. On Linux, `/proc/{pid}/stat` is checked but the process
//! could exit and the PID be reused between the check and the kill. This is
//! acceptable for the current use case (container-isolated agent processes) where
//! PID reuse is rare. For stronger guarantees, store the process start time at
//! spawn and compare with `/proc/{pid}/stat` field 22 (starttime) before killing.

use std::path::PathBuf;
use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;
use crate::db::Task;

/// Grace period between SIGTERM and SIGKILL (seconds).
const KILL_GRACE_PERIOD_SECS: u64 = 5;

/// Check if a process with the given PID is still alive by checking `/proc/{pid}/stat`.
///
/// This is a best-effort check — the process may exit between the check and the kill.
/// On non-Linux systems, always returns `true` (assume alive).
fn is_process_alive(pid: i64) -> bool {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from(format!("/proc/{pid}/stat")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        // On macOS/other: use kill -0 to check
        let _ = pid;
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .is_ok_and(|o| o.status.success())
    }
}

/// Send a signal to a process or process group.
///
/// When `process_group` is true, sends to the process group (negative PID).
fn send_signal(pid: i64, signal: &str, process_group: bool) -> bool {
    let target = if process_group {
        format!("-{pid}")
    } else {
        pid.to_string()
    };

    std::process::Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(&target)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Kill a process with SIGTERM, wait for grace period, then SIGKILL if still alive.
///
/// Attempts process-group kill first to handle child process trees (e.g.,
/// claude-pilot spawning Claude Code), falls back to single-PID kill if the
/// process is not a group leader.
///
/// Returns `true` if the process was terminated (or was already dead).
pub async fn kill_process_gracefully(pid: i64) -> bool {
    // Guard: reject invalid PIDs (negative would be interpreted as process group)
    if pid <= 0 {
        warn!(pid, "refusing to kill invalid PID");
        return false;
    }

    if !is_process_alive(pid) {
        debug!(pid, "process already dead, skipping kill");
        return true;
    }

    info!(pid, "sending SIGTERM to process group");

    // Try process group kill first, fall back to single process
    let term_sent = send_signal(pid, "TERM", true) || send_signal(pid, "TERM", false);

    if !term_sent {
        warn!(pid, "failed to send SIGTERM (ESRCH or EPERM)");
        return !is_process_alive(pid);
    }

    // Wait grace period, checking periodically
    for _ in 0..KILL_GRACE_PERIOD_SECS {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if !is_process_alive(pid) {
            info!(pid, "process exited after SIGTERM");
            return true;
        }
    }

    // Process still alive after grace period — escalate to SIGKILL
    info!(
        pid,
        "process still alive after grace period, sending SIGKILL"
    );
    let kill_sent = send_signal(pid, "KILL", true) || send_signal(pid, "KILL", false);

    if !kill_sent {
        warn!(pid, "failed to send SIGKILL");
    }

    // Brief wait for SIGKILL to take effect
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let dead = !is_process_alive(pid);

    if dead {
        info!(pid, "process terminated after SIGKILL");
    } else {
        warn!(
            pid,
            "process still alive after SIGKILL (zombie or kernel issue)"
        );
    }

    dead
}

/// Kill a process immediately with SIGTERM only (no grace period, no SIGKILL).
///
/// Used by the orphan cleanup path where we don't want to block.
pub fn kill_process_immediate(pid: i64) {
    if pid <= 0 {
        return;
    }
    // Try process group, fall back to single process
    if !send_signal(pid, "TERM", true) {
        send_signal(pid, "TERM", false);
    }
}

/// Outcome of a cancel-with-kill operation.
pub struct CancelOutcome {
    /// The cancelled task's label.
    pub label: String,
    /// Whether a process was killed (None = no process was running).
    pub process_killed: Option<bool>,
    /// The PID that was killed (if any).
    pub pid: Option<i64>,
}

/// Cancel a task and kill its running process (if any).
///
/// Shared logic used by the `cancel_task` tool, HTTP cancel endpoint, and CLI.
/// 1. Loads the task to get `process_id`
/// 2. Updates DB status to `cancelled`
/// 3. Kills the process if one is running
/// 4. Clears the `process_id` in DB
///
/// Returns `None` if the task was not found or not in a cancellable state.
pub async fn cancel_task_and_kill(
    db: &AsyncDatabase,
    task_id: &str,
) -> anyhow::Result<Option<CancelOutcome>> {
    // Load task to get process_id and label before cancelling
    let task: Option<Task> = db.get_task(task_id).await?;
    let process_id = task.as_ref().and_then(|t| t.process_id);
    let label = task
        .as_ref()
        .map(|t| t.label.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let cancelled = db.cancel_task(task_id).await?;
    if !cancelled {
        return Ok(None);
    }

    // Kill the process if one is running
    let process_killed = if let Some(pid) = process_id {
        info!(task_id, pid, "killing process for cancelled task");
        let killed = kill_process_gracefully(pid).await;
        let _ = db.clear_task_process_id(task_id).await;
        Some(killed)
    } else {
        None
    };

    Ok(Some(CancelOutcome {
        label,
        process_killed,
        pid: process_id,
    }))
}
