---
status: complete
priority: p2
issue_id: "203"
tags: [code-review, performance, tui]
dependencies: []
---

# Markdown Re-Parsed Every Frame for Every Message

## Problem Statement

Every 30ms tick triggers `terminal.draw()` which calls `draw_messages()`, iterating ALL messages and calling `markdown::render()` on each. Each render allocates new `String`, `Vec<Span>`, and `Vec<Line>` objects. At 33 FPS with 50+ messages, this creates thousands of allocations per second that are immediately dropped.

## Findings

- **Source:** performance-oracle (Issue 2.2)
- **Location:** `crates/mika-cli/src/tui/ui.rs:76,99`, `crates/mika-cli/src/tui/markdown.rs`
- **Evidence:** `markdown::render()` called inside message loop, called every frame. No caching. Each call does line iteration + inline parsing + String allocations.
- **Impact:** O(total_message_content * frame_rate) allocations. With 200+ messages in a long session, significant allocator pressure.

## Proposed Solutions

### Option 1: Cache rendered lines in ChatMessage
- **Pros**: Eliminates per-frame re-parsing for all completed messages; only in-flight reveal needs per-frame rendering
- **Cons**: Slightly larger ChatMessage struct
- **Effort**: Small
- **Risk**: Low

```rust
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    pub rendered: Vec<Line<'static>>,  // cached markdown output
}
```

### Option 2: Add dirty flag to skip redundant redraws when idle
- **Pros**: When nothing changes, skip entire draw cycle
- **Cons**: Does not help during active conversation or reveal
- **Effort**: Small
- **Risk**: Low

## Recommended Action

Both options — cache rendered markdown (eliminates O(n) per frame) AND add dirty flag (eliminates unnecessary redraws when idle).

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/app.rs`, `crates/mika-cli/src/tui/ui.rs`

## Acceptance Criteria

- [ ] Completed messages are rendered once, not per-frame
- [ ] Idle TUI does not re-render when nothing changes
- [ ] Progressive reveal still works correctly

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | |

## Resources

- Commit: 399ebf0
