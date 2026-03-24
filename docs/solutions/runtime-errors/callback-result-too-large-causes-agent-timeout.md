---
title: Callback result too large causes agent timeout
category: runtime-errors
date: 2026-03-24
issue: 259
tags: [callback, truncation, timeout, silent-agent, prompt-size]
severity: high
components: [agent-loop, format_callback_framing, silent-agent]
---

# Callback result too large causes agent timeout

## Problem

After a large claude-pilot run (226 turns, ~90KB callback result), the silent agent timed out during prompt assembly. `format_callback_framing()` injected the full 90KB result into the system prompt. The serialization/setup phase consumed the entire 5-minute `AGENT_TOTAL_TIMEOUT_SECS` (300s). The agent never reached the LLM API call — zero tool calls, work item left unprocessed.

**Symptom:** Task shows `delivered` but the agent produced the fallback message "I'm sorry, that took too long." with zero tool calls.

**Evidence:** Task `3a27a03f` (mika#257): callback delivered at 14:00:30, agent timed out at 14:05:30.

## Root Cause

`format_callback_framing()` in `crates/mika-agent/src/agent.rs` had no size cap on the `result` parameter. The API/tool layer accepts up to 100KB (`MAX_RESULT_LEN` in `complete_task.rs`, 100KB cap in `handlers.rs`), so a handler could legitimately send a 90KB result that would be injected verbatim into the system prompt.

The flow: handler → `/tasks/{id}/complete` (100KB cap) → task engine → `SilentTrigger::Callback` → `format_callback_framing(result)` → system prompt → LLM API call timeout.

## Solution

Added `CALLBACK_RESULT_MAX_BYTES = 10_240` (10KB) constant and UTF-8-safe truncation in `format_callback_framing()`:

```rust
const CALLBACK_RESULT_MAX_BYTES: usize = 10_240;

// Inside format_callback_framing():
const TRUNCATION_SUFFIX: &str = "\n...\n[truncated — full result available in task logs]";
let result = if result.len() > CALLBACK_RESULT_MAX_BYTES {
    warn!(
        original_bytes = result.len(),
        truncated_to = CALLBACK_RESULT_MAX_BYTES,
        "callback result truncated before prompt injection"
    );
    let cut = CALLBACK_RESULT_MAX_BYTES.saturating_sub(TRUNCATION_SUFFIX.len());
    let mut boundary = cut;
    while boundary > 0 && !result.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}{}", &result[..boundary], TRUNCATION_SUFFIX)
} else {
    result.to_string()
};
```

Key design decisions:
- **10KB cap chosen** because it's enough context for the LLM to summarize results, while staying well within prompt budget. Full results are always in task logs.
- **Truncation at prompt injection, not API layer** — the 100KB API cap remains unchanged. Persisted task results stay complete for dashboard/audit. Only the system prompt gets the truncated version.
- **UTF-8-safe** — walks back to a valid char boundary before slicing, preventing panics on multi-byte input (learned from `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md`).
- **warn! on truncation** — follows project convention of logging safety-net activations (per `docs/solutions/logic-errors/tool-calls-metadata-tail-drop-loses-entries.md`).

## Prevention

1. **Always cap injected data before prompt construction** — this is already documented in `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` (architectural invariant #4). The truncation enforces it.
2. **Reduce handler-side output** — the companion fix in `mika-skills/claude-pilot/handlers/run.sh` should reduce log tail from ~90KB to 10KB. Belt-and-suspenders: Rust-side cap protects against any handler.
3. **Log safety-net truncations** — the `warn!` makes it visible when truncation activates, so operators can tune handler output sizes proactively.

## Related

- Issue: [#259](https://github.com/senara-solutions/mika/issues/259)
- UTF-8 slicing lesson: `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md`
- Callback architecture: `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`
- Truncation conventions: `docs/solutions/logic-errors/tool-calls-metadata-tail-drop-loses-entries.md`
- Companion change needed: `mika-skills/claude-pilot/handlers/run.sh` (reduce log tail to 10KB)
