---
title: "fix: Inject context marker after rewind to prevent agent confabulation"
type: fix
status: completed
date: 2026-03-11
origin: docs/brainstorms/2026-03-11-conversation-rewind-brainstorm.md
---

# fix: Inject context marker after rewind to prevent agent confabulation

## Overview

After a rewind operation, the agent has no awareness that messages were removed. This causes confabulation — the agent pattern-matches from incomplete context and fabricates explanations to fill gaps. Fix by injecting a persistent "rewind marker" message into the rewound session, similar to how compaction injects a summary.

## Problem Statement

In production, 5 rapid cross-session rewinds in 9 minutes deleted ~12 messages from Mika's context. With no marker indicating the gap, Mika:

1. **Fabricated a tmux INSERT mode story** to explain a context gap
2. **Narrated an odds-engine claude-asked event as mika work** — no anchoring context remained to distinguish projects
3. **Gave confident but wrong approvals** — reasoning was reconstructed from incomplete state

The root cause: `execute_rewind()` deletes messages and reverses memory mutations, but injects **nothing** into the conversation that the agent will see on its next turn. The agent loop loads `load_recent_messages(20)` and sees a gap with no explanation.

## Proposed Solution

Inject a `role='system'` message into the rewound session after each rewind. This message appears in the agent's message history (via `load_recent_messages`) and provides:
- How many messages were removed
- What memory reversals occurred
- That the agent should not attempt to reconstruct or narrate what happened in the gap

### Why `role='system'` message (not system prompt)

- **Persists in DB** — survives across sessions, compaction loads it naturally
- **Positional** — appears at the right point in the conversation timeline
- **Already handled** — the agent loop maps all roles to Claude API messages; `system` role messages become `user` role in the API (same as `tool_result`)
- **No PromptContext changes needed** — unlike a system prompt section, this requires no struct changes

## Technical Approach

### 1. Add rewind marker injection to `execute_rewind()`

File: `crates/mika-agent/src/rewind.rs` (line ~378, after `delete_messages_after_id`)

After deleting messages and applying reversals, save a system message:

```rust
// Build context marker for the agent
let marker = build_rewind_marker(&result, session_id, originating_session_id);
db.save_message(session_id, "system", &marker, Some(&rewind_trace_id)).await?;
```

The marker content:

```
[Context notice: A rewind operation removed {N} messages from this conversation.
{M} memory changes were reversed ({list: e.g., "restored core_memory.current_priorities, deleted person 'Sarah'"}).
Do not attempt to reconstruct or narrate what happened in the removed messages.
Continue from the last visible message above.]
```

### 2. Add `build_rewind_marker()` function

File: `crates/mika-agent/src/rewind.rs`

The marker must include **specific reversal descriptions** (not just counts) so the agent knows what was rolled back. Add a `reversal_descriptions: Vec<String>` field to `RewindResult`, populated from the `ReversalPreview` descriptions during execution.

```rust
fn build_rewind_marker(
    result: &RewindResult,
    originating_session_id: Option<&str>,
) -> String {
    let mut marker = format!(
        "[Context notice: A rewind operation removed {} message(s) from this conversation.",
        result.messages_deleted
    );
    if !result.reversal_descriptions.is_empty() {
        marker.push_str("\nMemory changes reversed:");
        for desc in &result.reversal_descriptions {
            marker.push_str(&format!("\n- {desc}"));
        }
    }
    if let Some(orig) = originating_session_id {
        marker.push_str(&format!(
            "\nThis rewind was initiated from a different session ({}).",
            &orig[..orig.len().min(8)]
        ));
    }
    marker.push_str(
        "\nDo not attempt to reconstruct or narrate what happened in the removed messages. \
         Continue from the last visible message above.]"
    );
    marker
}
```

**Marker accumulation guard:** Before injecting a new marker, delete any prior rewind markers in the same session to prevent context starvation during rapid successive rewinds:

```rust
db.delete_rewind_markers(session_id).await?;
db.save_message(session_id, "system", &marker, Some(&rewind_trace_id)).await?;
```

Where `delete_rewind_markers` is: `DELETE FROM messages WHERE session_id = ?1 AND role = 'system' AND content LIKE '[Context notice: A rewind%'`

### 3. CRITICAL: Handle `system` role in agent loop message mapping

File: `crates/mika-agent/src/agent.rs` (line ~728)

**This is the most critical change.** Claude's Messages API does NOT accept `role="system"` in the `messages` array — system instructions go in the top-level `system` parameter. Passing `role="system"` will cause a 400 API error. Map it to `"user"` (same as `tool_result`):

```rust
role: if msg.role == "tool_result" || msg.role == "system" {
    "user".to_string()
} else {
    msg.role.clone()
},
```

Also check the silent agent loop at line ~1328 for the same mapping.

**Check:** Verify `system` is in the `role` CHECK constraint in the messages table schema. If not, add it in a migration or use an existing allowed role.

### 4. Verify role CHECK constraint

File: `crates/mika-agent/src/db.rs` (line ~673)

Current CHECK: `role IN ('user','assistant','system','summary','tool_result')`. `system` is already allowed.

### 5. Update TUI display after rewind

File: `crates/mika-cli/src/tui/commands/handlers.rs` (line ~937)

After rewind, the marker message is in the DB. The TUI should show it as a system message in the chat display. Add it to `app.messages` after the truncation/reload:

```rust
// Add rewind marker to display
app.messages.push(ChatMessage {
    role: ChatRole::System,
    content: format!(
        "Rewind complete: {} messages removed, {} reversals applied.",
        result.messages_deleted, result.reversals_applied
    ),
    rendered: None,
    channel: None,
});
```

**Fix cross-session reload filter** at line ~949: The reload loop filters `msg.role == "user" || msg.role == "assistant"`, which excludes `system` messages. Add `|| msg.role == "system"` so rewind markers appear after cross-session rewinds too:

```rust
if msg.role == "user" || msg.role == "assistant" || msg.role == "system" {
```

### 6. Server-side: no changes needed

The server endpoints (`handle_rewind_execute`) call `execute_rewind()` which will now inject the marker automatically. The marker persists in the DB, so the next time the agent processes a message in that session, it will see the marker in its history.

## Acceptance Criteria

- [x] `execute_rewind()` injects a `role='system'` marker message into the rewound session after deleting messages (`crates/mika-agent/src/rewind.rs`)
- [x] Marker message includes: count of deleted messages, specific reversal descriptions (not just counts), instruction not to confabulate
- [x] `RewindResult` carries `reversal_descriptions: Vec<String>` for marker content
- [x] Prior rewind markers in the same session are deleted before injecting a new one (accumulation guard)
- [x] Cross-session rewinds note the originating session ID in the marker
- [x] Agent loop maps `system` role to `user` for Claude API (same as `tool_result`) — conversation loop (`crates/mika-agent/src/agent.rs`). Silent loop not needed (uses single trigger message, no history).
- [x] TUI displays the rewind marker after executing `/undo` or `/rewind` (marker in DB, loaded on next turn)
- [x] TUI cross-session reload filter includes `role="system"` messages
- [x] Marker message has the rewind operation's `trace_id` for correlation
- [x] Marker is saved AFTER `delete_messages_after_id` (ordering matters — prevents self-deletion)
- [x] Existing rewind tests pass with marker injection (marker adds 1 message to remaining)
- [x] New test: verify marker content includes specific reversal descriptions
- [x] New test: verify cross-session marker includes originating session
- [x] New test: verify rapid successive rewinds consolidate to single marker
- [x] New test: verify `build_rewind_marker` unit tests (basic, with reversals, cross-session)
- [x] `cargo test` passes (1155 tests)
- [x] `cargo clippy` clean

## Context

The compaction system already uses this pattern — `replace_with_summary()` injects a `role='summary'` message that the agent loop loads via `load_conversation_summary()` and injects into the system prompt. The rewind marker uses a simpler approach: a `role='system'` message in the regular message history, which appears naturally in `load_recent_messages(20)`.

## Sources & References

### Origin

- **Brainstorm document:** [docs/brainstorms/2026-03-11-conversation-rewind-brainstorm.md](docs/brainstorms/2026-03-11-conversation-rewind-brainstorm.md) — The original rewind brainstorm focused on mechanical correctness (transaction safety, reversal ordering). This plan addresses the post-rewind agent awareness gap discovered in production.
- **User feedback:** 5 rapid cross-session rewinds hollowed out agent context, causing confabulation and cross-project confusion.

### Internal References

- Rewind engine: `crates/mika-agent/src/rewind.rs`
- Agent loop message loading: `crates/mika-agent/src/agent.rs:704`
- Compaction summary pattern: `crates/mika-agent/src/compaction.rs:72`, `crates/mika-agent/src/db.rs:2203`
- TUI rewind handler: `crates/mika-cli/src/tui/commands/handlers.rs:886`
- Server rewind handler: `crates/mika-agent/src/server/rewind.rs`
- Message role CHECK constraint: `crates/mika-agent/src/db.rs:673`
