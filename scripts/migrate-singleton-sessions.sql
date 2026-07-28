-- migrate-singleton-sessions.sql (mika#1401)
--
-- One-time, operator-runnable migration that merges an agent's scattered
-- per-ask sessions into its single canonical session. Written for mika-prime
-- (agent_id 'mika-prime', canonical session '00000000-0000-0000-0000-000000000000'),
-- but reusable for any agent newly opted into `[session] singleton = true`.
--
-- WHY: before mika#1401 every `mika ask` / `/send` minted a fresh Uuid::new_v4()
-- session, so a conceptually single-session oracle accumulated per-ask "session
-- confetti". The engine change stops the bleeding going forward; this script
-- consolidates the pre-existing rows so history reads as one thread.
--
-- SAFETY:
--   * Wrapped in `PRAGMA foreign_keys = OFF; BEGIN; ... COMMIT;` so FK-referencing
--     rows can be repointed before the old session rows are deleted.
--   * Idempotent — every WHERE clause self-narrows (`!= canonical`), so a second
--     run is a no-op.
--   * Take a backup first: `cp ~/.mika/data/mika.db ~/.mika/data/mika.db.bak`.
--
-- USAGE (defaults target mika-prime):
--   sqlite3 ~/.mika/data/mika.db < scripts/migrate-singleton-sessions.sql
--
-- For a DIFFERENT agent, edit the two :param values in the `params` CTE below
-- (SQLite has no CLI `.param set` in a piped script, so they are inlined).

-- ---------------------------------------------------------------------------
-- Parameters — edit these two lines for a non-mika-prime agent.
-- ---------------------------------------------------------------------------
-- AGENT_ID           = 'mika-prime'
-- CANONICAL_SESSION  = '00000000-0000-0000-0000-000000000000'

PRAGMA foreign_keys = OFF;

BEGIN;

-- 0. Ensure the canonical session row exists (INSERT OR IGNORE mirrors the
--    engine's get_or_create_canonical_session). channel_type 'cli' is arbitrary;
--    the singleton concept is orthogonal to channel.
INSERT OR IGNORE INTO sessions (id, agent_id, channel_type)
VALUES ('00000000-0000-0000-0000-000000000000', 'mika-prime', 'cli');

-- 1. Repoint every FK-referencing table from the old (non-canonical) sessions to
--    the canonical one. Each UPDATE is scoped to the agent and skips rows already
--    pointing at the canonical session (idempotency).

--    messages: preserves per-row created_at ordering, so the merged thread stays
--    chronologically coherent across the formerly-separate sessions.
UPDATE messages
SET session_id = '00000000-0000-0000-0000-000000000000'
WHERE agent_id = 'mika-prime'
  AND session_id != '00000000-0000-0000-0000-000000000000';

--    audit_events (session_id may be NULL for non-session events).
UPDATE audit_events
SET session_id = '00000000-0000-0000-0000-000000000000'
WHERE agent_id = 'mika-prime'
  AND session_id IS NOT NULL
  AND session_id != '00000000-0000-0000-0000-000000000000';

--    llm_calls.
UPDATE llm_calls
SET session_id = '00000000-0000-0000-0000-000000000000'
WHERE agent_id = 'mika-prime'
  AND session_id IS NOT NULL
  AND session_id != '00000000-0000-0000-0000-000000000000';

--    tool_calls.
UPDATE tool_calls
SET session_id = '00000000-0000-0000-0000-000000000000'
WHERE agent_id = 'mika-prime'
  AND session_id IS NOT NULL
  AND session_id != '00000000-0000-0000-0000-000000000000';

--    a2a_task_map (no agent_id column — narrow via the sessions it points at).
UPDATE a2a_task_map
SET session_id = '00000000-0000-0000-0000-000000000000'
WHERE session_id IN (
        SELECT id FROM sessions
        WHERE agent_id = 'mika-prime'
          AND id != '00000000-0000-0000-0000-000000000000'
      );

--    tasks.created_by_session (no agent_id filter needed — session-scoped).
UPDATE tasks
SET created_by_session = '00000000-0000-0000-0000-000000000000'
WHERE created_by_session IN (
        SELECT id FROM sessions
        WHERE agent_id = 'mika-prime'
          AND id != '00000000-0000-0000-0000-000000000000'
      );

--    sessions.parent_session_id (child sessions that pointed at an old parent).
UPDATE sessions
SET parent_session_id = '00000000-0000-0000-0000-000000000000'
WHERE parent_session_id IS NOT NULL
  AND parent_session_id IN (
        SELECT id FROM sessions
        WHERE agent_id = 'mika-prime'
          AND id != '00000000-0000-0000-0000-000000000000'
      );

-- 2. Delete the now-orphaned old session rows (everything for this agent except
--    the canonical session). All references were repointed above, so nothing
--    dangles.
DELETE FROM sessions
WHERE agent_id = 'mika-prime'
  AND id != '00000000-0000-0000-0000-000000000000';

-- 3. The canonical session must never be "ended" (pruning-safe). Clear any
--    ended_at that a prior end_session call may have set before the engine
--    fix landed.
UPDATE sessions
SET ended_at = NULL
WHERE id = '00000000-0000-0000-0000-000000000000';

COMMIT;

PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- Verification — run after the script. Expect exactly ONE row, the canonical
-- session, holding the full message count.
-- ---------------------------------------------------------------------------
-- SELECT session_id, COUNT(*) AS msg_count
-- FROM messages
-- WHERE agent_id = 'mika-prime'
-- GROUP BY session_id;
--
-- SELECT id, ended_at FROM sessions WHERE agent_id = 'mika-prime';
--   -> exactly one row, id = canonical session, ended_at = NULL
