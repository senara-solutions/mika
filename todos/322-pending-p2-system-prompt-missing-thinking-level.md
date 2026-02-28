---
status: pending
priority: p2
issue_id: "322"
tags: [code-review, agent-native, prompt]
dependencies: ["321"]
---

# System Prompt Does Not Mention thinking_level as Settable Config Key

## Problem Statement

The system prompt at `crates/mika-agent/src/prompt.rs:237` says:

> "You can read and update customer config (timezone, chat_id) with get_config and set_config."

This parenthetical list was not updated when `thinking_level` was added to the
config key allowlist. The agent has no context about the key's existence, valid
values, or purpose.

## Findings

- **Agent-native reviewer:** "Even if the schema enum is fixed, the agent has no context about what `thinking_level` means. This is a Context Starvation anti-pattern."

## Proposed Solutions

### Option A: Update parenthetical list (Recommended)

```rust
"- You can read and update customer config (timezone, chat_id, thinking_level) with get_config and set_config.\n",
```

- **Pros:** Simple, consistent with existing pattern
- **Cons:** None
- **Effort:** Trivial
- **Risk:** None

### Option B: Add detailed guidance

Include a sentence explaining valid values:

```rust
"- You can read and update customer config (timezone, chat_id, thinking_level) with get_config and set_config. thinking_level controls extended thinking (values: low, medium, high, off).\n",
```

- **Pros:** More informative for the agent
- **Cons:** Slightly longer prompt
- **Effort:** Trivial
- **Risk:** None

## Recommended Action

Option A is sufficient. The tool schema description (once fixed in #321) provides value details.

## Technical Details

- **File:** `crates/mika-agent/src/prompt.rs` line 237

## Acceptance Criteria

- [ ] System prompt mentions `thinking_level` alongside other config keys
- [ ] Agent can reason about setting thinking_level when asked

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from code review of think persistence feature | System prompt lists must be updated when adding config keys |

## Resources

- File: `crates/mika-agent/src/prompt.rs`
