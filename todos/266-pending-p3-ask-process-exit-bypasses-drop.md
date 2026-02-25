---
status: pending
priority: p3
issue_id: 266
tags: [code-review, quality, cli]
dependencies: []
---

# `mika ask` uses process::exit(1) bypassing Drop-based cleanup

## Problem Statement

The `mika ask` error handling calls `std::process::exit(1)` which terminates immediately without running destructors. The `AppContext` (which owns `AsyncDatabase` with its dedicated OS thread) never drops cleanly. Every other CLI command propagates errors via `Result<()>` back to `main()`, allowing normal cleanup.

## Findings

- **File**: `crates/mika-cli/src/commands/ask.rs:50-53`
- **Impact**: Low — SQLite handles abrupt close safely, and `ask` is a one-shot command
- **Pattern violation**: Only CLI command that short-circuits with `process::exit(1)` instead of returning `Result<()>`
- **Found by**: security-sentinel, pattern-recognition-specialist, agent-native-reviewer

## Proposed Solutions

### Option A: Move error handling to main.rs (Recommended)
Keep `ask::run()` returning `Result<()>` like all other commands. Handle the user-friendly formatting in `main.rs`:

```rust
// main.rs
Some(Commands::Ask { message }) => {
    match commands::ask::run(&message, &agent_name).await {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
```

- Pros: Consistent with all other commands, Drop runs normally
- Cons: Adds complexity to main.rs
- Effort: Small
- Risk: Low

### Option B: Return Ok(()) after eprintln
```rust
Err(e) => {
    eprintln!("Error: {e}");
    return Ok(());
}
```

- Pros: Simple, Drop runs, no process::exit
- Cons: Returns success exit code (0) even on error — misleading for scripts
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] `mika ask` with invalid API key prints clean error to stderr
- [ ] Process exits with non-zero code on error
- [ ] `AppContext::Drop` runs before exit
- [ ] All other CLI commands still work the same

## Work Log

| Date | Action | Notes |
|------|--------|-------|
| 2026-02-25 | Created | Found during PR #15 review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/15
