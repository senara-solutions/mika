---
status: pending
priority: p3
issue_id: 288
tags: [code-review, quality, maintainability]
dependencies: []
---

# Centralize hardcoded channel string constants

## Problem Statement

The string `"telegram"` appears as hardcoded literals in 3 files. When WhatsApp support is added, every call site must be updated. The project already has `VALID_CHANNELS` in `prompt.rs`.

## Findings

- **Architecture Strategist:** Hardcoded channel lists in 3 locations
- **Code Simplicity Reviewer:** Centralize the `"telegram"` string literal
- **Pattern Recognition:** Hardcoded channel names scattered across call sites

## Proposed Solutions

### Solution A: Define module-level constants

Add to `crates/mika-cli/src/tui/app.rs`:

```rust
/// Non-CLI channels to poll for cross-channel messages.
const POLLED_CHANNELS: &[&str] = &["telegram"];
```

Reference from `chat.rs` and `app.rs`. Keep `"cli"` inline since it's the local default.

- Effort: Small
- Risk: None

## Acceptance Criteria

- [ ] Channel names defined as constants
- [ ] No hardcoded "telegram" strings in business logic
