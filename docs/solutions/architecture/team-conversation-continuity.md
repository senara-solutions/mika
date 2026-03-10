---
title: Team Conversation Continuity — Previous Run Context Injection
problem_type: architecture
component: teams, db, server
severity: medium
tags:
  - team-engine
  - context-injection
  - prompt-engineering
  - orchestrator
  - conversation-continuity
symptoms:
  - Team runs start with amnesia — no knowledge of what previous runs discussed, found, or delivered
  - Orchestrator cannot follow up on pending tasks from previous runs
  - No way to build on previous deliverables or learn from previous failures
  - Dashboard cannot display previous run context alongside current run data
related_modules:
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/async_db.rs
  - crates/mika-agent/src/teams/engine.rs
  - crates/mika-agent/src/teams/prompt.rs
  - crates/mika-agent/src/server/dashboard.rs
  - crates/mika-agent/src/server/mod.rs
---

# Team Conversation Continuity — Previous Run Context Injection

## Problem

Every team run started fresh with no knowledge of the previous run. The orchestrator
had no idea what the team discussed, found, or delivered last time. This meant:

- The orchestrator couldn't follow up on incomplete work
- Pending tasks from previous runs were invisible
- Failed runs provided no learning signal
- Each run re-discovered context that was already known

## Root Cause

The `TeamEngine::decompose()` method built the orchestrator's system prompt without
any reference to historical runs. The `build_orchestrator_context()` function in
`teams/prompt.rs` only included the current run's configuration (team members, rules,
iteration count) and any conversation history from the current session.

## Solution

### Data Layer (db.rs)

Three new structs to represent the summary:

```rust
pub struct AgentResultSummary { agent_name: String, response_preview: String }
pub struct TaskStatusSummary { agent_id: String, label: String, status: String, task_id: String }
pub struct TeamRunSummary { run: TeamRunRow, agent_results: Vec<AgentResultSummary>,
    task_statuses: Vec<TaskStatusSummary>, pending_tasks: Vec<TaskStatusSummary>,
    critic_feedback: Option<String> }
```

Three new DB methods:

1. `get_last_completed_team_run(team_name)` — queries `team_runs` WHERE status IN
   ('completed', 'failed', 'suspended'), ordered by `started_at DESC LIMIT 1`
2. `get_team_run_summary(run_id)` — multi-query method that assembles agent results
   (from messages), task statuses (from tasks), critic feedback (from team_workspace),
   and pending tasks. Returns `Result<Option<TeamRunSummary>>`.
3. `get_last_completed_team_run_summary(team_name)` — convenience method combining both

### Prompt Rendering (teams/prompt.rs)

New function `build_previous_run_context(summary: &TeamRunSummary) -> String` formats
the summary as a structured block wrapped in `<context type="previous_run">` tags:

```
## Previous Team Run ({date})
**Goal:** {previous goal}
**Agent Results:** (top 5, 200 chars each)
**Deliverable:** (500 chars)
**Critic Feedback:** (if any)
**Task Status:** (all tasks with status)
**Pending from previous run:** (highlighted)
```

Total budget: 2500 chars with UTF-8-safe truncation using `is_char_boundary()`.

The enriched previous run replaces one entry from the "Older Runs" history section
(which uses a separate 1500-char budget and excludes the enriched run by ID).

### Engine Integration (teams/engine.rs)

At the start of `execute_inner()`, before decomposition:

```rust
let previous_run_summary = self
    .team_db
    .get_last_completed_team_run_summary(&self.run.team_name)
    .await
    .unwrap_or_else(|e| { debug!("Failed to load previous run: {e}"); None });
```

Passed through `decompose()` to `build_orchestrator_context()` as
`previous_run: Option<&TeamRunSummary>`.

### Dashboard API (server/dashboard.rs)

New endpoint `GET /api/v1/team-runs/:run_id/summary` returns the `TeamRunSummary`
as JSON. Uses the `Result<Option<T>>` pattern — 200 with data, 404 if not found.

## Key Design Decisions

1. **Read-only queries only** — no modifications to previous run data
2. **Context injection only** — no changes to team execution flow; the orchestrator
   decides what to do with the context
3. **Single previous run** — only the most recent, not full history
4. **Graceful degradation** — first run skips injection; DB errors log and continue
5. **Character budget, not token budget** — 2500 chars is approximately 1000 tokens,
   keeps the system prompt manageable
6. **No audit event** — this is a read-only query, not a memory mutation, so it
   doesn't belong in the audit_events table (which tracks mutations only)
7. **`truncate_chars` reuse** — made the existing `truncate_chars` helper `pub(crate)`
   instead of duplicating truncation logic

## Gotchas

- **UTF-8 truncation**: The 2500-char budget enforcement must use `is_char_boundary()`
  to avoid panicking on multi-byte characters. The initial implementation used raw byte
  offsets, caught during code review.
- **Agent result session naming**: Agent messages use session IDs formatted as
  `team-{run_id}-{agent_name}`. The query joins team_workspace assignments to find
  agent names, then queries messages by constructed session ID.
- **DB method signature**: `insert_team_run` takes `(run_id, team_name, goal,
  max_iterations, started_at)` — not a struct. Check the signature before writing tests.
- **`Result<Option<T>>` pattern**: The dashboard handler and DB methods use
  `Result<Option<TeamRunSummary>>` — `Ok(None)` means "not found", `Err` means
  "query failed". This is the project convention for queries that may not find records.

## Testing

14 new tests covering:
- DB: no runs, filtered running status, basic summary, critic feedback, not-found
- Prompt: completed run, failed run with pending tasks, empty summary, orchestrator
  context with/without previous run, history exclusion of enriched run
- Truncation: short/exact/long strings via `truncate_chars`

All 1108 tests pass.
