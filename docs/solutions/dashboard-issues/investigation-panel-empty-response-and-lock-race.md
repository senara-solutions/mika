---
title: "Investigation panel — empty LLM response and lock race"
category: dashboard-issues
date: 2026-04-13
tags: [investigation-panel, sse, llm-provider, concurrency, openrouter, qwen]
---

# Investigation Panel — Empty LLM Response and Lock Race

## Problem

The investigation panel showed tool badges (query_messages ✓, query_audit_events ✓) but no text response. Follow-up questions also produced no response. The panel appeared completely broken.

## Root Cause

**Three independent bugs:**

1. **Empty LLM response (primary):** The default agent's LLM (`qwen/qwen3-coder-next` via OpenRouter) returned **0 output tokens** with `stop_reason: EndTurn` after processing tool results. Server logs confirmed:
   ```
   llm_call completed  input_tokens=7887  output_tokens=0  stop_reason=EndTurn
   ```
   The backend sent `Done` without ever sending a `TextDelta`, leaving the frontend with tool badges but no answer. This is a provider-specific behavior — some models return empty content after tool results.

2. **Investigation lock race:** The handler used `try_lock()` (which acquires and immediately drops the guard) then the spawned task used `lock().await` to re-acquire. Between these two, concurrent requests could slip through. Server logs showed duplicate "starting investigation" entries at the same timestamp confirming the race (though these turned out to be duplicate logging from stdout + log file both going to the same path).

3. **History placeholder leak:** When tools ran but no text arrived, the frontend set assistant content to `'...'` as a placeholder. Follow-up questions sent this `'...'` as the assistant's response in the conversation history, confusing the LLM.

## Solution

### Empty response fallback (investigate.rs)

Track whether any `TextDelta` was sent during the investigation loop. When the loop exits without sending text (EndTurn with 0 tokens), send a user-visible fallback message:

```rust
let mut text_sent = false;

// In text sending block:
if !full_text.is_empty() {
    text_sent = true;
    send_event(&tx, InvestigateEvent::TextDelta { text: full_text }).await;
}

// At EndTurn exit:
if !text_sent {
    send_event(&tx, InvestigateEvent::TextDelta {
        text: "[The model did not generate a response...]".to_string(),
    }).await;
}
```

Also added `warn!` log when `response.content.is_empty()` for diagnostics.

### Lock race fix (investigate.rs)

Replaced `try_lock()` + spawned `lock()` with `try_lock_owned()` that moves the `OwnedMutexGuard` into the spawned task with no gap:

```rust
// Before (broken — guard drops immediately):
if state.investigation_lock.try_lock().is_err() { return 429; }
tokio::spawn(async move { let _guard = lock.lock().await; ... });

// After (correct — guard transfers directly):
let guard = state.investigation_lock.clone().try_lock_owned()?;
tokio::spawn(async move { let _guard = guard; ... });
```

### History placeholder (InvestigationPanel.tsx)

Filter `'...'` from history content before sending follow-up requests:

```typescript
content: m.content === '...' ? '' : m.content,
```

## Prevention

- **Empty response handling:** Any agent loop that calls an LLM after tool execution should check for 0-token responses. The main agent loop already has `EMPTY_RESPONSE_FALLBACK` — the investigation loop was missing this pattern.
- **Lock transfer pattern:** When a handler spawns an async task that needs exclusive access, always use `try_lock_owned()` to move the guard — never `try_lock()` followed by `lock()` in the spawn.
- **Provider testing:** The investigation panel uses the default agent's LLM, which may be a non-Anthropic model. Test investigation with the actual configured provider, not just Claude.

## Diagnostic checklist

When the investigation panel shows no response:

1. Check server logs for `output_tokens: 0` on investigation LLM calls
2. Check which model/provider the default agent uses
3. Look for the new `warn!` log: "investigation LLM returned empty response"
4. Check for duplicate "starting investigation" logs (lock race symptom)
