---
title: "Exec handler discarded stdout on non-zero exit codes"
module: crates/mika-agent/src/skills/executor.rs
problem_type: logic-error
severity: high
symptoms:
  - "Scripts with non-zero exit codes had stdout silently discarded"
  - "Shell-exec handler returned empty output due to run.sh 2>&1 merge"
  - "Agent received 'Process exited with code 1: ' with no useful content"
  - "Tools like grep, diff, linters, health checks lost all output on non-zero exit"
related_issues:
  - 100
date_resolved: 2026-03-12
pr: fix/run-shell-output-on-nonzero-exit
tags:
  - shell-exec
  - tool-execution
  - exit-codes
  - output-handling
  - executor
---

# Exec handler discarded stdout on non-zero exit codes

## Problem

The `execute_exec()` function in `crates/mika-agent/src/skills/executor.rs` branched on `output.status.success()`:

- **Exit 0:** captured stdout, parsed `__mika_v1` envelope, returned `ToolOutput::success`
- **Non-zero exit:** read only `output.stderr`, discarded stdout, returned `ToolOutput::error`

### The Double Problem

The shell-exec handler `run.sh` merges stderr into stdout via `eval "$COMMAND" 2>&1`. On non-zero exit:

1. stdout contained the actual command output (including any stderr that was merged)
2. stderr was **empty** (already redirected to stdout by the shell)
3. The executor read the empty stderr pipe
4. Result: `"Process exited with code 1: "` — no output at all

This broke tools that use non-zero exit codes as semantic status: `grep` (1 = no matches), `diff` (1 = files differ), linters (non-zero = warnings), health checks (2 = critical).

### Before (broken)

```rust
} else {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);
    Ok(ToolOutput::error(format!(
        "Process exited with code {exit_code}: {}",
        truncate_output(&stderr)  // EMPTY because run.sh merged stderr into stdout
    )))
}
```

## Root Cause

Conflation of **execution success** (process spawned and ran to completion) with **process exit code** (an output value the agent should interpret). The executor pre-judged non-zero exit codes as tool errors, which is incorrect for most Unix tools.

## Solution

Changed `execute_exec()` to always capture both stdout and stderr, returning `ToolOutput::success` with an exit code prefix for non-zero exits. Only OS-level failures (spawn error, timeout) return `ToolOutput::error`.

### After (fixed)

```rust
let stdout = String::from_utf8_lossy(&output.stdout);
let stderr = String::from_utf8_lossy(&output.stderr);

if output.status.success() {
    // Exit 0: unchanged behavior (envelope parsing, etc.)
} else {
    let code_display = match output.status.code() {
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
    let stderr_trimmed = stderr.trim();
    if !stderr_trimmed.is_empty() && stderr_trimmed != stdout.trim() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(stderr_trimmed);
    }

    let truncated = truncate_output(&combined);
    Ok(ToolOutput::success(format!("{code_display}\n{truncated}")))
}
```

### Output Format

| Scenario | Output |
|----------|--------|
| Exit 0 | `<stdout>` (no prefix) |
| Non-zero with output | `Exit code: 2\n<stdout>\n<stderr if different>` |
| Killed by signal | `Killed by signal: 9\n<output>` |
| Spawn failure | `ToolOutput::error(...)` |
| Timeout | `ToolOutput::error("timed out...")` |

### Supporting Changes

1. **`ToolCallSummary.non_zero_exit`** (`agent.rs`): Heuristic detection of exit code prefix in tool output, with `[NON-ZERO]` tag in tool history context (distinct from `[FAILED]`). Backward compatible via `#[serde(default)]`.

2. **Skill descriptions**: Updated `shell-exec/skill.toml` and `tools.json` with exit code semantics guidance so the agent knows non-zero exit is not necessarily an error.

3. **`__mika_v1` envelope**: Only parsed on exit 0. Images from failed executions could be misleading.

## Key Design Decisions

- **Exit code as prefix, not suffix** — survives `truncate_output()` at MAX_OUTPUT_LEN (10K chars)
- **Stderr dedup via equality check** — simple, safe, handles the `2>&1` merge case
- **No changes to run.sh** — the `2>&1` merge is intentional; executor fix is sufficient
- **`execute_http` excluded** — HTTP status codes are well-defined errors; exit codes are ambiguous

## Prevention

### Principle: Separate Execution from Interpretation

Always capture subprocess output **before** branching on exit status. Exit codes are data for the caller to interpret, not errors for the executor to judge.

```rust
// Good: capture first, interpret second
let output = child.wait_with_output().await?;
let stdout = String::from_utf8_lossy(&output.stdout);
let stderr = String::from_utf8_lossy(&output.stderr);
// NOW branch on output.status.success()

// Bad: branch first, capture selectively
if output.status.success() {
    let stdout = ...  // only here!
} else {
    let stderr = ...  // stdout lost!
}
```

### Test Coverage

Six regression tests in `executor.rs`:

1. `test_exec_handler_nonzero_exit_returns_output` — basic non-zero with stderr
2. `test_exec_handler_nonzero_exit_with_stdout` — exit 2 with stdout (health check pattern)
3. `test_exec_handler_nonzero_exit_empty_output` — silent failure (exit 3)
4. `test_exec_handler_nonzero_exit_via_run_sh` — integration through real `run.sh` handler
5. `test_exec_handler_exit_zero_unchanged` — exit 0 backward compatibility
6. `test_exec_handler_nonzero_exit_stdout_and_stderr` — both streams present

## Related Documentation

- [shell-exec-jq-json-parsing](../integration-issues/shell-exec-jq-json-parsing.md) — prior executor fix (grep to jq)
- [shell-exec-escape-chars-css-multiline](../integration-issues/shell-exec-escape-chars-css-multiline.md) — escape handling regression tests
- [env-var-leakage-exec-handler-child-processes](../security-issues/env-var-leakage-exec-handler-child-processes.md) — executor env scrubbing
