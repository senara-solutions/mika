-- Orchestrator inbox: spawned Claude Code tenants write messages addressed to
-- their parent orchestrator-Claude session. The orchestrator subscribes via SSE
-- and surfaces messages in real-time, eliminating the operator-mediated paste
-- cycle of the filesystem-inbox protocol (mika-platform#100).
--
-- See mika#1189 plan: docs/plans/2026-05-17-003-feat-1189-mika-gateway-orchestrator-inbox-v2-plan.md
CREATE TABLE orchestrator_inbox_messages (
    id              BIGSERIAL PRIMARY KEY,
    orchestrator_id TEXT        NOT NULL,
    spawn_id        TEXT,
    kind            TEXT        NOT NULL
                    CHECK (kind IN ('handoff', 'update', 'ack')),
    body            JSONB       NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at    TIMESTAMPTZ
);

-- Cursor replay: SSE clients resume from Last-Event-Id with id > cursor for
-- their orchestrator_id. Composite (orchestrator_id, id) lets the planner use
-- an index-only scan for `WHERE orchestrator_id = $1 AND id > $2 ORDER BY id`.
CREATE INDEX orchestrator_inbox_messages_recv_idx
    ON orchestrator_inbox_messages (orchestrator_id, id);

-- Retention sweep: partial index on undelivered rows lets the cleanup task
-- find candidates without scanning the full table.
CREATE INDEX orchestrator_inbox_messages_undelivered_idx
    ON orchestrator_inbox_messages (orchestrator_id, created_at)
    WHERE delivered_at IS NULL;
