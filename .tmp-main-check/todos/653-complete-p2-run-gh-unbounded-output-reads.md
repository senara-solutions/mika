---
status: pending
priority: p2
issue_id: "653"
tags: [code-review, security, performance]
dependencies: []
---

# `run_gh`: Unbounded stdout/stderr reads before truncation

## Problem Statement

`wait_with_output()` reads the entire stdout and stderr into memory before the `truncate_output` function caps the result at 10,000 characters. A command like `["run", "view", "12345", "--log"]` on a large CI run could produce megabytes of output that is fully buffered before truncation.

The executor in `executor.rs` uses `AsyncReadExt::take()` to cap reads at `MAX_OUTPUT_LEN` bytes at the source.

## Findings

- **Security sentinel**: Flagged as medium severity (memory exhaustion / DoS).
- **Performance oracle**: Confirmed the executor uses bounded reads and this handler should follow the same pattern.

## Proposed Solutions

### Solution 1: Use `take()` on stdout/stderr handles (Recommended)
Replace `wait_with_output()` with manual reads using `tokio::io::AsyncReadExt::take()`:
```rust
let mut stdout_buf = Vec::with_capacity(MAX_OUTPUT_LEN);
let mut stderr_buf = Vec::with_capacity(MAX_OUTPUT_LEN);
child.stdout.take().unwrap().take(MAX_OUTPUT_LEN as u64).read_to_end(&mut stdout_buf).await;
// ... similar for stderr
let status = child.wait().await;
```
- **Pros**: Prevents unbounded memory allocation, matches executor pattern
- **Cons**: Slightly more code
- **Effort**: Small
- **Risk**: Low

### Solution 2: Accept current behavior with a comment
The subcommand allowlist excludes `api` (the most likely source of large output). Document why unbounded reads are acceptable given the allowlist.
- **Pros**: No code change
- **Cons**: Leaves the theoretical vulnerability open
- **Effort**: None
- **Risk**: Low (mitigated by allowlist)

## Recommended Action

## Technical Details

- **Affected files**: `crates/mika-agent/src/skills/builtin_handlers.rs` (lines 318-340)
- **Components**: `run_gh` subprocess output handling

## Acceptance Criteria

- [ ] stdout/stderr reads are bounded to `MAX_OUTPUT_LEN` bytes
- [ ] Or: explicit comment explaining why unbounded reads are safe given the allowlist

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Flagged by security sentinel and performance oracle |

## Resources

- `executor.rs` — existing bounded read pattern with `AsyncReadExt::take()`
