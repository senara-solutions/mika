---
status: pending
priority: p3
issue_id: "325"
tags: [code-review, quality, ux]
dependencies: []
---

# "med" Alias Accepted by /think but Rejected by /config set

## Problem Statement

`resolve_thinking_level` in `commands/mod.rs` accepts `"med"` as an alias for
`"medium"`, but `validate_config_value` in `config_keys.rs` only accepts
`"low" | "medium" | "high" | "off"`.

This means `/think med` works in the TUI, but `/config set thinking_level med`
fails with "Invalid thinking_level".

Not a bug — the DB always stores the canonical `"medium"` form (the `&'static str`
from `resolve_thinking_level`). But it's a UX inconsistency.

## Findings

- **Security reviewer:** "Purely cosmetic. `/think med` works but `/config set thinking_level med` would fail."
- **Simplicity reviewer:** "Consider adding `"med"` to the config validator, or having the validator call `resolve_thinking_level` instead of maintaining a parallel match list."

## Proposed Solutions

### Option A: Add "med" to config_keys validation (Simplest)

```rust
"thinking_level" => {
    if !matches!(value, "low" | "medium" | "med" | "high" | "off") {
        return Err(...);
    }
}
```

- **Pros:** Simple, consistent UX
- **Cons:** Parallel lists still exist
- **Effort:** Trivial
- **Risk:** None

### Option B: Leave as-is

The `/config set` path is rarely used for thinking_level (most users use `/think`).

- **Pros:** No change
- **Cons:** Inconsistent UX
- **Effort:** None
- **Risk:** None

## Technical Details

- **File:** `crates/mika-agent/src/config_keys.rs` line 34
- **Related:** `crates/mika-cli/src/tui/commands/mod.rs` `resolve_thinking_level`

## Acceptance Criteria

- [ ] `/config set thinking_level med` works (if Option A chosen)
- [ ] Stored value is canonical `"medium"` regardless of input alias

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from code review | Aliases should be accepted consistently across all input paths |
