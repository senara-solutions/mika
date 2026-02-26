---
status: pending
priority: p2
issue_id: 287
tags: [code-review, security, validation]
dependencies: []
---

# Add length limit for config values

## Problem Statement

The `/config set` command does not enforce any length limit on config values. The project convention is "empty check + 10,000 char max" for tool inputs, but this is not applied to config values.

## Findings

- **Security Sentinel:** Config value length unbounded (Low)

## Proposed Solutions

### Solution A: Add 1000-char limit (Recommended)

**File:** `crates/mika-cli/src/tui/commands/handlers.rs:305` (before key-specific validation)

```rust
if value.len() > 1000 {
    return "Config value too long (max 1000 characters)".to_string();
}
```

- Effort: Small
- Risk: None

## Acceptance Criteria

- [ ] Values over 1000 chars rejected with clear error
- [ ] Normal values accepted
