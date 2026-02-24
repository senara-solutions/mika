---
status: pending
priority: p2
issue_id: "062"
tags: [code-review, architecture, quality, rust-v2]
dependencies: []
---

# list_preferences() Should Return Named Struct Like Other List Methods

## Problem Statement

Every other list method in `db.rs` returns a named struct (`Vec<Person>`, `Vec<Commitment>`, `Vec<CoreMemoryEntry>`, `Vec<MemoryEvent>`). `list_preferences()` is the only one returning `Vec<(String, String)>`. This breaks the established pattern, makes field access positional instead of named, and prevents adding fields (e.g. `updated_at`) without a breaking signature change.

**Location:** `crates/mika-agent/src/db.rs` — `list_preferences()`

**Reported by:** architecture-strategist

## Proposed Solutions

### Option A: Add Preference struct (Recommended)

```rust
#[derive(Debug, Clone)]
pub struct Preference {
    pub category: String,
    pub value: String,
    pub updated_at: String,
}
```

Update `list_preferences()` to return `Vec<Preference>` and update the call site in `search_memory.rs`.

- **Pros:** Consistent with all other list methods, forward-compatible, self-documenting
- **Cons:** ~10 lines added
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] `Preference` struct exists in `db.rs`
- [ ] `list_preferences()` returns `Vec<Preference>`
- [ ] `search_memory.rs` uses `pref.category` / `pref.value` instead of tuple destructuring
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from P1-fix review | Every other entity has a named struct; Preference is the outlier |
