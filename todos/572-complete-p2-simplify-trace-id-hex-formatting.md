---
status: complete
priority: p2
issue_id: "572"
tags: [code-review, simplicity, trace-id]
dependencies: []
---

# Simplify trace_id hex formatting in trace.rs

## Problem Statement

The fallback trace_id generation uses 8 lines of manual byte-to-hex folding when `uuid::Uuid::simple()` produces the exact same 32-char lowercase hex output in one line.

## Findings

- **Source:** Code Simplicity Reviewer
- **File:** `crates/mika-agent/src/trace.rs:22-29`
- **Current code:**
  ```rust
  let id = uuid::Uuid::new_v4();
  id.as_bytes()
      .iter()
      .fold(String::with_capacity(32), |mut s, b| {
          use std::fmt::Write;
          let _ = write!(s, "{b:02x}");
          s
      })
  ```
- **Proposed:** `uuid::Uuid::new_v4().simple().to_string()`

## Proposed Solutions

### Option A: Use Uuid::simple() (Recommended)
Replace 8 lines with `uuid::Uuid::new_v4().simple().to_string()`.

- **Effort:** Small (2 min)
- **Risk:** None — produces identical output

## Acceptance Criteria

- [ ] `generate_trace_id()` returns same format (32-char lowercase hex)
- [ ] Existing tests pass unchanged

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from PR #88 code review | uuid crate has built-in simple format |
