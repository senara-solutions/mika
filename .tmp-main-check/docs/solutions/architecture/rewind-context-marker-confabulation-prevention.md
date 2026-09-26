---
title: Rewind Context Marker to Prevent Agent Confabulation
problem_type: architecture
component: rewind, agent-loop, db
severity: high
tags:
  - rewind
  - confabulation
  - context-awareness
  - system-messages
  - cross-session
  - prompt-injection
symptoms:
  - Agent fabricates explanations for context gaps after rewind
  - Agent narrates events from wrong project/session after cross-session rewind
  - Agent gives confident but incorrect approvals based on incomplete context
  - Rapid successive rewinds hollow out agent's situational awareness
related_modules:
  - crates/mika-agent/src/rewind.rs
  - crates/mika-agent/src/agent.rs
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/async_db.rs
  - crates/mika-cli/src/tui/commands/handlers.rs
  - crates/mika-agent/src/server/rewind.rs
---

# Rewind Context Marker to Prevent Agent Confabulation

## Problem

After `execute_rewind()` deletes messages and reverses memory mutations, the agent sees a gap in its conversation history with no explanation. The agent pattern-matches from incomplete context and confabulates — fabricating stories to fill gaps, narrating events from the wrong project, and giving confident but wrong approvals.

**Production incident:** 5 rapid cross-session rewinds in 9 minutes deleted ~12 messages. The agent:
1. Fabricated a tmux INSERT mode story to explain a context gap
2. Narrated an odds-engine Claude Code session as mika development work
3. Gave confident but wrong approvals reconstructed from incomplete state

**Root cause:** `execute_rewind()` deleted messages and reversed memory mutations but injected **nothing** into the conversation. The agent's `load_recent_messages(20)` saw a gap with no explanation.

## Solution

Inject a `role='system'` context marker message into the rewound session after each rewind. The marker appears in the agent's message history via `load_recent_messages` and provides:

1. **Count of deleted messages** — the agent knows context was removed
2. **Specific reversal descriptions** — what memory changes were rolled back (wrapped in `<rewind_reversals trust="internal">` for prompt injection safety)
3. **Cross-session origin** — if the rewind was initiated from a different session
4. **Explicit instruction** — "Do not attempt to reconstruct or narrate what happened"

### Key Implementation Details

**Marker format:**
```
[Context notice: A rewind operation removed N message(s) from this conversation.
<rewind_reversals trust="internal">
Memory changes reversed:
- Restored core_memory field 'current_priorities'
- Deleted person 'Sarah'
</rewind_reversals>
This rewind was initiated from a different session (9085a9ab).
Do not attempt to reconstruct or narrate what happened in the removed messages. Continue from the last visible message above.]
```

**Accumulation guard:** Before injecting a new marker, delete any prior rewind markers in the same session via `delete_rewind_markers()`. This prevents context starvation during rapid successive rewinds — without this, 5 rewinds would leave 5 markers consuming the 20-message context window.

**Claude API compatibility:** Claude's Messages API does NOT accept `role="system"` in the messages array. The agent loop maps `system` → `user` (same treatment as `tool_result`). This is the most critical integration detail.

**Ordering:** The marker is saved AFTER `delete_messages_after_id` to prevent self-deletion.

**Correlation:** The marker message carries the rewind operation's `trace_id` for forensic analysis.

## Key Decisions

1. **`role='system'` message, not system prompt** — Persists in DB, positionally correct in timeline, survives across sessions and compaction, requires no `PromptContext` struct changes.

2. **Trust boundary on reversal descriptions** — Audit event `old_value`/`new_value` fields contain user-edited content. Wrapping in `<rewind_reversals trust="internal">` prevents prompt injection via crafted memory values.

3. **Accumulation guard via LIKE pattern** — `REWIND_MARKER_PREFIX` constant (`"[Context notice: A rewind"`) with `role = 'system'` filter. The combined filter makes false matches extremely unlikely.

4. **Reversal descriptions, not just counts** — The agent needs specifics ("restored core_memory.current_priorities") not just "3 reversals applied" to avoid re-confabulating about what changed.

## Gotchas

- **Don't forget the Claude API mapping.** If you add another special role to DB storage, it MUST be mapped to `user` or `assistant` before the API call. Claude returns HTTP 400 for unknown roles.

- **TUI cross-session reload filter.** The reload loop previously filtered `role == "user" || role == "assistant"`, which excluded system messages. Must include `system` for markers to appear after cross-session rewinds.

- **Marker must be saved AFTER message deletion.** If saved before `delete_messages_after_id`, the marker itself gets deleted.

- **`RewindResult.reversal_descriptions`** is populated from `build_reversal_previews()` filtering out `ReversalAction::Skip` items. If you change the preview logic, the marker content changes too.

## Testing

- `test_rewind_injects_context_marker` — Basic marker injection and content verification
- `test_rewind_marker_includes_reversal_descriptions` — Specific reversal descriptions in marker
- `test_rewind_marker_cross_session_includes_originator` — Cross-session origin in marker
- `test_rapid_rewinds_consolidate_markers` — Accumulation guard (only 1 marker after multiple rewinds)
- `test_build_rewind_marker_basic` — Unit test for marker builder
- `test_build_rewind_marker_with_reversals` — Unit test with reversal descriptions
- `test_build_rewind_marker_cross_session` — Unit test with originating session

## Related Patterns

- **Compaction summary injection** (`crates/mika-agent/src/compaction.rs`) — Similar pattern: `replace_with_summary()` injects a `role='summary'` message loaded into the system prompt. The rewind marker uses a simpler approach: a regular `role='system'` message in the message history.

- **Callback result trust boundary** (`agent.rs`) — `<callback_result trust="untrusted">` wrapper for external data. The rewind marker uses `<rewind_reversals trust="internal">` since the data is from the agent's own audit log, not external input.
