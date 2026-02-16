---
status: complete
priority: p2
issue_id: "207"
tags: [code-review, reliability, tui]
dependencies: []
---

# Agent Worker JoinHandle Dropped — Silent Panic Loss

## Problem Statement

The `tokio::spawn` for the agent worker in `chat.rs` returns a `JoinHandle` that is discarded. If the agent worker panics (e.g., from a tool execution error), the panic is silently swallowed and the TUI hangs forever waiting for a response that never comes.

## Findings

- **Source:** architecture-strategist (Finding 4), security-sentinel (Finding 4)
- **Location:** `crates/mika-cli/src/commands/chat.rs:51`
- **Evidence:** `tokio::spawn(async move { ... })` — JoinHandle not stored. Also, event reader thread JoinHandle at `tui/event.rs:24` is similarly not stored.
- **Impact:** Agent worker panic → TUI freezes in "thinking..." state forever. Event reader thread has minor race with terminal restore.

## Proposed Solutions

### Option 1: Store JoinHandle, check on each tick
- **Pros**: Detects worker crash, surfaces error in TUI
- **Cons**: Minor complexity in tick() to check handle status
- **Effort**: Small
- **Risk**: Low

### Option 2: Wrap agent worker in catch_unwind
- **Pros**: Converts panic to error response sent through channel
- **Cons**: Does not detect other failure modes (channel drop)
- **Effort**: Small
- **Risk**: Low

## Recommended Action

Both — catch_unwind in the worker to send error responses, plus store JoinHandle as a safety net.

## Technical Details

- **Affected files:** `crates/mika-cli/src/commands/chat.rs`, `crates/mika-cli/src/tui/event.rs`

## Acceptance Criteria

- [ ] Agent worker panic produces error message in TUI (not freeze)
- [ ] Event reader thread is joined on clean shutdown
- [ ] TUI remains responsive even after worker failure

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | |

## Resources

- Commit: 399ebf0
