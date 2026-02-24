---
status: complete
priority: p1
issue_id: "200"
tags: [code-review, observability, regression]
dependencies: []
---

# Missing Tracing Initialization in mika-cli

## Problem Statement

The new `mika-cli` crate never calls `mika_common::logging::init_pretty()` or `mika_common::logging::init()`. All `tracing::warn!`, `tracing::info!`, `tracing::debug!` calls throughout the agent loop, scheduler, DB migrations, tool execution, and compaction are silently dropped. This is a regression from the old `cli.rs` which called `logging::init_pretty("info")` as its first line.

## Findings

- **Source:** architecture-strategist (Finding 7), pattern-recognition-specialist (Finding 1), agent-native-reviewer (Finding 2)
- **Location:** `crates/mika-cli/src/main.rs` — missing between parse and dispatch
- **Evidence:** Old `cli.rs:18` had `logging::init_pretty("info")`. New main.rs has no logging init. `tracing` and `tracing-subscriber` are declared as deps but never used for initialization. `chat.rs:38` calls `tracing::warn!` that goes nowhere.
- **Impact:** All structured logging from mika-agent internals is lost. Debugging agent loop issues, tool execution failures, and reminder recovery is impossible.

## Proposed Solutions

### Option 1: Init tracing in main.rs with mode-specific routing
- **Pros**: Clean separation — non-TUI commands log to stderr, chat routes to log file
- **Cons**: Slightly more complex init logic
- **Effort**: Small
- **Risk**: Low

```rust
// In main.rs, before dispatch:
match &cli.command {
    Some(Commands::Chat) | None => {
        // TUI mode: route to file to avoid corrupting alternate screen
        // Use tracing-appender to ~/.mika/logs/
    }
    _ => {
        mika_common::logging::init_pretty("warn");
    }
}
```

### Option 2: Always init_pretty("warn") for all commands
- **Pros**: Single line, simple
- **Cons**: TUI chat mode will have tracing output mixed with raw terminal rendering (stderr goes to alternate screen)
- **Effort**: Trivial
- **Risk**: Low (tracing output in TUI is ugly but not breaking)

## Recommended Action

Option 2 as an immediate fix, then Option 1 as a follow-up if file-based logging is desired for TUI mode.

## Technical Details

- **Affected files:** `crates/mika-cli/src/main.rs`
- **Components:** All — logging is foundational

## Acceptance Criteria

- [ ] `tracing::warn!` calls in chat.rs and agent internals produce visible output
- [ ] Non-TUI commands (status, memory, etc.) show warnings on stderr
- [ ] TUI mode does not corrupt terminal with tracing output (or routes to file)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | Regression from old cli.rs |

## Resources

- Commit: 399ebf0
- Known pattern: `docs/solutions/architecture-decisions/phase2-axum-http-server-architecture.md`
