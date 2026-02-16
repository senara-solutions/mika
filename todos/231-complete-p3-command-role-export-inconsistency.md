---
status: complete
priority: p3
issue_id: 231
tags: [code-review, simplicity, slash-commands]
dependencies: []
---

# ChatRole::Command Export Inconsistency

## Problem Statement

`ChatRole::Command` messages are stored in `app.messages` but explicitly skipped during `/export`. This creates an inconsistency — they're visible in the UI but invisible in exports. The role's purpose and lifecycle aren't clearly documented.

## Findings

**Source:** Code Simplicity Reviewer

**Location:** `crates/mika-cli/src/tui/commands/handlers.rs:294-297` (`handle_export`)

```rust
ChatRole::Command => {
    // Skip command output in export
}
```

## Proposed Solutions

### Solution A: Include command output in exports with a marker (Recommended)
- Export command messages with a "Command:" prefix or in a distinct format
- **Pros:** Exports are complete, no hidden information
- **Cons:** Exports include transient info like `/status` output
- **Effort:** Small
- **Risk:** None

### Solution B: Keep current behavior, add documentation
- The skip is intentional — command output is transient/ephemeral
- Add a doc comment explaining why
- **Pros:** No behavior change, clear intent
- **Cons:** Exports may seem incomplete
- **Effort:** Small
- **Risk:** None

## Recommended Action

Solution B — add a doc comment. Command output IS ephemeral; excluding it from exports is reasonable.

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/commands/handlers.rs`

## Acceptance Criteria

- [ ] Decision documented (include or skip command output in exports)
- [ ] Doc comment added if keeping current behavior

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from code review | Simplicity reviewer flagged inconsistency |

## Resources

- PR branch: `feat/slash-commands`
