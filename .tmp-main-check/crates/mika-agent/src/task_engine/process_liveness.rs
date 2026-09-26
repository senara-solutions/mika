//! Process liveness detection for the callback watchdog (#959).
//!
//! Detects when a long-running subprocess has exited without delivering its
//! callback result, enabling the engine to mark the callback task as `failed`
//! and unblock the dispatch queue.
//!
//! **Platform assumption: Linux only.** Uses `/proc/<pid>/stat` for PID reuse
//! detection via process start time (field 22). Consistent with existing
//! precedent in `process_kill.rs` and the container-isolated deployment model.

/// Read the process start time (field 22 = `starttime`) from `/proc/<pid>/stat`.
///
/// Returns `None` if the process doesn't exist or `/proc` is unreadable.
/// Field 22 is the number of clock ticks since system boot when the process
/// started — unique enough with the PID to identify a specific process instance.
///
/// Parsing strategy: the `comm` field (field 2) can contain spaces and parens,
/// so we find the *last* `)` to skip past it, then index into whitespace-delimited
/// fields. After the closing paren, fields start at index 0 = state (field 3),
/// so `starttime` (field 22) is at offset index 19.
pub fn read_process_start_time(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after_comm = stat.rfind(')')? + 2; // skip past ") "
        if after_comm >= stat.len() {
            return None;
        }
        let fields: Vec<&str> = stat[after_comm..].split_whitespace().collect();
        // field 22 (starttime) is at index 19 after the comm closure
        fields.get(19)?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// Check if a process is alive AND is the same process we originally spawned.
///
/// Returns `true` only if:
/// 1. A process exists at the given PID (`kill(pid, 0)` succeeds or `/proc/<pid>` exists)
/// 2. Its start time matches `expected_start_time` (guards against PID reuse)
///
/// Returns `false` if:
/// - The PID doesn't exist (process died)
/// - The PID exists but belongs to a different process (PID was reused)
/// - `/proc/<pid>/stat` is unreadable (race between check and read — treat as dead)
/// - On non-Linux platforms (always returns false — watchdog disabled)
pub fn is_same_process_alive(pid: u32, expected_start_time: u64) -> bool {
    #[cfg(target_os = "linux")]
    {
        // First: is anything running at this PID?
        // Safety: kill(pid, 0) doesn't actually send a signal — it just checks
        // whether the process exists and we have permission to signal it.
        let signal_result = unsafe { libc::kill(pid as i32, 0) };
        if signal_result == -1 {
            // ESRCH (no such process) or EPERM — process gone or inaccessible
            return false;
        }
        // Second: is it the SAME process? Check /proc/<pid>/stat field 22
        match read_process_start_time(pid) {
            Some(actual_start_time) => actual_start_time == expected_start_time,
            // /proc entry gone between kill(0) and read — race, treat as dead
            None => false,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, expected_start_time);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_start_time_of_self() {
        // This test only runs on Linux
        if !cfg!(target_os = "linux") {
            return;
        }
        let pid = std::process::id();
        let start_time = read_process_start_time(pid);
        assert!(
            start_time.is_some(),
            "should be able to read own process start time"
        );
        // Start time should be non-zero
        assert!(start_time.unwrap() > 0);
    }

    #[test]
    fn read_start_time_of_nonexistent_pid() {
        // PID 999999999 almost certainly doesn't exist
        let start_time = read_process_start_time(999_999_999);
        assert!(start_time.is_none());
    }

    #[test]
    fn is_same_process_alive_self() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let pid = std::process::id();
        let start_time = read_process_start_time(pid).expect("should read own start time");
        assert!(is_same_process_alive(pid, start_time));
    }

    #[test]
    fn is_same_process_alive_wrong_start_time() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let pid = std::process::id();
        // Use a bogus start time — should detect PID reuse
        assert!(!is_same_process_alive(pid, 12345));
    }

    #[test]
    fn is_same_process_alive_nonexistent_pid() {
        // PID 999999999 doesn't exist
        assert!(!is_same_process_alive(999_999_999, 12345));
    }
}
