---
status: complete
priority: p2
issue_id: 286
tags: [code-review, observability, reliability]
dependencies: []
---

# Add tracing::warn for cross-channel poll errors

## Problem Statement

`poll_cross_channel_messages()` silently discards database errors (`Err(_) => return`). Persistent failures (corrupt DB, disk full) would go completely unnoticed — the user would simply stop seeing cross-channel messages with no indication.

## Findings

- **Security Sentinel:** Cross-channel polling silently swallows errors (Low)
- **Pattern Recognition:** Silent error swallowing in poll

## Proposed Solutions

### Solution A: Add tracing::warn (Recommended)

**File:** `crates/mika-cli/src/tui/app.rs:512`

```rust
Err(e) => {
    tracing::warn!("cross-channel poll failed: {e}");
    return;
}
```

Note: TUI uses alternate screen so tracing output goes to log file, not terminal.

- Effort: Small
- Risk: None

## Acceptance Criteria

- [ ] Poll errors logged at warn level
- [ ] TUI rendering not affected by the log
