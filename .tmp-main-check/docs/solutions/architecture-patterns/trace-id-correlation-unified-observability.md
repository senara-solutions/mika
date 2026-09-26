---
title: "Orthogonal Observability: Trace ID Correlation Across All Subsystems"
date: 2026-03-08
category: architecture-patterns
tags:
  - observability
  - trace-id
  - schema-migration
  - event-correlation
  - database
  - unified-timeline
modules:
  - crates/mika-agent/src/trace.rs
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/async_db.rs
  - crates/mika-agent/src/tools/mod.rs
  - crates/mika-agent/src/agent.rs
  - crates/mika-agent/src/teams/engine.rs
  - crates/mika-agent/src/skills/executor.rs
  - crates/mika-agent/src/prompt.rs
severity: high
resolution_time: "~4 hours across 6 commits"
pr: "#88"
origin: docs/brainstorms/2026-03-08-orthogonal-observability-brainstorm.md
plan: docs/plans/2026-03-08-feat-orthogonal-observability-plan.md
---

# Orthogonal Observability: Trace ID Correlation Across All Subsystems

## Problem

Mika had 5+ independent event systems that evolved without shared correlation IDs:

| System | Captures | Had trace_id? |
|--------|----------|---------------|
| `memory_events` (audit log) | Fact/memory mutations | No |
| `messages` | User/assistant messages | No |
| `tasks` | Scheduled/callback tasks | No |
| `team_workspace` | Team phase changes, agent status | No |
| `tracing` spans (OTel) | agent_turn, team_run | Yes (OTel only) |

**Impact:** Impossible to answer "what happened during this agent turn?" without querying each system separately. No correlation between Jaeger/Langfuse spans and the database rows they produced.

## Root Cause

Each subsystem was built independently. Messages had `session_id`, audit events had `session_id`, tasks had `agent_id`, but no shared request-level identifier tied them together. OTel spans existed but were disconnected from the persistence layer.

## Solution

Two orthogonal correlation axes:

- **Request axis (`trace_id`):** One per agent turn — 32-char lowercase hex — correlates with Jaeger/Langfuse spans
- **System axis (`session_id` + `agent_id`):** Groups events within a conversation and by actor

### Phase A: Schema Migration v4 -> v5

1. Renamed `memory_events` -> `audit_events` (semantic clarity)
2. Added nullable `trace_id TEXT` columns to `messages`, `audit_events`, `tasks` (`created_trace_id`), `team_workspace`
3. Created partial indexes (`WHERE trace_id IS NOT NULL`) on all trace_id columns
4. Created `unified_timeline` VIEW (`UNION ALL` across messages, audit_events, tasks)

### Phase B: Thread trace_id Through All Write Paths

1. New `trace.rs` module with `generate_trace_id()` — OTel extraction or UUID v4 fallback
2. Added `trace_id: &'a str` field to `ToolContext`
3. Added `trace_id: String` field to `LongRunningContext`
4. Added `trace_id: String` field to `TeamEngine`
5. Updated all DB write calls to pass `Some(trace_id)`

## Key Code Patterns

### Trace ID Generation (`trace.rs`)

```rust
pub fn generate_trace_id() -> String {
    #[cfg(feature = "telemetry")]
    {
        use opentelemetry::trace::TraceContextExt;
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        let span = tracing::Span::current();
        let ctx = span.context();
        let span_ref = ctx.span();
        let sc = span_ref.span_context();
        if sc.trace_id() != opentelemetry::trace::TraceId::INVALID {
            return format!("{}", sc.trace_id());
        }
    }
    uuid::Uuid::new_v4().simple().to_string()
}
```

### Threading Through Agent Loop (`agent.rs`)

```rust
let trace_id = crate::trace::generate_trace_id();

// Save user message with trace_id
db.save_message(session_id, "user", &text, Some(&trace_id)).await?;

// Create ToolContext with trace_id
let tool_ctx = ToolContext {
    db, session_id, trace_id: &trace_id, /* ... */
};

// Create LongRunningContext with trace_id
let lr_ctx = Some(LongRunningContext {
    db: db.clone(), agent_name: db.agent_id.clone(),
    session_id: session_id.to_string(),
    trace_id: trace_id.to_string(),
});
```

### Tool Audit Logging

```rust
// In any tool's execute():
ctx.db.log_audit_event(
    ctx.session_id, "store_fact", &target,
    None, &after, reasoning.as_deref(),
    Some(ctx.trace_id),  // Always pass trace_id
).await?;
```

### Spawned Task Contexts (JoinSet)

```rust
// Clone trace_id before spawning
let trace_id = self.trace_id.clone();
join_set.spawn(async move {
    db.save_message_with_metadata(
        &session_id, "assistant", response,
        Some(&metadata), Some(&trace_id),  // Use cloned value
    ).await
});
```

### Unified Timeline VIEW (shared constant)

```rust
const UNIFIED_TIMELINE_VIEW_SQL: &str = "\
    CREATE VIEW IF NOT EXISTS unified_timeline AS \
    SELECT trace_id, session_id, agent_id, 'message' AS event_type, \
        role AS event_subtype, \
        CASE WHEN length(content) > 200 THEN substr(content, 1, 200) || '...' \
             ELSE content END AS summary, \
        created_at \
    FROM messages \
    UNION ALL \
    SELECT trace_id, session_id, agent_id, 'audit' AS event_type, \
        tool_name AS event_subtype, \
        target_key || ': ' || COALESCE(before_value, '(none)') || ' -> ' || after_value AS summary, \
        created_at \
    FROM audit_events \
    UNION ALL \
    SELECT created_trace_id AS trace_id, created_by_session AS session_id, agent_id, \
        'task' AS event_type, action_type AS event_subtype, \
        label || ' [' || status || ']' AS summary, \
        created_at \
    FROM tasks";
```

### Schema Migration Pattern (Idempotent)

```rust
fn migrate_v4_to_v5(&self) -> Result<()> {
    // Check table existence before rename
    let has_old: bool = self.conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memory_events'",
        [], |r| r.get(0))?;
    if has_old {
        self.conn.execute_batch("ALTER TABLE memory_events RENAME TO audit_events;")?;
    }

    // Check column existence before adding
    if !self.column_exists("messages", "trace_id")? {
        self.conn.execute_batch("ALTER TABLE messages ADD COLUMN trace_id TEXT;")?;
    }

    // Indexes are always IF NOT EXISTS
    self.conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;")?;
}
```

## Review Findings Fixed (568-574)

| # | Severity | Finding | Fix |
|---|----------|---------|-----|
| 568 | P1 | `LongRunningContext` missing trace_id; callback tasks created with `None` | Added `trace_id: String` field to `LongRunningContext`, pass `Some(ctx.trace_id)` |
| 569 | P1 | Missing `idx_tasks_trace` partial index | Added to both `migrate_v1()` and `migrate_v4_to_v5()` |
| 570 | P2 | Incomplete `memory_events` -> `audit_events` rename in identifiers/prompts/docs | Renamed all: `recent_memory_events` -> `recent_audit_events`, prompt text, XML tags, architecture.md |
| 571 | P2 | Team engine JoinSet tasks pass `None` for trace_id | Clone `self.trace_id` into spawned closure |
| 572 | P2 | Manual byte-to-hex fold in trace.rs | Simplified to `uuid::Uuid::new_v4().simple().to_string()` |
| 573 | P3 | Unified timeline VIEW SQL duplicated across migrations | Extracted to `UNIFIED_TIMELINE_VIEW_SQL` constant |
| 574 | P3 | `column_exists()` accepts `&str` (could allow dynamic input) | Changed to `&'static str` for compile-time enforcement |

## Prevention Strategies

### Convention for New Code

When adding a new tool or DB write path:

1. **Receive trace_id** — `ToolContext` already carries it as `trace_id: &'a str`
2. **Pass to ALL DB writes** — `Some(ctx.trace_id)`, never `None`
3. **Clone for async contexts** — `let trace_id = ctx.trace_id.to_string()` before `tokio::spawn` or `JoinSet`
4. **New tables** — Add `trace_id TEXT` column + partial index + add to `unified_timeline` VIEW

### Common Mistakes to Avoid

| Mistake | Example | Prevention |
|---------|---------|------------|
| Missing trace_id in spawned contexts | JoinSet closure passes `None` | Clone trace_id before spawn |
| Missing partial index on new trace_id column | Table has column but no index | Always pair column with `WHERE IS NOT NULL` index |
| Incomplete rename across codebase | Table renamed but identifiers/prompts still use old name | `grep -r` for old name after rename |
| `None` instead of `Some(ctx.trace_id)` | Defaulting to None in new write paths | Code review: search for `None` in audit/message save calls |

### Checklist for Future Schema Migrations

- [ ] Use idempotent SQL (`IF NOT EXISTS`, column existence checks)
- [ ] Update both migration paths: `migrate_v1()` (clean-slate) and `migrate_vN_to_vN+1()` (incremental)
- [ ] Bump `CURRENT_SCHEMA_VERSION`
- [ ] If adding trace_id column: add partial index + update `UNIFIED_TIMELINE_VIEW_SQL`
- [ ] Test migration idempotency (run twice without error)

## Cross-References

- **Origin brainstorm:** [docs/brainstorms/2026-03-08-orthogonal-observability-brainstorm.md](../../brainstorms/2026-03-08-orthogonal-observability-brainstorm.md)
- **Implementation plan:** [docs/plans/2026-03-08-feat-orthogonal-observability-plan.md](../../plans/2026-03-08-feat-orthogonal-observability-plan.md)
- **PR:** [#88](https://github.com/senara-solutions/mika/pull/88)
- **Related solution:** [observability-otel-tui-dashboard.md](../architecture/observability-otel-tui-dashboard.md) — OTel span integration that this builds upon
- **Related solution:** [callback-resume-agent-lifecycle.md](../architecture/callback-resume-agent-lifecycle.md) — Callback task lifecycle now carries trace_id
- **Related solution:** [otlp-endpoint-path-requirement.md](../integration-issues/otlp-endpoint-path-requirement.md) — OTLP configuration for Jaeger/Langfuse
- **ADR-004:** Multi-agent teams orchestration (team engine now carries trace_id)
