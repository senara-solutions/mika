---
status: complete
priority: p3
issue_id: "314"
tags: [code-review, maintainability]
dependencies: []
---

# Make save_message delegate to save_message_with_metadata

## Problem Statement

`save_message` and `save_message_with_metadata` are two independent INSERT statements. If a future column is added, both need updating separately. The simpler variant should delegate to the richer one.

Identified by: architecture-strategist

## Proposed Solutions

```rust
pub fn save_message(&self, role: &str, content: &str, channel_type: &str) -> Result<i64> {
    self.save_message_with_metadata(role, content, channel_type, None)
}
```

## Technical Details

- **Affected file:** `crates/mika-agent/src/db.rs:416-422`
- Effort: Small (2 lines)

## Acceptance Criteria

- [ ] `save_message` delegates to `save_message_with_metadata`
- [ ] All existing tests pass

## Work Log

- 2026-02-27: Identified during code review of commit 573596b
