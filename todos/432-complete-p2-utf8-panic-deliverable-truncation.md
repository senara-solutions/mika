---
status: pending
priority: p2
issue_id: "432"
tags: [code-review, correctness, utf8]
dependencies: []
---

# UTF-8 Panic in Deliverable Truncation

## Problem Statement

In `crates/mika-agent/src/teams/prompt.rs`, the deliverable truncation uses byte indexing:

```rust
let truncated = if d.len() > 500 { &d[..500] } else { d };
```

If the deliverable contains multi-byte UTF-8 characters (CJK, emoji) and byte 500 falls within a multi-byte sequence, this will panic at runtime. The `floor_char_boundary` method is already used correctly elsewhere in this PR (e.g., `get_team_status.rs`).

## Findings

- **Security agent** and **Performance agent** both flagged this independently
- The rest of the codebase uses `floor_char_boundary()` for safe truncation (e.g., `get_team_status.rs:139`)
- This is a runtime crash waiting to happen with non-ASCII deliverables

## Proposed Solutions

### Option A: Use `floor_char_boundary` (Recommended)

```rust
let truncated = if d.len() > 500 { &d[..d.floor_char_boundary(500)] } else { d };
```

- **Pros:** One-line fix, consistent with codebase pattern
- **Cons:** None
- **Effort:** Small
- **Risk:** None

## Technical Details

- **File:** `crates/mika-agent/src/teams/prompt.rs`, line 40
- **Components:** Team orchestrator context builder

## Acceptance Criteria

- [ ] `&d[..500]` replaced with `&d[..d.floor_char_boundary(500)]`
- [ ] Test with multi-byte characters near the 500-byte boundary
