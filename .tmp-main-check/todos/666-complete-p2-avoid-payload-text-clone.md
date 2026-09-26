---
status: pending
priority: p2
issue_id: 666
tags: [code-review, performance, gateway]
dependencies: []
---

# Avoid Unnecessary payload.text.clone() in handle_send

## Problem Statement

In `handle_send`, when `agent_name` is `None` (the common single-agent case), `payload.text.clone()` copies up to 50KB unnecessarily. The cloned string is only passed by reference to `send_message(&formatted_text)`.

## Findings

- `crates/mika-gateway/src/routes.rs:614-615` — `None => payload.text.clone()` allocates unnecessarily

Identified by: performance-oracle, code-simplicity-reviewer, pattern-recognition-specialist

## Proposed Solutions

Use a reference instead of cloning:

```rust
let owned_text;
let text_to_send = match &payload.agent_name {
    Some(name) => {
        owned_text = format!("[{name}] {}", payload.text);
        &owned_text
    }
    None => &payload.text,
};
```

- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] No allocation when `agent_name` is None
- [ ] Messages still formatted correctly when agent_name is present
