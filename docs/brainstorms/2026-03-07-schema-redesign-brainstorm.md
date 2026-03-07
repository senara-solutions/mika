# Schema Redesign: Sessions + Messages

**Date:** 2026-03-07
**Status:** Decided

## What We're Building

Redesign Mika's data model to follow the industry-standard sessions + messages two-table pattern. The current `conversations` table becomes `messages` with a `session_id` FK, a new `sessions` table holds per-session metadata (channel_type, timestamps), and `team_messages` is renamed to `team_workspace` (storing only structured execution artifacts, not conversation messages).

### Problem Statement

The current model has unclear boundaries:
- `conversations` stores everything (chat, callbacks, team-adjacent data) with a `channel_type` column per message
- `team_messages` exists as a separate message store for team execution
- No explicit session concept despite session_id being used in `memory_events` and `tasks`
- `channel_type` is per-message when it's really a per-session attribute

### Goals

1. Single `messages` table as the source of truth for ALL conversation messages
2. Explicit `sessions` table for grouping and metadata
3. `team_workspace` for structured execution artifacts only (plans, assignments, progress)
4. Agent text responses from team runs stored in `messages` (they're conversations)
5. Clean naming: `conversations` -> `messages`, `ConversationMessage` -> `SessionMessage`

## Why This Approach

After auditing the codebase, we found that `conversations` and `team_messages` don't actually duplicate data -- they're fully separate stores. But `team_messages` mixes two concerns: structured artifacts (plans, assignments, critic feedback) and agent text responses. The redesign separates these cleanly.

OpenClaw uses file-based JSONL per session with composite session keys (`agent:{id}:{channel}:...`). While their architecture differs (no relational DB), the key insight applies: sessions should encode channel context, not individual messages.

## Key Decisions

1. **`team_messages` renamed to `team_workspace`** -- stores only structured artifacts (goal, plan, assignment, progress, critic_feedback, revision_request, deliverable). Agent `response` and `error` types move to `messages` table with a session linked to the team run.

2. **One session per team run** -- all agent responses within a team run reference the same session_id. Simple, maps to how the user experiences it.

3. **Callbacks continue existing session** -- callback results are injected back into the session that spawned the long-running task (via `tasks.created_by_session`). No more fragmented `callback-{uuid}` sessions.

4. **`channel_type` on `SessionMessage` via JOIN** -- `channel_type` is stored on `sessions` table only, but `SessionMessage` struct includes it populated via JOIN. This minimizes consumer-side changes.

5. **Rename `ConversationMessage` to `SessionMessage`** -- reflects the session-based model.

6. **No migration needed** -- single user, clean-slate drop and recreate. Schema version bumped to 3.

7. **Summary session convention** -- compaction summaries use a deterministic system session (`"system-{agent_id}"`) to avoid creating throwaway sessions.

## Schema

### sessions (NEW)
```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    channel_type TEXT NOT NULL DEFAULT 'cli',
    started_at INTEGER NOT NULL DEFAULT (unixepoch()),
    ended_at INTEGER,
    metadata TEXT
);
CREATE INDEX idx_sessions_agent ON sessions(agent_id, started_at DESC);
```

### messages (renamed from conversations)
```sql
CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('user','assistant','system','summary','tool_result')),
    content TEXT NOT NULL,
    metadata TEXT,
    compacted_through_id INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_msg_session ON messages(session_id, created_at ASC);
CREATE INDEX idx_msg_agent_created ON messages(agent_id, created_at DESC);
```

### team_workspace (renamed from team_messages)
```sql
CREATE TABLE team_workspace (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES team_runs(id) ON DELETE CASCADE,
    parent_id INTEGER REFERENCES team_workspace(id),
    agent_name TEXT,
    entry_type TEXT NOT NULL,
    content TEXT NOT NULL,
    iteration INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_team_ws_run ON team_workspace(run_id, created_at);
```

## Resolved Questions

- **What's in team_messages vs conversations?** Fully separate stores. team_messages has tree structure (parent_id), message_type, iteration. No actual row duplication.
- **Does sessions concept work with current code?** Yes -- session_id already exists in memory_events and tasks.created_by_session. Adding it to messages is a natural extension.
- **Silent mode persistence?** Heartbeat/reflection/skill_run don't save to conversations today. They'll create sessions for tracking but the no-save behavior stays.
- **Team agent conversations storage?** Agent text responses go in messages (linked to team session). Structured artifacts stay in team_workspace.
- **Callback session handling?** Callbacks continue the existing session that spawned the task, using tasks.created_by_session.
- **Compaction impact?** Compaction stays agent-scoped (not session-scoped). Uses a deterministic system session for summary storage.

## Open Questions

None -- all questions resolved during brainstorm.
