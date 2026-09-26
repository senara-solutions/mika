-- Gateway-side audit_events ledger (mika#1774 / mika#1711 AC3 follow-up).
--
-- Records every silent-no-op drop taken by `crates/mika-gateway/src/github.rs`
-- so operators and dashboards can key off a persistent row instead of scraping
-- a `warn!` log line. The 2026-07-01 14 h dead-qa window (mika#1711 root class)
-- went unnoticed for hours precisely because the drop only surfaced in stdout.
--
-- Rows written by `crates/mika-gateway/src/audit_events.rs::log_webhook_drop`
-- with `tool_name = 'gateway_webhook'` and a `target_key` naming the drop
-- reason (e.g. `webhook_no_route`, `webhook_reviewer_filter_dropped`,
-- `webhook_denylisted_skill_dropped`, `webhook_synchronize_no_diff_change`).
--
-- Additive, observability-only: writes are fire-and-forget (a DB failure logs
-- WARN and lets the drop decision stand — the ticket is out-of-scope for retry
-- logic and MUST NOT alter routing behavior).
CREATE TABLE audit_events (
    id          BIGSERIAL   PRIMARY KEY,
    tool_name   TEXT        NOT NULL,
    target_key  TEXT        NOT NULL,
    metadata    JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Supports the dashboard query pattern from ticket AC3:
--   SELECT count(*) FROM audit_events
--   WHERE tool_name = 'gateway_webhook'
--     AND target_key = 'webhook_no_route'
--     AND created_at > now() - interval '1 day';
CREATE INDEX audit_events_lookup_idx
    ON audit_events (tool_name, target_key, created_at DESC);
