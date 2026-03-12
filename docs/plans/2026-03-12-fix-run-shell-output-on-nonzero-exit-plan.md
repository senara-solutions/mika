---
title: "fix: run_shell always capture and return output regardless of exit code"
type: fix
status: completed
date: 2026-03-12
issue: 100
---

# fix: run_shell always capture and return output regardless of exit code

## Overview

The `run_shell` tool treats any non-zero exit code as a failure and discards stdout. This causes silent failures for scripts that use non-zero exit codes to signal status (health checks, linters, test runners). The executor should separate *execution success* from *process exit code*.

## Problem Statement

In `crates/mika-agent/src/skills/executor.rs`, `execute_exec()` branches on `output.status.success()`:
- **Success branch (exit 0):** captures stdout, parses `__mika_v1` envelope, returns `ToolOutput::success`
- **Failure branch (non-zero):** reads only `output.stderr`, discards stdout entirely, returns `ToolOutput::error`

**Double problem:** The shell-exec handler `run.sh` does `eval "$COMMAND" 2>&1` which merges stderr into stdout. On non-zero exit, the executor reads `output.stderr` (empty because of the merge), producing: `"Process exited with code 1: "` — no output at all.

## Proposed Solution

Change `execute_exec()` to always capture both stdout and stderr, returning `ToolOutput::success` with the exit code as a prefix line when non-zero. Only return `ToolOutput::error` for OS-level failures (spawn error, timeout).

### Output format

**Exit 0 (no change):**
```
<stdout content>
```

**Non-zero exit:**
```
Exit code: 2
<stdout content>
<stderr content if non-empty and different from stdout>
```

**OS-level failure (no change):**
```
ToolOutput::error("Failed to execute: <reason>")
```

### Design decisions

1. **Exit code as prefix line, not suffix** — survives truncation at `MAX_OUTPUT_LEN`
2. **Skip `__mika_v1` envelope parsing on non-zero exit** — images from failed executions may be misleading; the exit code prefix would break JSON parsing anyway
3. **Don't modify `run.sh`** — the `2>&1` merge is a deliberate simplification; the executor fix (always reading stdout) is sufficient
4. **`execute_http` is out of scope** — HTTP status codes are a well-defined error contract; non-zero exit codes are ambiguous (grep returns 1 for "no matches")
5. **`spawn_long_running_exec` is out of scope** — its stdout is `Stdio::null()` by design; scripts deliver results via `mika ask --task-id`

## Technical Considerations

- **`is_error` behavioral change:** Changing from `ToolOutput::error` to `ToolOutput::success` means the Claude API `is_error` flag flips from `true` to absent for non-zero exits. This is the correct design — the agent should interpret exit codes, not the executor. Tool call summaries in `messages.metadata` will now show `success: true` for these cases.
- **Truncation ordering:** Build exit code prefix first, then truncate the combined output, to guarantee the exit code is never lost.
- **Signal termination:** `status.code()` returns `None` for signal kills (SIGKILL, SIGSEGV). Use `status.signal()` on Unix to report the signal number. Fall back to -1 if neither is available.

## Acceptance Criteria

- [x] `execute_exec()` captures stdout on non-zero exit (no longer discards it)
- [x] `execute_exec()` includes stderr on non-zero exit (appended after stdout if non-empty)
- [x] Non-zero exit returns `ToolOutput::success` with `Exit code: N\n` prefix
- [x] Exit 0 behavior unchanged (no prefix, envelope parsing works)
- [x] OS-level spawn failures still return `ToolOutput::error`
- [x] Timeout still returns `ToolOutput::error`
- [x] Signal termination reports signal number (e.g., `Killed by signal: N`)
- [x] Truncation preserves exit code prefix
- [x] Existing test `test_exec_handler_failure` updated for new behavior
- [x] New test: non-zero exit with stdout content
- [x] New test: non-zero exit with empty output
- [x] New test: exit 0 unchanged behavior
- [ ] New test: signal termination handling

## MVP

### `crates/mika-agent/src/skills/executor.rs` — `execute_exec()` change

Replace the if/else branch at lines 325-383:

```rust
// After: always capture stdout regardless of exit code
let stdout = String::from_utf8_lossy(&output.stdout);
let stderr = String::from_utf8_lossy(&output.stderr);
let exit_code = output.status.code();

if output.status.success() {
    // Exit 0: existing behavior — envelope parsing, truncation, etc.
    // ... (keep current success branch logic intact)
} else {
    // Non-zero exit: return success with exit code prefix
    let code_display = match exit_code {
        Some(code) => format!("Exit code: {code}"),
        None => {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                match output.status.signal() {
                    Some(sig) => format!("Killed by signal: {sig}"),
                    None => "Exit code: unknown".to_string(),
                }
            }
            #[cfg(not(unix))]
            { "Exit code: unknown".to_string() }
        }
    };

    let mut combined = stdout.to_string();
    let stderr_str = stderr.trim();
    if !stderr_str.is_empty() && stderr_str != stdout.trim() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(stderr_str);
    }

    let truncated = truncate_output(&combined);
    Ok(ToolOutput::success(format!("{code_display}\n{truncated}")))
}
```

### Test updates in `executor.rs`

```rust
#[tokio::test]
async fn test_exec_handler_nonzero_exit_captures_stdout() {
    // Script that writes to stdout and exits 1
    // Assert: is_error == false, content contains stdout AND exit code
}

#[tokio::test]
async fn test_exec_handler_nonzero_exit_empty_output() {
    // Script that just does exit 2
    // Assert: is_error == false, content contains "Exit code: 2"
}

#[tokio::test]
async fn test_exec_handler_success_unchanged() {
    // Script that writes to stdout and exits 0
    // Assert: is_error == false, content does NOT contain "Exit code:"
}
```

## Sources

- GitHub Issue: #100
- Key file: `crates/mika-agent/src/skills/executor.rs` (lines 325-383)
- Handler: `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh` (line 28: `2>&1`)
- ToolOutput: `crates/mika-agent/src/tools/mod.rs` (lines 117-148)
- Prior art: `docs/solutions/integration-issues/shell-exec-escape-chars-css-multiline.md`
- Prior art: `docs/solutions/integration-issues/shell-exec-jq-json-parsing.md`
