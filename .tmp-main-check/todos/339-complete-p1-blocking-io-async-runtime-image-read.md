---
status: complete
priority: p1
issue_id: "339"
tags: [code-review, performance, multimodal-tool-results]
dependencies: []
---

# Blocking I/O on Async Runtime in read_and_validate_image

## Problem Statement

`read_and_validate_image()` in `crates/mika-agent/src/skills/executor.rs` performs synchronous filesystem operations (`std::fs::metadata`, `std::fs::read`, `std::fs::canonicalize`) directly on the tokio async runtime. For large image files (up to 5MB), `std::fs::read` can block the runtime thread, starving other async tasks.

This is especially problematic because the function is called from `execute_exec()` which runs inside the async agent loop.

## Findings

- **Source:** performance-oracle review agent
- **Severity:** CRITICAL — can block the tokio runtime thread
- **Location:** `crates/mika-agent/src/skills/executor.rs` — `read_and_validate_image()` function
- **Evidence:** `std::fs::read(&path)` for files up to 5MB, `std::fs::metadata(&path)`, `std::fs::canonicalize(path)` — all synchronous blocking calls

## Proposed Solutions

### Solution A: Use tokio::spawn_blocking (Recommended)

Wrap the entire `read_and_validate_image` function body in `tokio::spawn_blocking` to move file I/O to a dedicated blocking thread pool.

- **Pros:** Minimal code change, idiomatic tokio pattern, proven approach (already used for AsyncDatabase in this codebase)
- **Cons:** Adds `.await` to the call site, minor overhead from thread pool scheduling
- **Effort:** Small
- **Risk:** Low

### Solution B: Use tokio::fs

Replace `std::fs` calls with `tokio::fs` equivalents.

- **Pros:** Fully async, no thread pool overhead
- **Cons:** More invasive changes, tokio::fs still uses spawn_blocking internally for most operations
- **Effort:** Medium
- **Risk:** Low

## Recommended Action

Solution A — wrap in `spawn_blocking`

## Technical Details

- **Affected files:** `crates/mika-agent/src/skills/executor.rs`
- **Components:** `read_and_validate_image()`, `process_envelope_images()`

## Acceptance Criteria

- [ ] `read_and_validate_image()` does not block the async runtime
- [ ] All existing tests pass
- [ ] `cargo clippy` clean

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from code review | Identified by performance-oracle agent |

## Resources

- PR branch: `feat/multimodal-tool-results`
- Similar pattern: `AsyncDatabase` in `crates/mika-agent/src/db.rs` uses dedicated OS thread for blocking SQLite ops
