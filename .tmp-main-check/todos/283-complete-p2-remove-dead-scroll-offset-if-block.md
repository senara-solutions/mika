---
status: complete
priority: p2
issue_id: 283
tags: [code-review, quality, dead-code]
dependencies: []
---

# Remove dead if-block for scroll_offset in poll method

## Problem Statement

In `poll_cross_channel_messages()`, there is an empty `if` block that compiles to nothing but reads as if it should do something:

```rust
if self.scroll_offset == 0 {
    // Already at bottom, stay there
}
self.needs_redraw = true;
```

The `needs_redraw = true` fires unconditionally regardless of scroll position, contradicting the comment.

## Findings

- **Code Simplicity Reviewer:** Dead code, empty body, misleading
- **Architecture Strategist:** Empty `if` block for scroll offset — no-op
- **Pattern Recognition:** Empty if-block should be replaced with inline comment

## Proposed Solutions

### Solution A: Replace with comment (Recommended)

**File:** `crates/mika-cli/src/tui/app.rs:537-541`

Replace the empty if-block with a clear comment:

```rust
// Auto-scroll: stays at bottom if scroll_offset == 0; preserves position otherwise.
self.needs_redraw = true;
```

- Effort: Small
- Risk: None

## Acceptance Criteria

- [ ] Empty `if self.scroll_offset == 0 {}` block removed
- [ ] Comment clarifies scroll behavior
- [ ] `cargo test` passes
