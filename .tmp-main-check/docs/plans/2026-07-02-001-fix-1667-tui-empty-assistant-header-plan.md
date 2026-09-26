---
issue: 1667
type: fix
date: 2026-07-02
---

# Plan — fix(tui): skip empty-content assistant rows in transcript render (mika#1667)

## Problem

In the `mika` TUI, after an agent (observed: `mika-dev`) emits a turn ending with **only tool calls and no text** — reliably a dispatch turn (`update_task_status` + `run_claude_pilot` + `send_message`) — the transcript renders a second, empty `Mika Dev:` role header with no body directly below the real reply:

```
Mika Dev:
mika#1617: fresh dev-pilot dispatched (task 392e969c, subprocess 79353ae9). ...

Mika Dev:
                ← empty header, no content body
```

It **persists across exit-and-rejoin** because it is a real persisted `messages` row, not a render-time glitch.

## Root cause (verified on branch)

Two behaviors interact. The empty row is legitimately written by the engine; the TUI wrongly renders a header for it.

1. **Engine writes an empty-content assistant row for tool-only EndTurn turns.** When the LLM returns `stop_reason = EndTurn` with tool_use blocks but no text block (the mika#151 EndTurn-with-tool_use path), `strip_internal_tags(&response.text())` yields an empty string (`crates/mika-agent/src/agent_loop/mod.rs:984`), and the EndTurn save persists an `assistant` row with `content = ""` plus `tool_calls` metadata (`mod.rs:2067-2079`). The user-facing text is persisted **separately** by the `send_message` tool as its own `assistant` row (`crates/mika-agent/src/tools/send_message.rs:62-70`). One dispatch turn → two assistant rows: one with the visible body, one empty.

2. **TUI renders a role header for empty-content assistant rows.** `session_message_to_chat_message` (`crates/mika-cli/src/tui/app.rs:186-218`) maps every `assistant` row to `ChatRole::Assistant` with `content = msg.content.clone()` — no skip for empty content. `build_message_lines` (`crates/mika-cli/src/tui/ui.rs:296-313`) unconditionally pushes the `format!("{identity_name}: ")` header, then renders markdown of the empty content (which produces nothing) → a bare `Mika Dev:` header with no body.

**Codebase-verified on this branch:**
- `crates/mika-cli/src/tui/app.rs:189-200` — role match has a stale-framing skip for `user` rows (`content.starts_with("A background task has completed.")`) but no empty-content skip for `assistant` rows. This is the natural insertion point (mirrors the existing skip pattern).
- `crates/mika-cli/src/tui/app.rs:206-211` — `content` for non-`tool_result` rows is `msg.content.clone()` verbatim; empty strings pass through.
- `crates/mika-cli/src/tui/ui.rs:296-306` — Assistant arm pushes the header line before rendering any body; empty content produces a header-only line.
- `crates/mika-cli/src/tui/app.rs:1546` — `session_message_to_chat_message` is the single conversion point used by startup history load, cross-channel poll, and rewind reload (per its own doc comment at `app.rs:181-185`), so a skip here fixes every load path uniformly.

## Fix shape (Option A — TUI skip, body-recommended primary)

Per the ticket's "Suggested fix shape (scoped, TUI-side primary)": in `session_message_to_chat_message`, return `None` for `assistant` rows whose `content` is empty after trim. These rows never carry user-visible text — the visible body is always a separate `send_message`/text row — and a header with no body is never useful to display. This mirrors the existing stale-framing skip and leaves the row intact in the DB for introspection/replay.

**Why not the engine-side removal.** The empty row's `tool_calls` metadata feeds the dashboard's tool-call fallback and the `<context type="tool_history">` history builder appends to assistant messages (`crates/mika-agent/CLAUDE.md` § Agent Loop). Dropping the write at `mod.rs:2068` risks conversation-history/replay fidelity. The TUI skip is the lower-risk fix and resolves the user-visible symptom completely. Out of scope here (see § Out of scope).

## Implementation outline

1. **Edit `crates/mika-cli/src/tui/app.rs::session_message_to_chat_message`** (the `"assistant"` match arm at `app.rs:197`). Change the arm from a bare `"assistant" => ChatRole::Assistant,` to a block that returns `None` when `msg.content.trim().is_empty()`, else `ChatRole::Assistant`:

   ```rust
   "assistant" => {
       // mika#1667: tool-only EndTurn turns persist an empty-content assistant
       // row (carrying tool_calls metadata) alongside the separate send_message
       // body row. Rendering it produces a bare "<agent>:" header with no body.
       // Skip it here; the row stays in the DB for introspection/replay.
       if msg.content.trim().is_empty() {
           return None;
       }
       ChatRole::Assistant
   }
   ```

   `trim()` (not just `is_empty()`) guards against whitespace-only content that would still render a body-less header. Placement inside the role match keeps the existing early-return skip idiom (the `user` stale-framing skip immediately above).

2. **No change to `ui.rs`.** With empty assistant rows filtered upstream at the single conversion point, `build_message_lines` never receives one. No defensive guard is added there — the skip is authoritative at the conversion boundary, matching how the stale-framing skip works.

3. **Unit tests** in `crates/mika-cli/src/tui/app.rs` `#[cfg(test)] mod tests` (block at `app.rs:1683`; a `SessionMessage` literal is constructed with the fields at `crates/mika-agent/src/db.rs:140-152`: `id, session_id, agent_id, role, content, channel_type, metadata, trace_id, created_at, internal`):
   - `test_empty_assistant_row_skipped` — `role="assistant"`, `content=""` → returns `None`.
   - `test_whitespace_assistant_row_skipped` — `role="assistant"`, `content="   \n"` → returns `None`.
   - `test_nonempty_assistant_row_rendered` — `role="assistant"`, `content="real reply"` → returns `Some(ChatMessage { role: ChatRole::Assistant, content: "real reply", .. })`.
   - `test_empty_user_row_still_rendered` — `role="user"`, `content=""` → returns `Some` (the skip is assistant-scoped; user rows are unaffected — regression guard against over-broad filtering).

## Acceptance criteria

- **AC1** — `session_message_to_chat_message` returns `None` for `assistant` rows whose `content` is empty or whitespace-only. Non-empty `assistant` rows still map to `ChatRole::Assistant` with unchanged content. Verified by the unit tests in step 3.
- **AC2** — The skip is scoped to `assistant` rows only. `user` and `tool_result` rows with empty content are unaffected (their existing behavior is preserved). Verified by `test_empty_user_row_still_rendered`.
- **AC3** — After the fix, a tool-only EndTurn dispatch turn no longer renders a bare `<agent>:` header in the TUI transcript, across startup history load, cross-channel poll, and rewind reload (all route through the single conversion point). Verified structurally: the single-conversion-point invariant (`app.rs:181-185` doc comment + the three callers) means one skip covers all three load paths.
- **AC4** — The empty `messages` row remains in the DB (no engine write is removed) — introspection/replay and dashboard tool-call fallback are unaffected. Verified by inspection: no change outside `crates/mika-cli/src/tui/app.rs`.

## Definition of Done

- `session_message_to_chat_message` skips empty/whitespace-only assistant rows.
- Four unit tests added and passing.
- `cargo build -p mika-cli` clean.
- `cargo test -p mika-cli` green (new tests + no regression in existing `app.rs` tests).
- `cargo clippy -p mika-cli` clean.
- No changes outside `crates/mika-cli/src/tui/app.rs`.

## Out of scope

- **Engine-side removal of the empty EndTurn `save_message`** (`crates/mika-agent/src/agent_loop/mod.rs:2068`). Higher-risk — the empty row's `tool_calls` metadata feeds the dashboard tool-call fallback and the tool-history context builder. If pursued later, scope it to "skip the EndTurn save only when `text.is_empty()` AND the turn's tool calls were already persisted to the `tool_calls` table," and file as a separate ticket with its own conversation-history/replay-fidelity verification.
- **Backfill/cleanup of existing empty rows** already in `~/.mika/data/mika.db`. Not needed — the TUI skip handles them at render time on every load; they stay for introspection.
- **Dashboard rendering** of the same empty rows. The dashboard is a separate surface (React) and is not reported as affected; the empty row's metadata is load-bearing there for the tool-call fallback.

## Files involved

- `crates/mika-cli/src/tui/app.rs` — `session_message_to_chat_message` assistant-arm skip (step 1) + unit tests (step 3). **Sole file changed.**

## Verification

- **Static:** `cargo build -p mika-cli`, `cargo clippy -p mika-cli`, `cargo test -p mika-cli` (covers the four new unit tests + existing `app.rs` suite).
- **Manual (post-merge, optional):** in a `mika chat`/TUI session with `mika-dev`, trigger a `run_claude_pilot` dispatch turn (tool-only EndTurn), then exit and rejoin — confirm no bare `Mika Dev:` header appears below the real reply. The reproduction query from the ticket (`SELECT id, session_id, length(content), metadata FROM messages WHERE agent_id='mika-dev' AND role='assistant' AND length(content)=0`) still returns the row (DB unchanged), but the TUI no longer renders it.

## References

- mika#1667 — this ticket (read-only investigation; both code locations cited on `main` as of 2026-06-30, re-verified on this branch).
- mika#151 — EndTurn-with-tool_use path (the origin of the tool-only EndTurn shape that produces the empty row).
- `crates/mika-cli/src/tui/app.rs:181-185` — single-conversion-point doc comment (why one skip covers all load paths).
- `crates/mika-cli/src/tui/app.rs:191-194` — existing stale-framing skip for `user` rows (the idiom this fix mirrors).
- `crates/mika-agent/src/tools/send_message.rs:62-70` — the separate visible-body row write (why the empty row is redundant for display).
