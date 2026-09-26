---
status: pending
priority: p2
issue_id: "630"
tags: [code-review, quality, rewind]
dependencies: []
---

# Replace bare 3-tuple return with named ExchangeMatch struct

## Problem Statement

`find_recent_exchanges` returns `Option<(String, i64, Vec<String>)>` — a 3-tuple where field meanings are non-obvious. Every other public result type in `rewind.rs` uses named structs (`RewindPreview`, `RewindResult`, `MessagePreview`, etc.). The bare tuple is inconsistent and error-prone as the codebase evolves.

## Findings

- **Source:** Pattern recognition specialist, Architecture strategist
- **Location:** `crates/mika-agent/src/rewind.rs` line 82, callers in `handlers.rs` line 903
- **Evidence:** The `_trace_ids` discard at callsites indicates low readability. Field order mistakes are possible.

## Proposed Solutions

### Option A: Named struct
```rust
pub struct ExchangeMatch {
    pub session_id: String,
    pub anchor_message_id: i64,
    pub trace_ids: Vec<String>,
}
```
- **Pros:** Self-documenting, consistent with file style, prevents field-order bugs
- **Cons:** One more struct definition
- **Effort:** Small
- **Risk:** None

## Recommended Action

Option A.

## Technical Details

- **Affected files:** `crates/mika-agent/src/rewind.rs`, `crates/mika-cli/src/tui/commands/handlers.rs`

## Acceptance Criteria

- [ ] `ExchangeMatch` struct defined in `rewind.rs`
- [ ] `find_recent_exchanges` returns `Option<ExchangeMatch>`
- [ ] All callers updated to use named fields
- [ ] Tests updated

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | |

## Resources

- Branch: `feat/conversation-rewind`
