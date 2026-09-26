---
title: "feat: Add team conversation continuity"
type: feat
status: completed
date: 2026-03-10
---

# feat: Add Team Conversation Continuity

## Overview

Enrich the orchestrator's system prompt with structured context from the previous team run, so that consecutive runs for the same team build on prior work instead of starting with amnesia. This is a **context injection only** change — no modifications to the team execution flow.

## Problem Statement

Every team run starts fresh. The orchestrator has no memory of what the team discussed, found, or delivered last time. This forces users to re-explain context and prevents iterative multi-run workflows where teams refine work across sessions.

## Current State

History injection already exists in `build_orchestrator_context()` (`crates/mika-agent/src/teams/prompt.rs:32-52`). It uses `load_team_runs_for_prompt()` (`crates/mika-agent/src/db.rs:2780`) which loads up to 10 runs with only `goal` and `deliverable` (truncated to 500 chars), rendered as `<context type="history_goal">` and `<context type="history_deliverable">` XML tags within a 5000-character budget.

**What's missing:** Agent results, task statuses, pending work, critic feedback, and iteration count.

## Proposed Solution

Replace the current thin goal+deliverable history with a richer `TeamRunSummary` for the most recent run, while keeping condensed goal+status entries for older runs within a unified character budget.

### Injected Context Format

```
## Previous Team Run (2026-03-09, completed in 2/3 iterations)

**Goal:** {previous goal}

**Agent Results:**
- {agent_name}: {truncated response, ~200 chars}
- {agent_name}: {truncated response, ~200 chars}

**Deliverable:** {previous deliverable, ~500 chars}

**Task Status:**
- [completed] {agent}: {task label}
- [failed] {agent}: {task label}

**Pending from previous run:**
- [pending] researcher: literature review (task abc123)
```

## Technical Approach

### Phase 1: Data Layer — New Struct and DB Methods

**New struct `TeamRunSummary`** in `crates/mika-agent/src/db.rs`:

```rust
// crates/mika-agent/src/db.rs
pub struct AgentResultSummary {
    pub agent_name: String,
    pub response_preview: String, // truncated to 200 chars
}

pub struct TaskStatusSummary {
    pub agent_id: String,
    pub label: String,
    pub status: String,
    pub task_id: String,
}

pub struct TeamRunSummary {
    pub run: TeamRunRow,
    pub agent_results: Vec<AgentResultSummary>,
    pub task_statuses: Vec<TaskStatusSummary>,
    pub pending_tasks: Vec<TaskStatusSummary>,
    pub critic_feedback: Option<String>, // final iteration only, truncated to 200 chars
}
```

**New sync DB method `get_team_run_summary(run_id: &str) -> Result<TeamRunSummary>`** in `crates/mika-agent/src/db.rs`:

1. Load the `TeamRunRow` via existing `load_team_run_by_id()` (line 2812)
2. Query agent results from `messages` table via constructed session IDs (`team-{run_id}-{agent_name}`):
   - Get agent names from `team_workspace WHERE run_id = ? AND entry_type = 'assignment'`
   - For each agent, query `messages WHERE session_id = 'team-{run_id}-{agent_name}' AND role = 'assistant' ORDER BY created_at DESC LIMIT 1`
   - Truncate `content` to 200 chars
3. Query tasks: `SELECT agent_id, label, status, id FROM tasks WHERE team_run_id = ?`
   - Split into completed/failed vs pending/in_progress
4. Query critic feedback: `SELECT content FROM team_workspace WHERE run_id = ? AND entry_type = 'critic' ORDER BY iteration DESC, created_at DESC LIMIT 1`
   - Truncate to 200 chars

**New async wrapper** in `crates/mika-agent/src/db/async_db.rs`:
- `get_team_run_summary(run_id: String) -> Result<TeamRunSummary>`

**New method `get_last_completed_team_run(team_name: &str) -> Result<Option<TeamRunRow>>`** in `crates/mika-agent/src/db.rs`:
- `SELECT * FROM team_runs WHERE team_id IN (SELECT id FROM teams WHERE name = ?1 COLLATE NOCASE) AND status IN ('completed', 'failed', 'suspended') ORDER BY started_at DESC LIMIT 1`
- Excludes `running` and `cancelled` runs (incomplete/abandoned data)

**Async wrapper** in `async_db.rs`:
- `get_last_completed_team_run(team_name: String) -> Result<Option<TeamRunRow>>`

### Phase 2: Prompt Rendering — Enrich `build_orchestrator_context()`

**Modify `build_orchestrator_context()`** in `crates/mika-agent/src/teams/prompt.rs` (line 14):

1. Change signature to accept `previous_run: Option<&TeamRunSummary>` instead of (or in addition to) `history: &[TeamRunRow]`
2. If `previous_run` is `Some`, render the structured "Previous Team Run" block (format above)
3. Keep remaining older runs as condensed `<context type="history_goal">` + one-line status
4. **Budget:** 4000-character total for all history (replaces existing 5000-char budget). The enriched summary for the most recent run gets up to 2500 chars; remaining 1500 chars for older run summaries.
5. Agent results capped at 5 agents (sorted by response length descending — longest responses are most informative)
6. Use `<context type="previous_run">` wrapper tag (consistent with existing `<context type="history_...">` pattern, no `trust="untrusted"` — this is system-generated content)

**New function `build_previous_run_context(summary: &TeamRunSummary) -> String`** in `prompt.rs`:
- Renders the structured markdown block
- Enforces per-field truncation: goal 200 chars, agent results 200 chars each (max 5), deliverable 500 chars, critic 200 chars
- Returns empty string if summary has no meaningful content

### Phase 3: Engine Integration — Wire It Up

**Modify `execute_inner()`** in `crates/mika-agent/src/teams/engine.rs` (line 488):

1. After loading `history` via `load_team_runs_for_prompt()` (line 490), add:
   ```rust
   let previous_summary = self.team_db
       .get_last_completed_team_run(&self.run.team_name)
       .await
       .ok()
       .flatten();

   let previous_run_summary = match &previous_summary {
       Some(prev) => self.team_db
           .get_team_run_summary(&prev.id)
           .await
           .ok(),
       None => None,
   };
   ```
2. Pass `previous_run_summary.as_ref()` to `build_orchestrator_context()` calls (lines ~500 and ~549)
3. Filter `history` to exclude the most recent run (already covered by the enriched summary) to avoid duplication

### Phase 4: Observability — Audit Event and Tracing

**Audit event** — after building the previous run context in `execute_inner()`:

```rust
if previous_run_summary.is_some() {
    self.team_db.log_audit_event(
        &orchestrator_agent_id,
        &team_session_id,
        "team_context_injection",    // tool_name
        &format!("team_run:{}", self.run.id),  // target_key
        None,                         // before_value
        Some(&format!("previous_run_id={}", prev.id)),  // after_value
        "Injected previous team run context into orchestrator prompt",
        &self.trace_id,
    ).await.ok();
}
```

**Tracing span** — add `tracing::info!` with structured fields:

```rust
tracing::info!(
    previous_run_id = %prev.id,
    previous_run_status = %prev.status,
    agent_results_count = previous_run_summary.agent_results.len(),
    context_chars = rendered_context.len(),
    "Injected previous team run context"
);
```

### Phase 5: Dashboard API — Summary Endpoint

**New endpoint** `GET /api/v1/team-runs/:run_id/summary` in `crates/mika-agent/src/server/dashboard.rs`:

```rust
async fn handle_team_run_summary(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<TeamRunSummary>, StatusCode> {
    // Reuse get_team_run_summary from db layer
}
```

- Register route alongside existing team-run endpoints (dashboard.rs line ~86)
- Returns `TeamRunSummary` as JSON
- Same auth as other dashboard routes (dashboard token or internal token)

## System-Wide Impact

- **Interaction graph**: `execute_inner()` → `get_last_completed_team_run()` → `get_team_run_summary()` → `build_orchestrator_context()` → `decompose()`. No callbacks, middleware, or observers affected.
- **Error propagation**: All new DB queries use `.ok()` / `.unwrap_or_default()` — failures gracefully degrade to no previous context (existing behavior).
- **State lifecycle risks**: None — all queries are read-only against existing data. No state mutations.
- **API surface parity**: The `run_team` tool, TUI team mode, and server dispatcher all enter through `execute_inner()` or `execute_from_phase()`. Only `execute_inner()` gets the enrichment (resume path is out of scope).
- **Integration test scenarios**: (1) Team run with previous completed run verifies context injection. (2) First team run verifies no context injected. (3) Previous failed run verifies failure context included. (4) Dashboard API returns correct summary.

## Acceptance Criteria

- [x] `get_last_completed_team_run()` returns the most recent non-running, non-cancelled run for a team
- [x] `get_team_run_summary()` returns enriched data: agent results, task statuses, pending tasks, critic feedback
- [x] `build_previous_run_context()` renders the structured markdown block within 2500-char budget
- [x] `build_orchestrator_context()` includes the enriched previous run context when available
- [x] First team run (no history) works unchanged — no context injected
- [x] Failed/suspended previous runs are included with their status clearly labeled
- [x] Pending tasks from previous run are highlighted in a separate section
- [x] Agent results capped at 5, each truncated to 200 chars
- [x] Deliverable truncated to 500 chars
- [x] Audit event logged with `tool_name="team_context_injection"` when context is injected
- [x] `GET /api/v1/team-runs/:run_id/summary` returns `TeamRunSummary` JSON
- [x] All existing team engine tests pass unchanged
- [x] New unit tests for `get_team_run_summary()`, `build_previous_run_context()`, and the dashboard endpoint

## Dependencies & Risks

**Dependencies:**
- Existing `team_workspace`, `messages`, and `tasks` table schemas (no migration needed)
- Existing `build_orchestrator_context()` prompt builder

**Risks:**
- **Token budget tension**: The existing 5000-char history budget is replaced with a 4000-char budget. If teams rely on seeing 10 runs of goal+deliverable history, reducing this could be a regression. Mitigated by the richer context from the most recent run being more useful than thin summaries of many runs.
- **Query performance**: Agent results require multi-step queries through constructed session IDs. Mitigated by limiting to 5 agents and one message per agent. No new indexes needed for the expected volume.
- **Audit table semantics**: `team_context_injection` is the first non-mutation audit event. This slightly stretches the `audit_events` table's semantic contract. Acceptable given the spec requirement; can revisit if it causes noise in the audit stream.

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` | Add `TeamRunSummary`, `AgentResultSummary`, `TaskStatusSummary` structs; add `get_team_run_summary()`, `get_last_completed_team_run()` methods |
| `crates/mika-agent/src/db/async_db.rs` | Add async wrappers for the two new methods |
| `crates/mika-agent/src/teams/prompt.rs` | Add `build_previous_run_context()`; modify `build_orchestrator_context()` signature and rendering |
| `crates/mika-agent/src/teams/engine.rs` | Wire up previous run query and context injection in `execute_inner()` |
| `crates/mika-agent/src/server/dashboard.rs` | Add `handle_team_run_summary` handler and route |
| `crates/mika-agent/src/server/mod.rs` | Register the new route |

## Out of Scope

- **Resume path enrichment**: `execute_from_phase()` does not get previous run context (can be added later)
- **Multi-run history**: Only the most recent previous run is enriched; older runs keep condensed format
- **Dashboard UI changes**: The API endpoint is added but frontend rendering is a separate task
- **Schema migration**: No new tables, columns, or indexes needed

## Sources & References

- Similar pattern: `build_orchestrator_context()` history rendering — `crates/mika-agent/src/teams/prompt.rs:32-52`
- Existing DB methods: `load_team_runs_for_prompt()` — `crates/mika-agent/src/db.rs:2780`
- Team workspace schema — `crates/mika-agent/src/db.rs:728-739`
- Trust boundary pattern: `<callback_result trust="untrusted">` — `crates/mika-agent/src/agent.rs`
- Prompt injection learnings: `docs/solutions/logic-errors/team-engine-code-review-findings-batch.md`
- Task tree patterns: `docs/solutions/database-issues/team-task-child-wrong-agent-id.md`
- ADR-004: Multi-agent teams orchestration — `docs/adr/004-multi-agent-teams-orchestration.md`
