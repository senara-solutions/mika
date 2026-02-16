---
status: complete
priority: p2
issue_id: "244"
tags: [code-review, robustness, migration]
dependencies: []
---

# Migration rename errors not fault-tolerant for concurrent runs

## Problem Statement

`migrate_to_multi_agent` performs 9 sequential `fs::rename` calls. If two processes (CLI + server) run migration simultaneously, the second may fail when `fs::rename` finds the source already moved. Each rename is individually safe (atomic on same FS), but the sequence is not transactional.

## Findings

- **Source:** Security Sentinel, Performance Oracle
- **File:** `crates/mika-common/src/home.rs:62-109`

## Proposed Solutions

### Option A: Make rename errors non-fatal for NotFound [Recommended]

```rust
match std::fs::rename(&src, &dst) {
    Ok(()) => {}
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
    Err(e) => return Err(e).with_context(|| ...),
}
```

- **Pros:** Simple, handles concurrent migrations gracefully
- **Cons:** None
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Rename errors for NotFound sources are silently skipped
- [ ] Concurrent migrations don't fail with errors

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | TOCTOU window between layout check and rename |
