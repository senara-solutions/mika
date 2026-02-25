---
status: complete
priority: p3
issue_id: 267
tags: [code-review, quality, cli]
dependencies: []
---

# Agent-switch error paths still use {e:#} full chain format

## Problem Statement

The agent-switch error paths in `chat.rs` (lines 237 and 246) still use `format!("Failed to switch agent: {e:#}")` which concatenates the full anyhow error chain. This is a pre-existing inconsistency not introduced by PR #15, but it means agent-switch errors may display raw internal details in the TUI.

## Findings

- **File**: `crates/mika-cli/src/commands/chat.rs:237` and `:246`
- **Pattern**: `format!("Failed to switch agent: {e:#}")`
- **Impact**: Low — agent-switch errors are config/DB errors, not API errors, but the full chain format is still harder to read
- **Found by**: pattern-recognition-specialist

## Proposed Solutions

### Option A: Change to {e} (Recommended)
```rust
content: format!("Failed to switch agent: {e}"),
```

- Pros: Consistent with PR #15's approach
- Cons: May lose useful nested context for config/DB errors
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] Agent-switch errors display cleanly in TUI
- [ ] User can understand what went wrong from the error message

## Work Log

| Date | Action | Notes |
|------|--------|-------|
| 2026-02-25 | Created | Found during PR #15 review (pre-existing) |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/15
