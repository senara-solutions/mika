---
status: complete
priority: p3
issue_id: "512"
tags: [code-review, validation, performance]
dependencies: []
---

# `cancel_task` Tool Missing UUID Length Validation

## Problem Statement

`cancel_task` validates that `id` is non-empty but does not check that it is a valid UUID (≤36 chars). An LLM hallucinating a 10,000-character string as the task ID would send it through the DB channel and execute a full `UPDATE` query against SQLite before returning "not found".

## Findings

- **Source**: performance-oracle (recommendation)
- **Location**: `crates/mika-agent/src/tools/cancel_task.rs:37-41`

```rust
let id = input["id"].as_str().unwrap_or("").trim();
if id.is_empty() {
    return Ok(ToolOutput::error("'id' is required."));
}
let cancelled = ctx.db.cancel_task(id).await?;
```

No length or format check. SQLite's `WHERE id = ?1` will simply match nothing, but a pre-check is free and catches malformed LLM output before touching the DB. Same issue applies to `complete_task` (501) and `get_task` (505) when added.

## Proposed Solutions

### Option A: Add UUID length cap (Recommended)

```rust
if id.len() > 36 {
    return Ok(ToolOutput::error("'id' must be a valid task UUID (36 characters)."));
}
```

- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] `cancel_task` rejects IDs longer than 36 characters with a clear error
- [ ] Existing tests pass
- [ ] New test: excessively long ID returns error without DB call

## Work Log

- 2026-03-06: Identified by performance-oracle review of feat/unified-task-engine
