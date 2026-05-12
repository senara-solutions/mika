---
title: "mika agent reset <name> — wipe agent state without deleting agent"
type: feat
status: active
date: 2026-05-12
ticket: mika#964
---

# mika agent reset \<name\> — wipe agent state without deleting agent

## Overview

Add a `mika agent reset <name>` CLI subcommand that deletes all per-agent state (sessions, messages, memory, tasks, KG corpus rows, audit events, tool/LLM call history, etc.) while preserving the agent row in `agents` table, `identity.toml`, and the `~/.mika/agents/<name>/` directory layout. This enables clean-slate testing and recovery without losing per-agent customization.

## Problem Frame

Currently the closest primitive is `mika kg purge --agent <name>` — KG-scoped only. No way to also wipe sessions/messages/memory/tasks/audit/tool history. The alternative — delete + re-provision — cascades the agent row (losing `identity.toml` customization via re-bootstrap) and is unsafe for agents with in-flight tasks.

## Requirements Trace

- **R1.** New `Reset` variant in `AgentsCommand` enum (`crates/mika-cli/src/cli.rs`) with args: `name: String`, `--force` (bypass active-task guard), `--dry-run` (preview only), `--yes` (skip confirmation).
- **R2.** New `Database::reset_agent_state(agent_id: &str) -> Result<ResetAgentCounts>` method in `crates/mika-agent/src/db.rs`. Deletes rows from all 17 agent-scoped child tables in a single transaction, returns per-table deleted counts.
- **R3.** Active-task guard: refuse to reset if `tasks` table has rows with `status IN ('pending', 'in_progress', 'recurring_active')` for the agent. Error message includes the blocking task ID(s). `--force` bypasses.
- **R4.** `--dry-run` reports per-table row counts that would be deleted, without deleting.
- **R5.** Interactive confirmation prompt (agent name echo) unless `--yes` or non-TTY.
- **R6.** KG shared-corpus safety: only delete from shared KG tables (`kg_chunks`, `kg_subject_entities`, `kg_subject_relationships`, `kg_chunk_subjects`, `kg_chunk_subject_relationships`, `kg_extractions`) if no other agent references the same `docs_root_hash` via `agent_kg_corpora`. Otherwise delete only the `agent_kg_corpora` row (unlinking) and the per-agent resolution tables.
- **R7.** `skill_overrides` table: included in reset (wipe per-skill LLM overrides). Post-reset, `seed_bundled_skills()` should be called if the agent is a well-known dev-mode agent to restore bundled skill index.
- **R8.** Unit tests: empty-agent reset (idempotent), populated-agent reset (all child tables zeroed, agent row + identity preserved), active-task guard (refuses with structured error).

## Scope Boundaries

### In scope

- `mika agent reset <name>` subcommand (CLI only)
- `Database::reset_agent_state()` method
- Active-task guard with `--force` override
- `--dry-run` flag (count-only preview)
- `--yes` flag (skip confirmation)
- KG shared-corpus safety (don't delete shared tables if other agents reference the hash)
- Unit tests for the DB method

### Out of scope

- `mika agent delete <name>` (separate concept — removes agent entirely)
- Reset-via-API endpoint (CLI only for v1)
- Audit-log preservation flag (defer to follow-up)
- Cross-agent shared `kg_chunks` rows where `docs_root_hash` is shared — preserve those per R6

### Deferred to separate tasks

- `--kg-only` flag to make `mika agent reset` subsume `mika kg purge --agent` (ticket says this but it's additive, defer)

## Implementation Plan

### Step 1: Add `ResetAgentCounts` struct and `Database::reset_agent_state()` (db.rs)

**File:** `crates/mika-agent/src/db.rs`

Add a `ResetAgentCounts` struct with per-table deleted counts:

```rust
pub struct ResetAgentCounts {
    pub sessions: usize,
    pub messages: usize,
    pub core_memory: usize,
    pub llm_calls: usize,
    pub tool_calls: usize,
    pub audit_events: usize,
    pub audit_event_summaries: usize,
    pub people: usize,
    pub commitments: usize,
    pub preferences: usize,
    pub events: usize,
    pub search_content: usize,
    pub tasks: usize,
    pub kg_subject_resolutions: usize,
    pub kg_resolutions_log: usize,
    pub agent_kg_corpora: usize,
    pub kg_invalidated_no_match: usize,
    pub skill_overrides: usize,
    // Shared KG tables (only if no other agent shares the corpus)
    pub kg_chunks: usize,
    pub kg_subject_entities: usize,
    pub kg_subject_relationships: usize,
    pub kg_chunk_subjects: usize,
    pub kg_chunk_subject_relationships: usize,
    pub kg_extractions: usize,
}
```

Implement `reset_agent_state(&self, agent_id: &str) -> Result<ResetAgentCounts>`:

1. Begin transaction.
2. Verify agent exists: `SELECT id FROM agents WHERE id = ?`.
3. Determine KG shared-corpus status: query `agent_kg_corpora` for the agent's `docs_root_hash` values, then check if any other agent references the same hashes.
4. Delete from all 17 agent-scoped tables with direct `agent_id` FK: `DELETE FROM <table> WHERE agent_id = ?`. Order doesn't matter within a transaction since we're deleting by `agent_id`, not cascading from agent row.
5. For shared KG tables: if no other agent shares the `docs_root_hash`, delete rows matching the hash. Otherwise, skip and return 0 for those counts.
6. Commit transaction, return `ResetAgentCounts`.

Also add `count_agent_state(&self, agent_id: &str) -> Result<ResetAgentCounts>` for dry-run — same logic but `SELECT COUNT(*)` instead of `DELETE`.

Also add `get_active_tasks_for_agent(&self, agent_id: &str) -> Result<Vec<(String, String)>>` returning `(task_id, status)` pairs for the active-task guard (if not already exposed).

### Step 2: Add `Reset` variant to CLI (cli.rs)

**File:** `crates/mika-cli/src/cli.rs`

Add to `AgentsCommand` enum:

```rust
/// Wipe all state for an agent without deleting the agent itself
Reset {
    /// Agent name to reset
    name: String,
    /// Skip active-task safety check
    #[arg(long)]
    force: bool,
    /// Preview what would be deleted without deleting
    #[arg(long)]
    dry_run: bool,
    /// Skip confirmation prompt
    #[arg(long, short)]
    yes: bool,
},
```

### Step 3: Implement handler (agents.rs)

**File:** `crates/mika-cli/src/commands/agents.rs`

Add `AgentsCommand::Reset { name, force, dry_run, yes }` match arm, calling a new `fn reset()`:

1. Resolve agent: validate name exists in DB via existing pattern (match `kg.rs:run_purge()`).
2. Active-task guard: query `tasks` for `status IN ('pending', 'in_progress', 'recurring_active')` where `agent_id` matches. If any found and `!force`, print error with task IDs and suggest `--force`. Return error.
3. Dry-run path: call `db.count_agent_state(agent_id)` and display per-table counts. Return.
4. Confirmation: unless `--yes` or non-TTY, prompt user to type agent name to confirm (follow `kg purge` pattern).
5. Call `db.reset_agent_state(agent_id)`.
6. Display per-table deleted counts summary.
7. If agent is a well-known dev-mode agent and `MIKA_DEV_MODE` is set, note that `seed_bundled_skills()` will restore skill indexes on next startup (no inline call needed — startup handles it).

### Step 4: Unit tests (db.rs)

**File:** `crates/mika-agent/src/db.rs` (in `#[cfg(test)] mod tests`)

Three test cases:

1. **`test_reset_agent_empty`** — Create agent, immediately reset. All counts should be 0. Agent row still exists. Idempotent (reset again, still 0).
2. **`test_reset_agent_populated`** — Create agent, insert rows into sessions, messages, core_memory, tasks, people, etc. Reset. Verify all child table counts are 0 via `count_agent_state`. Verify agent row still exists.
3. **`test_reset_agent_active_task_guard`** — Create agent, insert a task with `status = 'in_progress'`. Verify `get_active_tasks_for_agent` returns non-empty. (The guard logic lives in the CLI handler, but the DB query should be tested.)

## Table Deletion Order

Within the transaction, deletion order is irrelevant because we delete by `agent_id` column, not by cascading from a parent row. All 17 direct-FK tables can be deleted in any order. For readability, group by category:

1. **Conversation:** sessions, messages, llm_calls, tool_calls
2. **Memory:** core_memory, people, commitments, preferences, events, search_content
3. **Audit:** audit_events, audit_event_summaries
4. **Tasks:** tasks
5. **KG per-agent:** kg_subject_resolutions, kg_resolutions_log, agent_kg_corpora, kg_invalidated_no_match
6. **KG shared (conditional):** kg_chunks, kg_subject_entities, kg_subject_relationships, kg_chunk_subjects, kg_chunk_subject_relationships, kg_extractions
7. **Skills:** skill_overrides

## Risk Assessment

- **Low risk:** Pure additive — new subcommand, new DB method. No existing behavior changes.
- **KG shared-corpus safety** is the main complexity. Follows the proven pattern from `purge_kg_for_agent()` — check `agent_kg_corpora` for other references before deleting shared tables.
- **Foreign key cascades:** Since `ON DELETE CASCADE` is on `agents(id)`, deleting child rows directly (not the agent row) won't trigger cascades. This is the intended behavior — we want surgical per-table deletes.

## Acceptance Criteria Mapping

| Ticket AC | Plan Step | Verification |
|-----------|-----------|-------------|
| `--dry-run` reports rows-by-table | Step 1 (`count_agent_state`) + Step 3 (dry-run path) | Manual test: `mika agent reset mika-test --dry-run` |
| `--yes` returns agent to fresh state | Step 1 (`reset_agent_state`) + Step 3 | Manual test: `mika agent reset mika-test --yes` |
| Active-task guard with `--force` | Step 3 (guard logic) + Step 4 (unit test) | Unit test + manual test during dispatch |
| Unit tests | Step 4 | `cargo test -p mika-agent reset_agent` |
