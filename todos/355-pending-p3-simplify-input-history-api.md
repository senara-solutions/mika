---
status: pending
priority: p3
issue_id: "355"
tags: [code-review, quality, tui]
dependencies: []
---

# Simplify InputHistory API

## Problem Statement

The `InputHistory` struct has a few areas that could be simplified:
1. `HistoryNavResult` enum (Entry/Draft/None) could be flattened to `Option<String>` since callers don't distinguish between Entry and Draft
2. `max_size` is a runtime field but is never configured — could be a constant
3. `is_browsing()` method exists but is minimally used

## Findings

- **Code Simplicity Reviewer**: `HistoryNavResult` has three variants but callers treat Entry and Draft the same way (both replace textarea content). Flattening to `Option<String>` would simplify the API.
- **Code Simplicity Reviewer**: `max_size` field is always set to 500 via `InputHistory::new()`. Making it a `const MAX_SIZE: usize = 500` removes a field and clarifies intent.
- **Source**: PR #33 — `crates/mika-cli/src/tui/app.rs` lines 77-160

## Proposed Solutions

### Solution A: Flatten HistoryNavResult + const max_size
Replace `HistoryNavResult` with `Option<String>`, change `max_size` to a constant, and inline `is_browsing()` at call sites.

- **Pros**: Simpler API, fewer types, clearer intent
- **Cons**: Loses semantic distinction between "restored draft" and "history entry" (currently unused but could be useful for UI differentiation)
- **Effort**: Small
- **Risk**: Low

### Solution B: Keep current design
The current API works correctly and is well-tested with 14 unit tests. The semantic richness of HistoryNavResult may be useful for future features (e.g., styling draft differently).

- **Pros**: No code churn, preserves future flexibility
- **Cons**: Slightly more complex than necessary for current needs
- **Effort**: None
- **Risk**: None

## Recommended Action

_To be filled during triage_

## Technical Details

**Affected files:**
- `crates/mika-cli/src/tui/app.rs` — `InputHistory`, `HistoryNavResult`

## Acceptance Criteria

- [ ] If simplified: `HistoryNavResult` removed, `next()` returns `Option<String>`
- [ ] If simplified: `max_size` field replaced with `const MAX_SIZE`
- [ ] All 14 InputHistory tests updated and passing

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from PR #33 code review | Simplicity reviewer flagged overengineering |

## Resources

- PR #33: https://github.com/senara-solutions/mika/pull/33
