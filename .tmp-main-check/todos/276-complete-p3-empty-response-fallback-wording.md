---
status: complete
priority: p3
issue_id: 276
tags: [code-review, ux, consistency]
dependencies: []
---

# Unify empty response fallback wording across CLI modes

## Problem Statement

The fallback message for tool-only agent responses differs across modes:
- TUI: `"Agent processed your request."`
- CLI ask: `"(Agent processed your request — no text response)"`
- Server: `"Done."`

## Findings

- **Files**: `crates/mika-cli/src/tui/app.rs:224`, `crates/mika-cli/src/commands/ask.rs:51`, `crates/mika-agent/src/server/handlers.rs:149`
- **Impact**: Low — minor UX inconsistency
- **Found by**: agent-native-reviewer

## Proposed Solution

Extract a shared constant in `mika-agent`:

```rust
pub const EMPTY_RESPONSE_FALLBACK: &str = "Done.";
```

Reference from all three consumer sites. The server wording "Done." is the most conversational for a chat context.

## Acceptance Criteria

- [ ] Single constant for the fallback message
- [ ] All three modes use the same wording
