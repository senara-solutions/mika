---
status: complete
priority: p2
issue_id: "552"
tags: [code-review, security, parity]
dependencies: []
---

# No size limit on callback result in CLI `ask.rs` path

## Problem Statement

The server-side handler at `handlers.rs:313` enforces a 100KB limit on callback results. The CLI `mika ask --task-id` path has no equivalent guard. A subprocess returning gigabytes of output via `mika ask --task-id <uuid> -` (stdin) would read it all into memory and pass the entire string to the LLM system prompt, causing memory exhaustion or unexpectedly large API calls.

## Findings

- **Source:** Security sentinel agent
- **Location:** `crates/mika-cli/src/commands/ask.rs:22-28` — `read_to_string` with no size limit
- **Comparison:** Server enforces 100KB at `crates/mika-agent/src/server/handlers.rs:313`

## Proposed Solutions

### Solution A: Add size check after reading stdin when task-id is present (Recommended)

```rust
const MAX_CALLBACK_RESULT: usize = 100_000;
if task_id.is_some() && user_message.len() > MAX_CALLBACK_RESULT {
    anyhow::bail!(
        "Callback result too large: {} bytes (max: {})",
        user_message.len(), MAX_CALLBACK_RESULT
    );
}
```

- **Pros:** Simple, consistent with server limit
- **Cons:** None
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] CLI callback result limited to 100KB, matching server behavior
- [ ] Clear error message when limit is exceeded

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | Parity gap between server and CLI callback paths |
