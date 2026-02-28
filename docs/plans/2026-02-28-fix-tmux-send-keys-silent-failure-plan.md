---
title: "fix: tmux send-keys silently stops working after extended use"
type: fix
status: completed
date: 2026-02-28
---

# fix: tmux send-keys silently stops working after extended use

## Overview

The tmux skill's `send_command.sh` handler executes successfully (exit 0, correct command logged in debug output) but after extended use, `tmux send-keys -t <session> Enter` stops having a visible effect in the target terminal. Running the exact same command manually works immediately. This is a reliability bug that makes the tmux skill unusable for sustained agent automation.

## Architecture Context

**Mika does NOT run inside a tmux session.** Mika is a standalone process (CLI or server) that sends tmux commands to an external tmux session — typically one running Claude Code. All tmux interactions go through the tmux server's Unix domain socket (`/tmp/tmux-$UID/default`). The flow is:

```
Mika agent loop → executor.rs spawns subprocess → handler.sh runs
  → `tmux send-keys -t <session>` → tmux server → target pane (Claude Code)
```

This means the `TMUX` environment variable is **not set** in Mika's process environment during normal operation (Mika is not a tmux client). The original plan incorrectly identified `TMUX` env var pollution as the root cause.

## Problem Statement

**Symptoms:**
- Handler script runs, logs the exact tmux command being called
- `tmux send-keys -t <session> Enter` returns exit 0
- The keystroke has no visible effect in the target pane
- Running the identical command manually from a different terminal works immediately

**Root cause: Unknown — likely a combination of factors.**

Since Mika does not run inside tmux, the `TMUX` environment variable is not the cause. Possible contributing factors:

1. **Pane state issues**: The target pane enters copy-mode, or the pane process dies, silently swallowing send-keys input
2. **Rapid-fire timing**: Multiple `send_command` calls in quick succession overwhelm the tmux server or the application in the pane (Claude Code is an interactive TUI)
3. **Terminal input buffer pressure**: After many send-keys calls, the pty buffer or terminal emulator state may degrade
4. **Zero observability on success**: Executor logs tool name + input at INFO but discards stdout/stderr on exit 0 (`executor.rs:261-293`). Silent failures are invisible — we literally cannot tell what's happening

**Contributing code-level issues:**
1. **`echo` vs `printf` fragility**: `kill_session.sh` and `wait_for_text.sh` still use `echo "$INPUT" | jq` which can misbehave if input starts with `-n`/`-e`/`-E` flags
2. **No pane state verification in read/wait handlers**: `read_output.sh` and `wait_for_text.sh` don't check `pane_dead`, leading to stale reads or pointless polling
3. **Timeout mismatch**: `wait_for_text.sh` accepts timeout up to 60s, but the skill timeout in `skill.toml` is 30s — the executor kills the handler before its internal timeout fires
4. **Schema/implementation mismatch**: `tools.json` advertises an `interval` parameter for `wait_for_text` but the handler ignores it (hardcoded `sleep 1`)
5. **No agent guidance for dead panes**: System prompt has no recovery guidance when pane_dead is detected

## Proposed Solution

Three-phase approach: add diagnostics first (we need visibility), then harden all handlers consistently, then add agent-level guidance.

### Phase 1: Add Exec Handler Diagnostics

**`crates/mika-agent/src/skills/executor.rs`** — Fix debug logging to include tool name and capture stderr.

The current working-tree changes already add debug logging at lines 266-276, but they're missing the tool name field. Fix:

```rust
// executor.rs — add tool name to debug log
tracing::debug!(
    tool = %skill_tool.definition.name,  // ADD THIS — currently missing
    stdout_len = stdout.len(),
    stdout_preview = %&stdout[..stdout.len().min(200)],
    "skill exec succeeded"
);
if !stderr.trim().is_empty() {
    tracing::debug!(
        tool = %skill_tool.definition.name,  // ADD THIS — currently missing
        stderr = %&stderr[..stderr.len().min(500)],
        "skill exec stderr on success"
    );
}
```

Note: The `execute_exec` function doesn't have access to `skill_tool` — it receives `command` and `skill_dir`. The tool name must be passed down or the logging must move to `execute_inner` which has access to `skill_tool`. The cleanest approach: add the tool name as a parameter to `execute_exec`, or move the debug logging to `execute_inner` after the `execute_exec` call returns.

**Files:**
- `crates/mika-agent/src/skills/executor.rs` (fix debug logging to include tool name)

### Phase 2: Harden All 6 Handler Scripts Consistently

The current working-tree changes updated 4 of 6 handlers. Complete the remaining 2 and add missing checks.

#### 2a. Complete `kill_session.sh` and `wait_for_text.sh`

Both scripts need:
- `unset TMUX TMUX_PANE` at the top (defensive — protects if scripts are ever invoked from within tmux during development)
- Replace all `echo "$INPUT" | jq/grep` with `printf '%s\n' "$INPUT" | jq/grep`

**Files:**
- `templates/skills/tmux/handlers/kill_session.sh`
- `templates/skills/tmux/handlers/wait_for_text.sh`

#### 2b. Add pane_dead awareness to `read_output.sh`

After the session existence check, query `#{pane_dead}`. If the pane is dead, prepend a warning line to the output so the agent knows it's reading stale content:

```sh
PANE_DEAD=$(tmux display-message -t "$SESSION" -p '#{pane_dead}' 2>/dev/null)
if [ "$PANE_DEAD" = "1" ]; then
    echo "[WARNING: pane in session '$SESSION' is dead — output below is stale]"
fi
```

**Files:**
- `templates/skills/tmux/handlers/read_output.sh`

#### 2c. Add pane_dead early-exit to `wait_for_text.sh`

Inside the polling loop, check `#{pane_dead}` and exit immediately with an error rather than polling stale content until timeout:

```sh
# Inside the while loop, before capture-pane:
PANE_DEAD=$(tmux display-message -t "$SESSION" -p '#{pane_dead}' 2>/dev/null)
if [ "$PANE_DEAD" = "1" ]; then
    echo "Error: pane in session '$SESSION' died while waiting for '$PATTERN'" >&2
    exit 1
fi
```

**Files:**
- `templates/skills/tmux/handlers/wait_for_text.sh`

#### 2d. Verify copy-mode exit in `send_command.sh`

After the `-X cancel` + `sleep 0.1`, re-check `pane_in_mode`. If still in copy-mode, return an error instead of silently sending keys that will be interpreted as copy-mode navigation:

```sh
# After cancel + sleep:
PANE_MODE=$(tmux display-message -t "$SESSION" -p '#{pane_in_mode}' 2>/dev/null)
if [ "$PANE_MODE" = "1" ]; then
    echo "Error: pane in session '$SESSION' is stuck in copy-mode" >&2
    exit 1
fi
```

**Files:**
- `templates/skills/tmux/handlers/send_command.sh`

#### 2e. Fix timeout mismatch in `wait_for_text.sh`

Clamp the handler's internal timeout to 25 seconds (skill timeout 30s minus 5s safety margin) so the handler always exits cleanly before the executor kills it:

```sh
# Change max timeout from 60 to 25
if [ "$TIMEOUT" -gt 25 ]; then TIMEOUT=25; fi
```

**Files:**
- `templates/skills/tmux/handlers/wait_for_text.sh`

#### 2f. Implement `interval` parameter in `wait_for_text.sh`

The `tools.json` schema advertises an `interval` parameter (default 0.5s) but the handler ignores it. Either implement it or remove it from the schema. Implementing is straightforward:

```sh
# Parse interval (add to JSON parsing section)
INTERVAL=$(printf '%s\n' "$INPUT" | jq -r '.interval // 0.5')
# Validate: must be a number between 0.2 and 5
case "$INTERVAL" in
    ''|*[!0-9.]*) INTERVAL=1 ;;
esac
# ... use in loop:
sleep "$INTERVAL"
```

Simpler alternative: remove `interval` from `tools.json` and keep `sleep 1`. Given the 25s max timeout, 1s polling is adequate.

**Decision: Remove `interval` from `tools.json`** — simpler, and 1s polling is fine for all practical use cases. The LLM won't waste tokens specifying a parameter that does nothing.

**Files:**
- `templates/skills/tmux/tools.json` (remove `interval` property)

#### 2g. Keep existing improvements (already in working tree)

These changes are already applied and correct:
- `unset TMUX TMUX_PANE` in `send_command.sh`, `create_session.sh`, `read_output.sh`, `list_sessions.sh`
- `printf` instead of `echo` in the 4 updated handlers
- Pane liveness checks (`pane_dead`, `pane_in_mode`) in `send_command.sh` and `create_session.sh`
- Auto-exit copy-mode in `send_command.sh`
- `pane_current_command` diagnostic in `send_command.sh` output
- `sleep 0.2` (increased from 0.1) in `send_command.sh` and `create_session.sh`
- `.env_remove("TMUX")` and `.env_remove("TMUX_PANE")` in `executor.rs` (defensive — harmless and protects against development-time scenarios where Mika is launched from within tmux)

### Phase 3: Update System Prompt with Dead Pane Recovery Guidance

Add agent-level guidance for the new pane state errors introduced in Phase 2:

```markdown
**Error recovery:**
- If a pane is reported as dead, kill the session with `tmux_kill_session` and create a new one. Inform the user that the previous process has exited.
- If a pane is stuck in copy-mode, kill the session and recreate it. This is rare but unrecoverable via send-keys.
- If `tmux_read_output` reports stale output from a dead pane, do not retry the same command — the process has exited.
```

**Files:**
- `templates/skills/tmux/system_prompt.md`

## Technical Considerations

- **No retry mechanism**: Adding post-send verification (capture-pane diffing) would add significant complexity. The pane state checks and diagnostics will make the failure mode visible, which is the first step. If the bug persists after these changes, the debug logs will provide the data needed to identify the actual root cause.
- **`env_remove` is defensive, not curative**: Since Mika doesn't run inside tmux, the `TMUX` variable is not set during normal operation. The `env_remove("TMUX")` and `unset TMUX` are defense-in-depth for development scenarios only. They do not fix the production bug.
- **Bundled skill re-seeding**: Changes to `templates/skills/tmux/handlers/` are compiled into the binary and re-seeded to `~/.mika/agents/main/skills/tmux/` on startup. Users must restart Mika to get the updated handlers.
- **Backward compatibility**: All changes are additive — no API or tool schema changes (except removing the unused `interval` parameter that was never implemented).

## Acceptance Criteria

- [x] `executor.rs` debug logging includes the tool name in both stdout and stderr log entries
- [x] All 6 tmux handler scripts unset `TMUX` and `TMUX_PANE` at the top
- [x] All 6 tmux handler scripts use `printf '%s\n'` instead of `echo` for piping input to jq/grep
- [x] `send_command.sh` checks `#{pane_dead}` and `#{pane_in_mode}` before sending keys
- [x] `send_command.sh` auto-exits copy-mode and verifies it exited (error if stuck)
- [x] `send_command.sh` outputs pane diagnostic info (`pane_current_command`) on success
- [x] `read_output.sh` warns when reading from a dead pane
- [x] `wait_for_text.sh` exits immediately with error when pane dies during polling
- [x] `wait_for_text.sh` max timeout clamped to 25s (within skill timeout of 30s)
- [x] `tools.json` removes unused `interval` parameter from `tmux_wait_for_text`
- [x] `system_prompt.md` includes dead pane recovery guidance
- [x] `executor.rs` removes `TMUX` and `TMUX_PANE` from exec handler subprocess environment (defensive)
- [x] `send_command.sh` and `create_session.sh` sleep increased from 0.1 to 0.2
- [x] All existing tests pass (`cargo test`)
- [x] New unit test for `env_remove` behavior in executor (verify `TMUX` is not passed to child)

## Dependencies & Risks

- **Low risk**: All changes are backward-compatible, no schema changes
- **Root cause unresolved**: These changes provide visibility and hardening but may not fix the underlying issue. The debug logs and pane state checks will provide data for further investigation if the bug persists.
- **Re-seeding**: Users with customized runtime handler scripts (e.g., debug logging added manually) will have their changes overwritten on next Mika restart

## References

- Executor code: `crates/mika-agent/src/skills/executor.rs:237-245`
- Handler: `templates/skills/tmux/handlers/send_command.sh`
- Security todo: `todos/211-complete-p1-exec-handler-env-leakage.md`
- Prior fix: `docs/plans/2026-02-27-fix-telegram-delivery-and-tmux-skill-availability-plan.md`
- Shell injection hardening: `docs/solutions/security-issues/code-review-7aba1ec-shell-injection-memory-safety.md`
- jq envelope detection: `docs/solutions/logic-errors/jq-pretty-print-envelope-detection.md`
