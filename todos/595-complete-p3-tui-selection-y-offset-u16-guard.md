---
status: complete
priority: p3
issue_id: "595"
tags: [code-review, safety, tui]
dependencies: []
---

# Add u16 overflow guard on y_offset casts

## Problem Statement

In `draw_messages`, `y_offset as u16` casts could theoretically overflow if `y_offset` exceeds `u16::MAX` (65535 lines). While extremely unlikely in practice, a `.min(u16::MAX as usize)` guard would prevent undefined rendering.

## Findings

- `crates/mika-cli/src/tui/ui.rs` — `draw_messages` function, `y_offset as u16` cast
- Conversations would need 65K+ wrapped lines to trigger this

## Proposed Solutions

### Solution 1: Add .min() guard (Recommended)
Replace `y_offset as u16` with `y_offset.min(u16::MAX as usize) as u16`.

**Pros:** Defensive, prevents theoretical overflow
**Cons:** Negligible — one extra comparison
**Effort:** Small
**Risk:** Low

## Acceptance Criteria

- [ ] All `y_offset as u16` casts have `.min()` guard
- [ ] Tests pass

## Work Log

| Date | Action |
|------|--------|
| 2026-03-09 | Created from PR #93 code review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/93
