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
- **R2.** New `Database::reset_agent_state(agent_id: &str) -> Result<ResetAgentCounts>` method in `crates/mika-agent/src/db.rs`. Deletes rows from all 22 agent-scoped child tables in a single transaction, returns per-table deleted counts. Rebuilds FTS5 `fts_search` index post-commit.
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

## Pinned Sources (mika-arch F1 — schema verification)

**Schema version:** v34 (current as of 2026-05-12).

**Verification method:** `grep 'agent_id TEXT NOT NULL REFERENCES agents' crates/mika-agent/src/db.rs` in the v1 migration (`migrate_v1`), cross-referenced with table rebuilds in later migrations. Every table with `agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE` is listed below.

### Complete agent-scoped table inventory (22 tables with direct `agent_id` FK)

| # | Table | Category | Notes |
|---|-------|----------|-------|
| 1 | `sessions` | Conversation | |
| 2 | `messages` | Conversation | FK to `sessions(id)` — no self-referential FK |
| 3 | `core_memory` | Memory | PK is `(agent_id, key)` |
| 4 | `llm_calls` | Observability | `agent_id TEXT NOT NULL` (no FK constraint, but agent-scoped) |
| 5 | `tool_calls` | Observability | `agent_id TEXT NOT NULL` (no FK constraint, but agent-scoped) |
| 6 | `audit_events` | Audit | |
| 7 | `audit_event_summaries` | Audit | |
| 8 | `people` | Memory/Facts | |
| 9 | `commitments` | Memory/Facts | |
| 10 | `preferences` | Memory/Facts | PK is `(agent_id, category)` |
| 11 | `events` | Memory/Facts | |
| 12 | `search_content` | Search | Regular table (NOT FTS5 virtual) — see FTS5 note below |
| 13 | `tasks` | Task Engine | Has `parent_task_id` self-ref FK with ON DELETE CASCADE |
| 14 | `kg_subject_resolutions` | KG per-agent | |
| 15 | `kg_resolutions_log` | KG per-agent | |
| 16 | `agent_kg_corpora` | KG per-agent | Maps agent to `docs_root_hash` |
| 17 | `kg_invalidated_no_match` | KG per-agent | v32 sidecar table |
| 18 | `skill_overrides` | Skills | PK is `(agent_id, skill_name)` |
| 19 | `heartbeat_sends` | Operations | Heartbeat dispatch tracking |
| 20 | `reflection_runs` | Operations | Reflection execution log |
| 21 | `customer_config` | Config | PK is `(agent_id, key)` |
| 22 | `failed_sends` | Operations | Outbound message retry queue |

**Tables NOT in this list (verified absent):**
- No `reminders` table exists — reminders are stored as `tasks` rows with `trigger_type='reminder'`.
- `team_workspace` and `team_runs` FK to `teams(id)`, not `agents(id)` — excluded from agent reset scope.
- `llm_calls` and `tool_calls` use bare `agent_id TEXT NOT NULL` without REFERENCES — but are agent-scoped and must be included.

### FTS5 cleanup (mika-arch F2 — resolved)

`search_content` is a **regular table** (not an FTS5 virtual table):

```sql
CREATE TABLE search_content (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    source_id INTEGER,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
```

The FTS5 virtual table `fts_search` uses **external content mode** pointing at `search_content`:

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS fts_search
    USING fts5(content, content='search_content', content_rowid='id');
```

**Cleanup sequence:** `DELETE FROM search_content WHERE agent_id = ?` removes the base table rows. Since `fts_search` is an external-content FTS5 table with `content='search_content'`, the FTS index entries become stale but do not auto-delete. We must explicitly rebuild the FTS index after deleting base rows:

```sql
-- After deleting from search_content:
INSERT INTO fts_search(fts_search) VALUES('rebuild');
```

The `'rebuild'` command re-scans the content table and reconstructs the entire FTS index. This is safe and correct for a reset operation (the content table is now empty for the target agent, so the rebuilt index will simply not contain those entries).

**Alternative:** The per-row `INSERT INTO fts_search(fts_search, rowid, content) VALUES('delete', ?, ?)` form exists but requires knowing the old content values at delete time. The `'rebuild'` approach is simpler and correct for bulk operations.

### Precedent: `purge_kg_for_agent()` (db.rs:9405)

Follows the same pattern: single `unchecked_transaction()`, per-table deletes with `agent_id` filter for per-agent tables, `docs_root_hash` filter for shared tables, commit, return counts struct.

Key difference from `purge_kg_for_agent()`: reset covers ALL agent state (22+ tables), while purge covers only KG tables (4 total). The shared-corpus safety check is identical — query `agent_kg_corpora` to find the agent's hashes, check if other agents reference the same hashes.

### CLI enum: `AgentsCommand` (cli.rs:259-314)

Current variants: `List`, `Create`, `Delete`, `Switch`, `Clone`, `Validate`. New `Reset` variant follows the same pattern.

### Transaction size estimate (mika-arch F3 — resolved)

**Worst-case agent: mika-arch** — ~30k KG subject entities, ~10k tool_calls, ~5k messages, ~5k llm_calls, ~30k search_content rows, ~30k audit_events. Total: ~110k rows across all tables.

**SQLite performance:** DELETE with an indexed WHERE clause processes ~100k rows in <1s on modern hardware (SSD). The write lock duration is dominated by WAL journaling. For 110k rows: estimated <2s total transaction time.

**Decision: accept single transaction.** The <2s estimate is well under the 5s engine poll interval. The active-task guard ensures no dispatches are running. Concurrent read queries (dashboard, etc.) are unaffected in WAL mode. The operation is idempotent — if interrupted, re-run completes the reset.

**If the estimate proves wrong in practice:** the chunked-delete fallback (per-category transactions) is trivial to implement. But we start with the simpler single-transaction approach.

### Post-reset zero-session state (mika-arch F6 — resolved)

Session creation is on-demand: `create_session()` / `create_session_if_not_exists()` is called at the start of every new conversation. The CLI chat handler creates a fresh session UUID before entering the agent loop. The server `handle_message` creates a session per incoming message. No code path assumes a pre-existing session. Zero sessions after reset is safe.

### `skill_overrides` recovery (mika-arch F7 — resolved)

`seed_bundled_skills()` runs on **every** startup unconditionally (not gated on "agent has no skills"). It writes skill files to `~/.mika/agents/<name>/skills/` for every bundled skill. This is the file-level skill index — it's always restored.

**Custom `skill_overrides` DB rows are permanently lost.** This includes operator-set model routing overrides (e.g., Sonnet 4.6 overrides on mika-arch skills). The CLI output will document this: "Custom skill overrides (LLM model routing) have been cleared. Re-apply via `mika skills llm set` if needed."

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
    pub heartbeat_sends: usize,
    pub reflection_runs: usize,
    pub customer_config: usize,
    pub failed_sends: usize,
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

1. Begin transaction via `unchecked_transaction()`.
2. Verify agent exists: `SELECT id FROM agents WHERE id = ?`.
3. Determine KG shared-corpus status: query `agent_kg_corpora` for the agent's `docs_root_hash` values, then check if any other agent references the same hashes.
4. Delete from all 22 agent-scoped tables: `DELETE FROM <table> WHERE agent_id = ?`. Order doesn't matter within a transaction since we're deleting by `agent_id`, not cascading from agent row. `tasks` self-referential FK is safe — SQLite's `ON DELETE CASCADE` on `parent_task_id` handles child tasks within the same agent. `messages` has no self-referential FK.
5. For shared KG tables: if no other agent shares the `docs_root_hash`, delete rows matching the hash (follow `purge_kg_for_agent()` FK-safe order: chunk_subject_relationships → chunk_subjects → subject_relationships → subject_entities → extractions → chunks). Otherwise, skip and return 0 for those counts.
6. Commit transaction.
7. **After commit:** Rebuild FTS index: `INSERT INTO fts_search(fts_search) VALUES('rebuild')`. This must be outside the transaction because FTS5 virtual table operations can conflict with active transactions.
8. Return `ResetAgentCounts`.

Also add `count_agent_state(&self, agent_id: &str) -> Result<ResetAgentCounts>` for dry-run — same logic but `SELECT COUNT(*) FROM <table> WHERE agent_id = ?` for each table.

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
7. Print note: "Custom skill overrides (LLM model routing) have been cleared. Re-apply via `mika skills llm set` if needed."
8. Print note: "Bundled skills will be restored on next startup."

### Step 4: Unit tests (db.rs)

**File:** `crates/mika-agent/src/db.rs` (in `#[cfg(test)] mod tests`)

Three test cases:

1. **`test_reset_agent_empty`** — Create agent, immediately reset. All counts should be 0. Agent row still exists. Idempotent (reset again, still 0).
2. **`test_reset_agent_populated`** — Create agent, insert rows into sessions, messages, core_memory, tasks, people, heartbeat_sends, reflection_runs, customer_config, failed_sends, etc. Reset. Verify all child table counts are 0 via `count_agent_state`. Verify agent row still exists.
3. **`test_reset_agent_active_task_guard`** — Create agent, insert a task with `status = 'in_progress'`. Verify `get_active_tasks_for_agent` returns non-empty. (The guard logic lives in the CLI handler, but the DB query should be tested.)

## Table Deletion Order

Within the transaction, deletion order is irrelevant for most tables because we delete by `agent_id` column, not by cascading from a parent row. The `tasks` table has a self-referential `parent_task_id` FK with `ON DELETE CASCADE`, so deleting parent tasks cascades to children — safe since both parent and children share the same `agent_id`.

For readability, group by category (all 22 direct + 6 conditional):

1. **Conversation:** sessions, messages, llm_calls, tool_calls
2. **Memory:** core_memory, people, commitments, preferences, events, search_content
3. **Audit:** audit_events, audit_event_summaries
4. **Tasks:** tasks
5. **Operations:** heartbeat_sends, reflection_runs, customer_config, failed_sends
6. **KG per-agent:** kg_subject_resolutions, kg_resolutions_log, agent_kg_corpora, kg_invalidated_no_match
7. **KG shared (conditional — FK-safe order):** kg_chunk_subject_relationships, kg_chunk_subjects, kg_subject_relationships, kg_subject_entities, kg_extractions, kg_chunks
8. **Skills:** skill_overrides
9. **Post-transaction:** FTS5 `fts_search` rebuild

## Risk Assessment

- **Low risk:** Pure additive — new subcommand, new DB method. No existing behavior changes.
- **KG shared-corpus safety** is the main complexity. Follows the proven pattern from `purge_kg_for_agent()` — check `agent_kg_corpora` for other references before deleting shared tables.
- **Foreign key cascades:** Since `ON DELETE CASCADE` is on `agents(id)`, deleting child rows directly (not the agent row) won't trigger cascades. This is the intended behavior — we want surgical per-table deletes.
- **FTS5 external content:** The `fts_search` virtual table uses `content='search_content'`. After deleting from `search_content`, a `'rebuild'` command reconstructs the FTS index. This is a proven SQLite pattern.
- **Transaction size:** Worst case ~110k rows across 22 tables, estimated <2s — well within acceptable limits for a CLI-only destructive operation.
- **Custom skill overrides:** Permanently lost on reset. Documented in CLI output. `seed_bundled_skills()` on next startup restores file-level skill index.

## Acceptance Criteria Mapping

| Ticket AC | Plan Step | Verification |
|-----------|-----------|-------------|
| `--dry-run` reports rows-by-table | Step 1 (`count_agent_state`) + Step 3 (dry-run path) | Manual test: `mika agent reset mika-test --dry-run` |
| `--yes` returns agent to fresh state | Step 1 (`reset_agent_state`) + Step 3 | Manual test: `mika agent reset mika-test --yes` |
| Active-task guard with `--force` | Step 3 (guard logic) + Step 4 (unit test) | Unit test + manual test during dispatch |
| Unit tests | Step 4 | `cargo test -p mika-agent reset_agent` |
