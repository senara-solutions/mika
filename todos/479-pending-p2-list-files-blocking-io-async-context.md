---
status: pending
priority: p2
issue_id: "479"
tags: [code-review, performance, security, tools]
dependencies: []
---

# list_files Uses Blocking I/O on Tokio Worker Thread

## Problem Statement

`collect_entries` in `list_files.rs` calls `std::fs::read_dir` (synchronous blocking I/O)
recursively from within `async fn execute`. With `MAX_DEPTH = 10` and `MAX_ENTRIES = 500`,
this can issue hundreds of blocking directory-read syscalls on a tokio worker thread. Tokio's
thread pool has a fixed number of workers (default: number of CPU cores). On a 1–2 CPU VPS
(typical per-customer container), a single blocked worker delays every other concurrent async task
— including the tick loop, HTTP handler, and in-progress agent turns.

## Findings

- **Source**: performance-oracle and security-sentinel reviews
- **Location**: `crates/mika-agent/src/tools/list_files.rs:66` (call site) and `:103` (blocking syscall)
- `collect_entries` is called directly from `async fn execute` at line 66 without `spawn_blocking`
- `std::fs::read_dir(dir)` at line 103 blocks the tokio thread for each directory level
- Up to 500 entries × depth 10 = potentially thousands of blocking syscalls per tool call

## Proposed Solutions

### Option A: Wrap collect_entries in spawn_blocking (Recommended)
```rust
let list_dir_clone = list_dir.clone();
let mut entries = tokio::task::spawn_blocking(move || {
    let mut out = Vec::new();
    collect_entries(&list_dir_clone, &list_dir_clone, &mut out, 0);
    out
}).await?;
```
- **Pros**: Correct async pattern, no structural change to collect_entries
- **Cons**: Slight overhead for spawn_blocking
- **Effort**: Small | **Risk**: None

### Option B: Rewrite using tokio::fs::read_dir (async)
Replace the synchronous recursive traversal with an async iterative approach using
`tokio::fs::read_dir` and a work queue.
- **Pros**: Fully async, more idiomatic
- **Cons**: Larger rewrite, more complex code
- **Effort**: Medium | **Risk**: Low

## Acceptance Criteria

- [ ] `collect_entries` (or equivalent) does not block the tokio worker thread
- [ ] Uses `tokio::task::spawn_blocking` or async fs APIs
- [ ] All existing tests still pass
- [ ] `cargo clippy` passes with no new warnings

## Work Log

- 2026-03-06: Identified by performance-oracle and security-sentinel reviews of feat/unified-task-engine
