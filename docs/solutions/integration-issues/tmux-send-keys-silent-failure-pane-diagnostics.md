---
title: "Fix tmux send-keys silent failure after extended use"
date: 2026-02-28
category: integration-issues
module: mika-agent/skills/tmux
severity: high
tags:
  - tmux
  - silent-failure
  - observability
  - exec-handler
  - pane-state-validation
  - defense-in-depth
  - shell-hardening
  - echo-to-printf
  - copy-mode
  - dead-pane-detection
  - timeout-mismatch
  - schema-cleanup
related_files:
  - crates/mika-agent/src/skills/executor.rs
  - templates/skills/tmux/handlers/send_command.sh
  - templates/skills/tmux/handlers/create_session.sh
  - templates/skills/tmux/handlers/read_output.sh
  - templates/skills/tmux/handlers/list_sessions.sh
  - templates/skills/tmux/handlers/kill_session.sh
  - templates/skills/tmux/handlers/wait_for_text.sh
  - templates/skills/tmux/tools.json
  - templates/skills/tmux/system_prompt.md
---

# Fix tmux send-keys Silent Failure After Extended Use

## Problem

After extended use, `tmux send-keys -t <session> Enter` stops having visible effect in the target pane. The command returns exit 0, the handler script logs it correctly, but the keystroke produces no result. Running the identical command manually from a different terminal works immediately.

### Symptoms

- Handler script executes successfully (exit code 0)
- Debug output shows the exact tmux command being issued
- The keystroke has no visible effect in the target pane
- Manual execution of the same command works immediately

### Key Architectural Misunderstanding

The original debugging hypothesis assumed Mika runs **inside** a tmux session, causing `TMUX` environment variable pollution to make `tmux send-keys` target the wrong server context.

**The corrected understanding:** Mika runs **outside** tmux as a standalone process (CLI or server). It sends commands **to** an external tmux session (typically running Claude Code) via the tmux server's Unix domain socket. The `TMUX` variable is not set in Mika's process environment during normal operation.

```
Mika agent loop → executor.rs spawns subprocess → handler.sh runs
  → `tmux send-keys -t <session>` → tmux server → target pane (Claude Code)
```

## Root Cause

**Unknown — likely multi-factor.** Since Mika does not run inside tmux, `TMUX` env var pollution was ruled out. The actual cause is bounded to these possibilities:

1. **Pane state issues**: Target pane enters copy-mode or dies, silently swallowing send-keys input
2. **Rapid-fire timing**: Multiple `send_command` calls overwhelm the tmux server or the TUI application
3. **Terminal input buffer pressure**: After many send-keys calls, the pty buffer may degrade
4. **Zero observability**: Executor discarded stdout/stderr on exit 0, making failures invisible

### Contributing Code Issues Found

| Issue | Impact |
|-------|--------|
| `echo "$INPUT" \| jq` in handlers | Misbehaves if input starts with `-n`/`-e`/`-E` |
| No pane_dead check in read/wait handlers | Stale reads, pointless polling |
| wait_for_text max timeout 60s vs 30s skill timeout | Executor kills handler before internal timeout |
| tools.json `interval` param never implemented | Schema/implementation mismatch |
| No agent guidance for dead panes | LLM has no recovery strategy |

## Solution

Three-phase approach: diagnostics, hardening, agent guidance.

### Phase 1: Executor Diagnostics (`executor.rs`)

Added `tool_name` parameter to `execute_exec()` and debug logging on success:

```rust
let stdout_end = {
    let mut b = stdout.len().min(200);
    while b > 0 && !stdout.is_char_boundary(b) {
        b -= 1;
    }
    b
};
tracing::debug!(
    tool = %tool_name,
    stdout_len = stdout.len(),
    stdout_preview = %&stdout[..stdout_end],
    "skill exec succeeded"
);
```

Key details:
- Char-boundary-safe slicing prevents panics on multi-byte UTF-8
- Both stdout and stderr logged at debug level, even on success
- `.env_remove("TMUX")` / `.env_remove("TMUX_PANE")` added as defense-in-depth

### Phase 2: Handler Script Hardening (all 6 scripts)

**All handlers received:**
- `unset TMUX TMUX_PANE` at the top (defense-in-depth)
- `printf '%s\n'` replacing `echo` for input piping

**send_command.sh — pane state checks:**
```sh
# Check pane is alive
PANE_DEAD=$(tmux display-message -t "$SESSION" -p '#{pane_dead}' 2>/dev/null)
if [ "$PANE_DEAD" = "1" ]; then
    echo "Error: target pane in session '$SESSION' is dead" >&2
    exit 1
fi

# Auto-exit copy-mode with verification
if [ "$PANE_MODE" = "1" ]; then
    tmux send-keys -t "$SESSION" -X cancel 2>/dev/null
    sleep 0.1
    PANE_MODE=$(tmux display-message -t "$SESSION" -p '#{pane_in_mode}' 2>/dev/null)
    if [ "$PANE_MODE" = "1" ]; then
        echo "Error: pane in session '$SESSION' is stuck in copy-mode" >&2
        exit 1
    fi
fi
```

**read_output.sh — dead pane warning:**
```sh
PANE_DEAD=$(tmux display-message -t "$SESSION" -p '#{pane_dead}' 2>/dev/null)
if [ "$PANE_DEAD" = "1" ]; then
    echo "[WARNING: pane in session '$SESSION' is dead — output below is stale]"
fi
```

**wait_for_text.sh — pane_dead early-exit + timeout clamp:**
```sh
# Inside polling loop:
PANE_DEAD=$(tmux display-message -t "$SESSION" -p '#{pane_dead}' 2>/dev/null)
if [ "$PANE_DEAD" = "1" ]; then
    echo "Error: pane in session '$SESSION' died while waiting for '$PATTERN'" >&2
    exit 1
fi
```
Timeout clamped from 60s to 25s (within 30s skill timeout minus 5s safety margin).

**Other changes:**
- Sleep increased from 0.1s to 0.2s in send_command.sh and create_session.sh
- Pane diagnostic (`pane_current_command`) added to send_command.sh output
- Unused `interval` parameter removed from tools.json

### Phase 3: Agent Recovery Guidance (`system_prompt.md`)

```markdown
**Error recovery:**
- If a pane is reported as dead, kill the session and create a new one.
- If copy-mode is stuck, tmux_send_command attempts auto-recovery.
  If still stuck, kill and recreate the session.
- If read_output reports stale output from a dead pane, the process has exited.
```

## Prevention Strategies

### For Silent Failures Generally

1. **Observable outcomes**: Every side-effecting operation should produce a verifiable result. Exit 0 is not proof of success.
2. **State pre-checks**: Validate external resource state before operating (pane alive, not in copy-mode, etc.)
3. **Debug logging with identity**: Always include tool name, handler name in log entries for correlation.
4. **Bounded timeouts everywhere**: Internal handler timeouts must be less than executor timeouts.

### Shell Script Handler Checklist

- [ ] `printf '%s\n'` instead of `echo` for arbitrary data
- [ ] All variable expansions double-quoted
- [ ] Input parameters validated for emptiness and character set
- [ ] External commands checked with `command -v`
- [ ] Timeouts clamped to sane ranges
- [ ] Pre-condition checks before core operation
- [ ] Post-condition verification where possible
- [ ] stdout for results, stderr for diagnostics
- [ ] No `eval` on LLM-provided input
- [ ] tools.json schema matches actual handler implementation

### Key Insight

> The absence of an error is not the presence of success. Every operation at an integration boundary needs positive confirmation that it achieved its intended effect.

## Related Documentation

- [Shell injection hardening (7aba1ec)](../security-issues/code-review-7aba1ec-shell-injection-memory-safety.md) — Prior P1-critical fixes to tmux handlers
- [Skill availability and send_message honesty](../logic-errors/skill-availability-and-send-message-honesty.md) — `safe_always_on_skills()` filtering for exec handlers
- [jq pretty-print envelope detection](../logic-errors/jq-pretty-print-envelope-detection.md) — Executor output parsing patterns
- [Filesystem skill registry](../architecture-decisions/filesystem-skill-registry-implementation.md) — Skill system architecture
- [Multimodal tool results](../feature-implementation/multimodal-tool-results.md) — `__mika_v1` image protocol in executor
- [Disable bundled skills config](../feature-implementation/disable-bundled-skills-config.md) — Bundled skill re-sync behavior
