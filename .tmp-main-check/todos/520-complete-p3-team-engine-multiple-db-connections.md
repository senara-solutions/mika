---
status: complete
priority: p3
issue_id: 520
tags: [code-review, architecture, database]
dependencies: []
---

# TeamEngine Opens N+1 Database Connections Per Team Run

## Problem Statement

When a team run executes, the system opens multiple SQLite connections to the same container database:
1. The caller (run_team tool or CLI) opens one connection for `team_db`
2. `TeamEngine::new()` opens one connection per agent in the team

For a 3-agent team, this means 4 OS threads running concurrently against the same WAL-mode SQLite file. This works correctly (WAL supports concurrent readers, busy_timeout handles write contention) but is wasteful.

**Severity:** P3 — Works correctly, but creates unnecessary OS threads and file handles.

## Findings

- `crates/mika-agent/src/teams/engine.rs:78-82` — Opens one `Database::open` per agent
- `crates/mika-agent/src/tools/run_team.rs:82-83` — Opens one for team_db
- `crates/mika-cli/src/commands/teams.rs:40-41` — Opens one for CLI team_db
- `AsyncDatabase::shutdown()` kills the inner thread — cannot share clones across shutdown boundaries

## Proposed Solutions

### Option A: Accept current behavior (recommended)
- **Pros:** Simple, correct, WAL handles concurrency
- **Cons:** N+1 OS threads per team run
- **Effort:** None
- **Risk:** None

### Option B: Pass shared AsyncDatabase to TeamEngine
- Requires rethinking shutdown semantics (can't shutdown shared handles)
- Would need `with_agent()` clones instead of `new_with_agent()`
- **Pros:** Single OS thread for all DB ops
- **Cons:** Requires AsyncDatabase refactor to separate shutdown lifecycle
- **Effort:** Medium
- **Risk:** Low (but touches core infrastructure)

## Acceptance Criteria

- [ ] Document the connection multiplicity in engine.rs comments
- [ ] OR implement Option B to share a single connection

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-06 | Created during DB consolidation review | WAL mode handles N connections correctly; shutdown semantics prevent simple sharing |
