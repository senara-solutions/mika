---
title: "fix: investigation panel — empty response handling and lock race"
type: fix
status: active
date: 2026-04-13
---

# fix: investigation panel — empty response handling and lock race

## Problem Statement

The investigation panel is broken: tools execute successfully (green checkmarks) but no text response appears. Follow-up questions also produce no response. Server logs confirm the root cause: `qwen/qwen3-coder-next` via OpenRouter returns **0 output tokens** with `stop_reason: EndTurn` after processing tool results. The backend sends `Done` without ever sending a `TextDelta`, leaving the user with no feedback.

Two additional bugs compound the issue:
1. The investigation lock race allows concurrent investigations through a `try_lock()`/`lock()` gap
2. The frontend sends `'...'` placeholder text as history content for follow-up questions

## Evidence

Server logs from 2026-04-12T15:23:

```
15:23:14 → LLM call #1: 80 tokens, stop_reason=ToolUse   (tools dispatched)
15:23:24 → LLM call #2: 37 tokens, stop_reason=ToolUse   (more tools)
15:23:33 → LLM call #3: 0 tokens, stop_reason=EndTurn    ← EMPTY
15:24:15 → "allo ?" follow-up: 0 tokens, stop_reason=EndTurn ← EMPTY again
```

## Proposed Solution

### Bug 1: Empty LLM response (backend)

**File:** `crates/mika-agent/src/server/investigate.rs`

Track whether any `TextDelta` was sent during the investigation. After the loop ends (either by EndTurn, empty tool_uses, or max steps), if no text was emitted, send a fallback `TextDelta`:

```rust
let mut text_sent = false;

// In the text sending block (~line 825):
if !text_parts.is_empty() {
    text_sent = true;
    // ... send TextDelta
}

// After the loop, before the max-steps fallback (~line 910):
if !text_sent {
    let _ = send_event(&tx, InvestigateEvent::TextDelta {
        text: "\n\n[The model did not generate a response after using tools. This can happen with some providers. Try asking again or check the agent's LLM configuration.]".to_string(),
    }).await;
}
```

Also add a `warn!` log when the LLM returns 0 output tokens.

### Bug 2: Investigation lock race (backend)

**File:** `crates/mika-agent/src/server/investigate.rs`

Replace the broken `try_lock()` + spawned `lock()` pattern with `try_lock_owned()` that moves the `OwnedMutexGuard` into the spawned task:

```rust
// Before (broken):
if state.investigation_lock.try_lock().is_err() { return 429; }
// ...
tokio::spawn(async move {
    let _guard = lock.lock().await;  // race window!
    run_investigation(...).await;
});

// After (correct):
let guard = match state.investigation_lock.clone().try_lock_owned() {
    Ok(g) => g,
    Err(_) => return 429,
};
// ...
tokio::spawn(async move {
    let _guard = guard;  // no gap — guard moves directly
    run_investigation(...).await;
});
```

Remove the separate `lock` variable and the `lock.lock().await` in the spawned task.

### Bug 3: History placeholder leak (frontend)

**File:** `dashboard/src/components/InvestigationPanel.tsx`

When sending history for follow-up questions, filter out the `'...'` placeholder:

```typescript
const history = messagesRef.current.map((m) => ({
    role: m.role,
    content: m.content === '...' ? '' : m.content,
}))
```

## Acceptance Criteria

- [ ] When the LLM returns 0 tokens after tool use, the user sees a fallback message explaining the model didn't respond
- [ ] A `warn!` log is emitted when the LLM returns 0 output tokens during investigation
- [ ] Concurrent investigation requests are properly rejected with 429 (no race window)
- [ ] Follow-up questions don't send `'...'` as assistant content in history
- [ ] Existing investigation functionality (text responses, tool badges, error handling) continues to work
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes
