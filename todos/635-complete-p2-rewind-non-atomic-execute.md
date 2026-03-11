---
status: complete
priority: p2
issue_id: "635"
tags: [code-review, architecture, rewind]
dependencies: []
---

# Non-atomic execute_rewind — multiple sequential DB operations without transaction

## Problem Statement

`execute_rewind()` performs multiple sequential DB operations (delete messages, reverse audit events, delete rewind markers, save marker message) without transaction wrapping. A crash mid-execution could leave the database in an inconsistent state (e.g., messages deleted but no marker injected, or partial reversals applied).

## Findings

- **Source:** Architecture review agent
- **Location:** `crates/mika-agent/src/rewind.rs` — `execute_rewind()` function
- Each DB call is independent: `delete_messages_after_id`, `reverse_audit_event`, `delete_rewind_markers`, `save_message`
- SQLite supports nested transactions via savepoints
- The `AsyncDatabase` closure-dispatch pattern makes multi-operation transactions non-trivial (each closure gets its own `&Connection`)

## Proposed Solutions

### Option A: Transaction wrapper in Database (sync layer)
Add a `with_transaction` method to `Database` that takes a closure receiving `&Transaction`, and expose it through `AsyncDatabase`.
- **Pros:** Proper atomicity, crash-safe
- **Effort:** Medium — requires new `AsyncDatabase` method and refactoring execute_rewind to use it
- **Risk:** Low

### Option B: Accept current behavior with documentation
Document that partial failure is possible but recoverable (rewind can be retried, marker re-injected).
- **Pros:** No code changes
- **Effort:** Small
- **Risk:** Low — SQLite single-writer serialization already prevents concurrent corruption

## Acceptance Criteria

- [ ] Either: `execute_rewind` operations are wrapped in a transaction, OR: documented as acceptable with recovery path
