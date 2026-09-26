# Brainstorm: Orthogonal Observability Design

**Date:** 2026-03-08
**Status:** Ready for planning

## What We're Building

A unified observability architecture for Mika where every subsystem shares a common correlation language. Two orthogonal axes thread through all events:

- **Request axis (`trace_id`):** One per agent turn (one user message → one trace_id) — correlates with Jaeger/Langfuse spans
- **System axis (`session_id` + `agent_id`):** Groups events within a conversation and by actor

Today, Mika has 5+ independent event systems (memory_events, messages.metadata, tracing spans, TeamEvent, task engine) that don't share structure or correlation IDs. This makes it impossible to answer "what happened during this agent turn?" without querying each system separately.

Adding btrfs snapshot correlation as a 6th dimension — Mika-triggered snapshots tagged with trace_id at defined points (before agent turns, before destructive file ops).

## Why This Approach

### The Problem

Each subsystem evolved independently:

| System | Captures | Correlation | Storage |
|---|---|---|---|
| `memory_events` | Fact/memory mutations | session_id, agent_id | SQLite table |
| `messages.metadata` | Tool call summaries (JSON) | session_id (via message) | Embedded in messages |
| `tracing` spans | agent_turn, team_run | trace_id (OTel) | Jaeger/OTLP export |
| `TeamEvent` enum | Phase changes, agent status | team_run_id | Callback + team_workspace |
| Task engine | Status transitions | agent_id | SQLite tasks table |
| btrfs snapshots | Filesystem changes | timestamp (unlinked) | btrfs subvolumes |

No shared envelope. No way to JOIN across them. No trace_id in the database. No session_id on tasks.

### The Insight

Traces and metrics are abstractions built on logs along two orthogonal axes: request-centric (trace) and system-centric (metric). Mika already has both axes partially — OTel trace_id exists in spans, session_id exists on messages. The gap is threading them consistently through all subsystems.

### Event Sourcing as Motivation

Agent systems are inherently event-driven (LLM calls, tool executions, human-in-the-loop). The correlation architecture lays groundwork for event sourcing by ensuring every mutation is traceable. Full event replay is out of scope for now, but the immediate benefits are:

- **Audit completeness:** Immutable log of every mutation with causal chain
- **Debugging:** Follow one request across all layers via trace_id
- **Future option:** Correlation IDs enable event replay/rehydration later without another migration

## Key Decisions

### 1. Hybrid enforcement (schema migration + EventContext struct)

**Decision:** Add `trace_id` and `session_id` columns to existing tables via schema migration. Define a shared `EventContext` struct for new code.

```rust
struct EventContext {
    trace_id: String,
    session_id: String,
    agent_id: String,
}
```

**Rationale:** Pure struct enforcement means rewriting every write path today. Pure schema convention means fields stay null forever because someone forgot. Hybrid gives compiler enforcement going forward and pragmatic migration for existing code. Functions that write to the DB take `&EventContext` — the compiler won't let you skip it.

### 2. SQLite VIEW for unified timeline

**Decision:** `CREATE VIEW unified_timeline` that UNIONs across messages, audit_events (renamed memory_events), and tasks. External events (Jaeger spans, btrfs snapshots) stay in their native systems, correlated by trace_id.

**Rationale:** Zero storage overhead, immediately queryable, no new write paths. When you need cross-system correlation (e.g., match a Jaeger span to a DB event), query by trace_id in both systems. No need to duplicate Jaeger data into SQLite.

### 3. Mika-triggered btrfs snapshots

**Decision:** Mika creates btrfs snapshots at defined points (before agent turns, before destructive file operations) and tags snapshot names with trace_id.

**Rationale:** Full control and deterministic correlation. No timestamp-matching heuristics. `btrfs diff` between two trace_id-tagged snapshots shows exactly what an agent turn changed on the filesystem.

**Constraint:** Requires the agent's home directory to reside on a btrfs filesystem. Snapshot creation requires `CAP_SYS_ADMIN` or root privileges. This feature is deployment-dependent — graceful degradation when btrfs is unavailable.

### 4. Fold memory_events rename into this migration

**Decision:** Rename `memory_events` to `audit_events` and `memory_event_summaries` to `audit_event_summaries` as part of the same schema migration (closes #87).

**Rationale:** We're already doing a breaking schema migration. One migration, one clean break. The rename reflects reality — this table tracks all state-mutating tool calls, not just memory operations.

### 5. trace_id sourcing

**Decision:** Extract trace_id from the current OTel span context. When OTel is disabled (no `telemetry` feature), generate a UUID-based trace_id locally. Either way, every agent turn has a trace_id.

**Rationale:** OTel trace_id is the industry standard. But Mika must work without OTel enabled. Local UUID fallback ensures the correlation axis always exists, even without external tracing infrastructure.

## What Changes (by subsystem)

### Schema migration (v4)

- `memory_events` renamed to `audit_events`, add `trace_id TEXT` column
- `memory_event_summaries` renamed to `audit_event_summaries`
- `tasks` table: add `trace_id TEXT`, add `session_id TEXT`
- `messages` table: add `trace_id TEXT` as a dedicated column (not in metadata JSON — must be directly indexable for the VIEW)
- `team_workspace` table: add `trace_id TEXT`
- Create `unified_timeline` VIEW

### New code

- `EventContext` struct in `mika-agent`
- `trace_id` generation/extraction utility (OTel span → trace_id, or UUID fallback)
- btrfs snapshot integration (trigger + tag with trace_id)
- Incremental migration of existing write paths to take `&EventContext`

### Existing code (incremental)

- Agent loop: create `EventContext` at turn start, pass through tool execution
- Tool handlers: receive `EventContext` via `ToolContext`, pass to DB writes
- Task engine: populate trace_id and session_id on task creation
- Team engine: thread trace_id through TeamEvent and team_workspace writes

## Scope Boundaries

### In scope

- EventContext struct and trace_id generation
- Schema migration v4 (column additions, table renames, VIEW)
- Threading EventContext through agent loop and tool execution
- btrfs snapshot trigger points with trace_id tagging

### Out of scope (future)

- Event replay/rehydration engine
- Real-time event streaming to TUI
- Cross-container event correlation (gateway ↔ agent)
- Metrics derivation from the event stream
- Event retention policies beyond current 30-day task pruning

## Open Questions

None — all key decisions resolved during brainstorming.
