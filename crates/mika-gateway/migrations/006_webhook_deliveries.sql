-- Dead-letter queue for GitHub webhook deliveries that exhausted retries.
-- Tracks delivery attempts and allows manual/automatic replay.
CREATE TABLE webhook_deliveries (
    delivery_id     TEXT PRIMARY KEY,
    event_type      TEXT NOT NULL,
    target_agent    TEXT NOT NULL,
    repo_full_name  TEXT,
    payload         TEXT NOT NULL,
    request_id      TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'delivered', 'dead')),
    last_error      TEXT
);

-- Index for background worker: find pending entries eligible for retry.
CREATE INDEX idx_webhook_deliveries_pending
    ON webhook_deliveries (status, last_attempt_at)
    WHERE status = 'pending';

-- Index for CLI: list dead entries.
CREATE INDEX idx_webhook_deliveries_dead
    ON webhook_deliveries (created_at DESC)
    WHERE status = 'dead';
