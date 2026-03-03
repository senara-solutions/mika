---
status: complete
priority: p3
issue_id: "422"
tags: [code-review, quality, reflection]
dependencies: []
---

# Name the 50K Conversation Digest Magic Number

## Problem Statement

The inline `50_000` in `agent.rs:1093` is a prompt-size safety limit with no name or documentation. A reader wouldn't immediately understand why 50,000 was chosen.

## Proposed Solutions

```rust
/// Maximum characters of conversation digest injected into reflection prompt.
/// ~12,500 tokens at 4 chars/token — keeps total prompt well within Claude's context.
const MAX_REFLECTION_DIGEST_CHARS: usize = 50_000;
```

- **Effort**: Small (1 LOC added)

## Acceptance Criteria

- [ ] Named constant with descriptive comment
- [ ] Used in both conversation and memory events digest caps
