---
status: complete
priority: p2
issue_id: 284
tags: [code-review, quality, duplication]
dependencies: []
---

# Extract channel_prefix_span() helper in ui.rs

## Problem Statement

The channel prefix rendering logic (`if let Some(ref ch) = msg.channel { spans.push(...) }`) is duplicated identically in the User and Assistant rendering arms of `ui.rs`. The plan called for a `channel_prefix_span()` helper but it was not extracted.

## Findings

- **Code Simplicity Reviewer:** Identical 5-line block duplicated in User and Assistant rendering
- **Pattern Recognition:** Channel prefix rendering duplicated between roles

## Proposed Solutions

### Solution A: Extract helper function (Recommended)

**File:** `crates/mika-cli/src/tui/ui.rs`

```rust
fn channel_prefix_span(channel: &Option<String>) -> Option<Span<'static>> {
    channel.as_ref().map(|ch| {
        Span::styled(format!("[{ch}] "), Style::default().fg(Color::Yellow))
    })
}
```

Then in each arm: `if let Some(span) = channel_prefix_span(&msg.channel) { spans.push(span); }`

- Effort: Small
- Risk: None

## Acceptance Criteria

- [ ] Helper function extracted
- [ ] Both User and Assistant arms use the helper
- [ ] Visual rendering unchanged
