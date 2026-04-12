---
title: "feat: Silence internal dev questions in the TUI"
type: feat
status: completed
date: 2026-04-12
issue: "#494"
---

# Silence Internal Dev Questions in the TUI

## Overview

Keep agent-to-agent "dev" questions out of the TUI's main view while preserving full threading in the audit log. The TUI becomes an **escalation-only inbox** — quiet until a question is actually addressed to the human.

## Problem Statement

The TUI currently shows every question regardless of intended audience. When mika-dev asks mika-qa something, the human sees it and has to mentally filter. This breaks the "Mika as coworker" perspective and turns the TUI into a debug console.

## Proposed Solution

Two complementary changes:

1. **Tag at source:** Add `internal: bool` field to the `messages` table. Messages in delegate sessions (`delegate-*`) are implicitly internal. The `send_message` tool gains an optional `internal` parameter for explicit opt-in from other contexts (callbacks, heartbeats).

2. **Filter at view:** TUI defaults to **inbox mode** (hides `internal: true` messages). A `/inbox` toggle switches to **audit mode** (shows all). A footer badge shows the count of hidden internal messages.

## Technical Approach

### Phase 1: Data Model — `internal` Column on `messages`

**Schema migration v21→v22** in `crates/mika-agent/src/db.rs`:

```sql
ALTER TABLE messages ADD COLUMN internal INTEGER NOT NULL DEFAULT 0;
```

A dedicated column (not JSON metadata) because:
- SQL-level filtering with correct `LIMIT` semantics (no limit starvation)
- Indexable for performance
- Clean query predicates without `json_extract()`

**Struct changes:**
- `SessionMessage` (`db.rs:192`): add `pub internal: bool`
- All `SELECT` queries that read from `messages` must include the new column

**Write paths — set `internal = true` for:**
- `save_message()` and `save_message_with_metadata()`: add `internal: bool` parameter
- All delegate sessions (`delegate-*`): the `delegate_task` tool (`tools/delegate_task.rs`) saves messages with `internal: true`
- `send_message` tool: new optional `internal` bool parameter (default `false`)
- Callback result messages saved by the TUI (`chat.rs:322`): `internal: false` (callback results are user-relevant)
- Agent loop response saves (`agent.rs`): pass `internal: false` for conversation mode, `internal: true` for silent mode delegate contexts

### Phase 2: DB Query Filtering

**`load_recent_messages(agent_id, limit)`** → **`load_recent_messages(agent_id, limit, exclude_internal: bool)`**

```sql
SELECT ... FROM messages m JOIN sessions s ON m.session_id = s.id
WHERE m.agent_id = ?1 AND m.role != 'summary' AND s.channel_type != 'team'
  AND (?3 = 0 OR m.internal = 0)   -- filter when exclude_internal = true
ORDER BY m.id DESC LIMIT ?2
```

**`load_messages_after(agent_id, after_id)`** → **`load_messages_after(agent_id, after_id, exclude_internal: bool)`**

Same pattern — add `AND (?3 = 0 OR m.internal = 0)`.

**`count_internal_messages_after(agent_id, after_id) -> i64`** — new query for the footer badge count.

**`AsyncDatabase` wrappers:** Update to pass through the new parameter.

### Phase 3: TUI Inbox Mode

**`App` struct** (`crates/mika-cli/src/tui/app.rs:554`):
- Add `pub inbox_mode: bool` (default `true`) alongside `verbose_mode`
- Add `pub hidden_internal_count: usize` (for footer badge)

**`ChatMessage` struct** (`app.rs:44`):
- Add `pub internal: bool` field

**Four display paths must respect `inbox_mode`:**

1. **Startup history load** (`chat.rs:491-525`): Pass `exclude_internal: self.inbox_mode` to `load_recent_messages`. When constructing `ChatMessage`, carry `internal` from `SessionMessage`.

2. **Cross-channel poll** (`app.rs:1370-1425`, `poll_cross_channel_messages`): Pass `exclude_internal: self.inbox_mode` to `load_messages_after`. When `inbox_mode` is true, skip internal messages. Increment `hidden_internal_count` for skipped messages.

3. **Callback poll** (`app.rs:1427-1530`, `poll_callback_tasks`): Pass through — callback task results are user-facing (`internal: false`).

4. **Rewind history reload** (`handlers.rs:1196-1228`): Same as startup — pass `exclude_internal: self.inbox_mode`.

**Shared conversion helper:**
Extract a `session_message_to_chat_message(msg: &SessionMessage, inbox_mode: bool) -> Option<ChatMessage>` function to unify the four paths and eliminate copy-pasted conversion logic.

### Phase 4: Toggle Command and Footer Badge

**`/inbox` slash command** (`crates/mika-cli/src/tui/commands/`):
- Register in `handlers.rs:21` command list and `handlers.rs:69` dispatch
- Add to `completers.rs` for tab completion
- Toggles `app.inbox_mode` between `true` (inbox/escalation-only) and `false` (audit/show all)
- On toggle to audit mode: reload messages from DB without the internal filter (call existing rewind-reload logic with `exclude_internal: false`)
- On toggle to inbox mode: reload with filter active
- Print system message: `"Switched to inbox mode (internal messages hidden)"` or `"Switched to audit mode (all messages visible)"`

**Footer badge** (`crates/mika-cli/src/tui/ui.rs:936`, badge pattern at lines 1047-1062):
- When `inbox_mode == true && hidden_internal_count > 0`: render `[N hidden]` badge in DarkGray
- Follows existing badge pattern (`[N tasks]` in Cyan, `[N running]` in Yellow)

**Hidden count maintenance:**
- Increment `hidden_internal_count` in `poll_cross_channel_messages` when skipping internal messages
- Reset on `/clear` (new session)
- Reset on mode toggle (reloaded from DB)
- Periodically refresh via `count_internal_messages_after` query (piggyback on existing 5s poll tick)

### Phase 5: `/undo` and `/rewind` Awareness

**`/undo`** in inbox mode should skip internal messages — undo the last **visible** exchange:
- In `handlers.rs` rewind logic: when `inbox_mode == true`, filter out internal messages from the candidate list before selecting the undo target

**`/rewind N`** in inbox mode: count only visible messages when determining how far back to go.

## Acceptance Criteria

- [x] Schema v22: `messages.internal` column (INTEGER NOT NULL DEFAULT 0)
- [x] `SessionMessage` struct carries `internal: bool`
- [x] `save_internal_message` / `save_internal_message_with_metadata` persist internal flag
- [x] Delegate task messages saved with `internal: true`
- [x] `send_message` tool accepts optional `internal` parameter
- [x] `load_recent_messages_filtered` supports `exclude_internal` filter
- [x] `ChatMessage` carries `internal: bool`
- [x] TUI defaults to inbox mode (`inbox_mode: true`)
- [x] Startup history load respects inbox mode
- [x] Cross-channel poll respects inbox mode
- [x] Rewind reload respects inbox mode
- [x] `/inbox` command toggles between inbox and audit mode with message reload
- [x] Footer shows `[N hidden]` badge when internal messages are suppressed
- [x] `/undo` in inbox mode works correctly (exchange-level boundaries naturally skip internal)
- [x] All existing tests pass (`cargo test`)
- [x] New tests for internal message filtering (6 DB-level tests)

## Design Decisions

### Column vs. JSON metadata
**Chosen: dedicated column.** JSON metadata (`json_extract`) cannot be indexed efficiently, causes limit starvation when filtering with `LIMIT`, and adds parsing overhead. A column enables clean SQL predicates and correct pagination.

### Session-level vs. message-level tagging
**Chosen: message-level with session-level defaults.** Delegate sessions are implicitly internal (all messages `internal: true`). Other contexts use explicit opt-in via the `send_message` tool parameter. This gives both convenience (delegate sessions auto-tagged) and flexibility (any message can be tagged).

### `internal: false` as global default
**Chosen: false globally.** The issue's open question #1 asked about per-agent defaults. Global false is simpler — the primary use case (delegate sessions) is handled by implicit tagging, not per-agent config. Agent-level defaults can be added later without schema changes.

### Deferred to follow-up issues
- Auto-promote internal questions on timeout (issue open question #2)
- Retroactive tagging of in-flight questions (issue open question #4)
- Dashboard API filtering for internal messages
- Compaction handling of internal messages (summary inherits internal flag)

## System-Wide Impact

### Interaction graph
`save_message_with_metadata()` is called from: agent loop EndTurn, agent loop tool_result, `send_message` tool, delegate task save, callback result save. All callers need the `internal` parameter.

### Error propagation
No new error paths. The `internal` column has a `DEFAULT 0` — callers that don't pass it get `false`.

### State lifecycle risks
None. The column is additive with a safe default. Existing messages get `internal = 0` (visible).

### API surface parity
Dashboard API (`/api/v1/messages`, `/api/v1/sessions/*/messages`): `SessionMessage` will carry `internal` in the JSON response. No server-side filtering — dashboard decides client-side.

`mika ask` stdout output: unaffected (shows all messages, no inbox concept).

## Sources

- Issue: [#494](https://github.com/senara-solutions/mika/issues/494)
- Precedent: `verbose_mode` toggle (`app.rs:554`, `handlers.rs:970`)
- Precedent: `strip_internal_tags()` pattern (`docs/solutions/ui-bugs/strip-internal-metadata-tags-from-display.md`)
- Learning: dual-path consistency (`docs/solutions/logic-errors/tui-callback-skips-mika-qa-delegation.md`)
- Learning: inclusive filter patterns (`docs/solutions/dashboard-issues/dev-runs-source-filter-too-restrictive.md`)
