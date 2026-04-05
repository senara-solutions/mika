---
status: complete
priority: p1
issue_id: "651"
tags: [code-review, security]
dependencies: []
---

# `run_gh`: Missing `kill_on_drop(true)` and `stdin(Stdio::null())`

## Problem Statement

The spawned `gh` process does not set `kill_on_drop(true)`. If the `run_gh` future is cancelled (e.g., due to per-tool timeout), the `gh` child process will be orphaned and continue running. The executor at `executor.rs:297` sets `kill_on_drop(true)` for non-long-running exec handlers.

Additionally, stdin defaults to inherit. If `gh` ever prompts for input (despite `GH_PROMPT_DISABLED=1`), it would hang reading from the inherited stdin.

## Findings

- **Security sentinel**: Flagged both as one-line fixes with high defensive value.
- **Architecture reviewer**: Noted the executor convention includes `kill_on_drop(true)`.

## Proposed Solutions

### Solution 1: Add both settings (Recommended)
```rust
cmd.kill_on_drop(true);
cmd.stdin(std::process::Stdio::null());
```
- **Pros**: Two one-line fixes, matches executor conventions, prevents orphaned processes and stdin hangs
- **Cons**: None
- **Effort**: Small
- **Risk**: Low

## Recommended Action

## Technical Details

- **Affected files**: `crates/mika-agent/src/skills/builtin_handlers.rs` (around line 309)
- **Components**: `run_gh` builtin handler, process spawning

## Acceptance Criteria

- [ ] `cmd.kill_on_drop(true)` is set before spawning
- [ ] `cmd.stdin(std::process::Stdio::null())` is set before spawning

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Flagged by security sentinel |

## Resources

- `executor.rs:297` — existing pattern for `kill_on_drop`
