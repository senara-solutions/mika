---
title: "Fix TUI displaying callback task results as 'You:' instead of system message"
type: fix
status: completed
date: 2026-03-09
---

# Fix TUI Callback Result Display

## Overview

Callback task results (from long-running skills) are displayed in the TUI chat history with the **"You:"** prefix and raw `<callback_result trust="untrusted">` XML tags visible. They should be displayed as system/task messages with proper styling, and the XML framing should be hidden.

## Problem Statement

When a long-running skill completes, the callback result flows through two save paths:

1. **Raw result** saved as `role='tool_result'` in DB (`chat.rs:268-276`) — skipped during history display (`_ => continue`)
2. **Framed result** (with XML wrapper) passed as `user_message` to `run_agent()` (`chat.rs:279-302`), which saves it as `role='user'` (`agent.rs:591`)

On TUI restart, the framed message loads from history as a normal `"user"` role message → maps to `ChatRole::User` → renders with **"You:"** prefix including the raw `<callback_result trust="untrusted">` XML tags.

### Root Cause Chain

```
poll_callback_tasks()
  → AgentRequest::CallbackResult { task_id, label, result }
    → save_message("tool_result", &result)          // raw result saved (invisible in TUI)
    → format_callback_framing(&label, &task_id, &result)  // wraps in XML
    → run_agent(user_message: &framing)
      → save_message("user", &framing)              // ← BUG: framing saved as "user"
        → on restart: maps to ChatRole::User → "You:" with XML visible
```

## Proposed Solution

**Two-pronged fix:** prevent the framing from being saved as a user message, and properly display `tool_result` messages.

### Part A: Prevent framing from saving as user message

Mark callback turns so `run_agent` can skip saving the user message (or save with a distinct role). The callback result is already persisted as `tool_result` at `chat.rs:271` — the framing is an internal prompt construct and should not be saved again.

**Approach:** Add a `skip_save_user_message: bool` field to `AgentParams`. When `is_callback_turn` is true, set `skip_save_user_message: true` to prevent the duplicate save at `agent.rs:589-592`. This is the simplest fix since the raw result is already stored as `tool_result`.

**Files:**
- `crates/mika-agent/src/agent.rs:556` — add `skip_save_user_message` to `AgentParams`
- `crates/mika-agent/src/agent.rs:589-592` — guard save with `if !params.skip_save_user_message`
- `crates/mika-cli/src/commands/chat.rs:282` — set `skip_save_user_message: true` for callback turns
- All other `AgentParams` call sites — set `skip_save_user_message: false` (default behavior)

### Part B: Display `tool_result` messages properly in TUI

Map `tool_result` role to `ChatRole::System` with a descriptive prefix derived from metadata. No new `ChatRole` variant needed — keeps the change minimal.

**Three display locations to fix:**

1. **History loading on startup** (`chat.rs:395-398`)
   - Add match arm: `"tool_result"` → `ChatRole::System`
   - Extract label from `msg.metadata` JSON (`callback_task_id`, `label` fields)
   - Format content as `"[Task: {label}] Result received"` (brief, since the agent's response follows as the next assistant message)

2. **Cross-channel polling** (`app.rs:1039-1042`)
   - Same mapping: `"tool_result"` → `ChatRole::System` with metadata label extraction

3. **No change needed** for `poll_callback_tasks()` — it already shows `"[{label}] completed"` as `ChatRole::System` correctly

### Part C: Clean up stale framing messages (migration/cleanup)

Existing databases may have `role='user'` messages containing `<callback_result trust="untrusted">` from before the fix. These should be handled gracefully:

- On history load, detect messages starting with `"A background task has completed."` (the `format_callback_framing` prefix) and either skip them or remap to `ChatRole::System` with truncated display
- This is a display-time heuristic, not a DB migration — keeps it simple

## Technical Considerations

### Agent History Builder (Security Improvement)

Currently in `agent.rs:726`, `tool_result` messages are mapped to `"user"` role for the Claude API without the untrusted framing. This means callback results are framed as untrusted on the first turn but appear as trusted user input on subsequent turns.

**Out of scope for this fix** but flagged as a follow-up: the history builder should wrap `tool_result` content in the `<callback_result trust="untrusted">` framing when constructing messages for the API.

### Compaction

`compaction.rs:96-112` includes `tool_result` messages with the literal role prefix `"tool_result: <content>"`. This is acceptable since:
- The agent's response (assistant message) already captures the semantic content
- The compaction summarizer can handle varied role prefixes
- No change needed for this fix

### Content Truncation

Callback results can be up to 100KB. The TUI display for `tool_result` messages should show only a brief summary (the label), not the full content. The full analysis is in the agent's assistant response that follows.

## Acceptance Criteria

- [x] Callback results no longer display as "You:" messages in TUI history
- [x] Raw `<callback_result trust="untrusted">` XML is not visible to the user
- [x] Callback results display as system-styled messages with task label (e.g., `"[Task: analyze_codebase] Result received"`)
- [x] `run_agent` does not save the framing message as a `"user"` role on callback turns
- [x] Existing databases with stale framing messages render gracefully (no raw XML shown)
- [x] Cross-channel polling handles `tool_result` messages correctly
- [x] All existing tests pass (`cargo test`)

## Implementation Steps

### Step 1: Add `skip_save_user_message` to `AgentParams`

**File:** `crates/mika-agent/src/agent.rs`

```rust
// In AgentParams struct (~line 556)
pub skip_save_user_message: bool,

// In run_agent (~line 589)
if !params.skip_save_user_message {
    params
        .db
        .save_message(params.session_id, "user", &save_text, Some(&trace_id))
        .await?;
}
```

Update all `AgentParams` construction sites to include `skip_save_user_message: false`:
- `crates/mika-cli/src/commands/chat.rs` — normal message handler (`false`), callback handler (`true`)
- `crates/mika-cli/src/commands/ask.rs` — `false`
- `crates/mika-agent/src/agent.rs` — any test helpers
- `crates/mika-agent/src/server/handlers.rs` — `false`
- `crates/mika-agent/src/tools/management.rs` — delegated agent calls (`false`)
- `crates/mika-agent/src/team/engine.rs` — team agent calls (`false`)

### Step 2: Add metadata label extraction helper

**File:** `crates/mika-cli/src/tui/app.rs` or `crates/mika-cli/src/commands/chat.rs`

```rust
/// Extract callback task label from message metadata JSON.
fn callback_label_from_metadata(metadata: &Option<String>) -> Option<String> {
    let meta = metadata.as_ref()?;
    let parsed: serde_json::Value = serde_json::from_str(meta).ok()?;
    parsed.get("label")?.as_str().map(|s| s.to_string())
}
```

### Step 3: Fix history loading on startup

**File:** `crates/mika-cli/src/commands/chat.rs:395-398`

```rust
let role = match msg.role.as_str() {
    "user" => {
        // Skip stale framing messages from before the fix
        if msg.content.starts_with("A background task has completed.") {
            continue;
        }
        ChatRole::User
    }
    "assistant" => ChatRole::Assistant,
    "tool_result" => ChatRole::System, // callback results as system messages
    _ => continue,
};

// For tool_result, format display content with label
let content = if msg.role == "tool_result" {
    let label = callback_label_from_metadata(&msg.metadata)
        .unwrap_or_else(|| "background task".to_string());
    format!("[Task: {label}] Result received")
} else {
    msg.content
};
```

### Step 4: Fix cross-channel polling

**File:** `crates/mika-cli/src/tui/app.rs:1039-1042`

Apply the same `tool_result` → `ChatRole::System` mapping and stale framing detection as Step 3.

### Step 5: Update `SessionMessage` to carry metadata for display

The `load_recent_messages` and `load_messages_after` queries already SELECT `m.metadata`. Verify that `SessionMessage.metadata` is populated and accessible in the history loading and polling code paths.

### Step 6: Tests

- Test that callback turn does not save framing as "user" message
- Test that `tool_result` messages render as system messages in history
- Test that stale framing messages (pre-fix) are skipped
- Test `callback_label_from_metadata` helper with valid/missing/malformed JSON

## Sources

- GitHub Issue: [#83](https://github.com/senara-solutions/mika/issues/83)
- Callback TUI delivery polling: `docs/solutions/architecture-patterns/callback-tui-delivery-polling.md`
- Callback loop prevention: `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`

### Key Files

| File | Lines | Purpose |
|------|-------|---------|
| `crates/mika-agent/src/agent.rs` | 556-592 | `AgentParams`, user message save |
| `crates/mika-agent/src/agent.rs` | 48-62 | `format_callback_framing()` |
| `crates/mika-agent/src/agent.rs` | 724-730 | History builder role mapping |
| `crates/mika-cli/src/commands/chat.rs` | 257-338 | Callback handler in worker |
| `crates/mika-cli/src/commands/chat.rs` | 393-412 | History loading on startup |
| `crates/mika-cli/src/tui/app.rs` | 44-51 | `ChatRole` enum |
| `crates/mika-cli/src/tui/app.rs` | 1026-1065 | Cross-channel polling |
| `crates/mika-cli/src/tui/app.rs` | 1068-1115 | Callback polling |
| `crates/mika-cli/src/tui/ui.rs` | 190-282 | Message rendering by role |
