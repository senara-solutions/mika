---
title: "Consolidate per-agent and per-team databases into single container database"
date: 2026-03-06
category: database-issues
tags: [sqlite, database-topology, schema-v1, unified-task-engine, per-container-isolation, foreign-keys, async-database]
severity: medium
affected_modules:
  - crates/mika-common/src/config.rs
  - crates/mika-common/src/home.rs
  - crates/mika-agent/src/server/mod.rs
  - crates/mika-agent/src/teams/engine.rs
  - crates/mika-agent/src/teams/mod.rs
  - crates/mika-agent/src/tools/delegate_task.rs
  - crates/mika-agent/src/tools/run_team.rs
  - crates/mika-agent/src/tools/get_team_status.rs
  - crates/mika-agent/src/tools/get_team_history.rs
  - crates/mika-cli/src/init.rs
  - crates/mika-cli/src/commands/chat.rs
root_cause: "Schema v1 added agent_id foreign keys to all tables but never consolidated the database file topology — each agent and team still opened its own mika.db file"
resolution_type: refactor
---

# Consolidate Per-Agent and Per-Team Databases into Single Container Database

## Problem Symptom

The unified task engine (schema v1) was designed around a single-database-per-container model where agents and teams are rows in shared tables, with `agent_id` foreign keys scoping every query. However, the database **file topology** was never actually consolidated:

- **Per-agent databases** at `~/.mika/agents/{name}/data/mika.db` — opened by CLI init, the HTTP server, the team engine, and the `delegate_task` tool.
- **Per-team databases** at `~/.mika/teams/{name}/data/mika.db` — opened by `open_or_create_team_db()` and CLI team mode.

Cross-agent queries ("show me all pending tasks across every agent") were impossible without `ATTACH` gymnastics. Foreign key relationships between tasks spawned by team runs and conversations owned by different agents could not be enforced. The `container_db_path()` helper in `home.rs` already returned the correct path (`~/.mika/data/mika.db`) but nothing used it.

## Root Cause

Schema v1 correctly added `agent_id` foreign keys to all tables, but the code that opens database connections was never updated to use a single file. Six different code paths each constructed their own per-agent or per-team DB path.

## Solution

### Config Path Change (`config.rs`)

The single change that cascades to all downstream `Database::open()` calls:

```rust
// BEFORE: resolves to {agent_home}/data/mika.db
settings.db_path = agent_home.join("data").join("mika.db");

// AFTER: resolves to {global_home}/data/mika.db
settings.db_path = global_home.join("data").join("mika.db");
```

### Per-Team DB Functions Eliminated (`teams/mod.rs`)

Deleted: `TeamDbError`, `open_team_db_sync()`, `open_team_db()`, `open_or_create_team_db()`. All callers now receive the shared `AsyncDatabase` from context.

### TeamEngine Connection Pattern (`teams/engine.rs`)

```rust
// BEFORE: Opens a SEPARATE file per agent
for ta in &team.agents {
    let db_path = home_dir.join("data").join("mika.db");
    let db = Database::open(&db_path)?;
    let async_db = AsyncDatabase::new(db);
}

// AFTER: Same container DB, separate connections for WAL concurrency
let db_path = home::container_db_path(global_home);
for ta in &team.agents {
    let db = Database::open(&db_path)?;
    let async_db = AsyncDatabase::new_with_agent(db, &ta.name);
}
```

### Server Init (`server/mod.rs`)

Added `global_home` parameter to `init_agent()`. Uses `home::container_db_path(global_home)` instead of `agent_home.join("data").join("mika.db")`.

### Tool Changes

- **`delegate_task`**: Uses `home::container_db_path()` + `AsyncDatabase::new_with_agent()` instead of per-agent path.
- **`run_team`**: Opens container DB instead of per-team DB.
- **`get_team_status` / `get_team_history`**: Converted to unit structs, use `ctx.db` from `ToolContext` instead of opening per-team DBs.

### Bootstrap Cleanup (`home.rs`)

- Removed `create_dir_all(home_dir.join("data"))` from per-agent `bootstrap()` — no longer creates unused per-agent `data/` directories.
- Updated `is_initialized()` and `is_legacy_layout()` to check `container_db_path()`.
- `bootstrap_fresh_install()` creates `~/.mika/data/` at the container level.

## Why It Works

**WAL mode concurrency.** SQLite WAL supports concurrent readers with a single writer. `AsyncDatabase` serializes writes through a dedicated OS thread + `sync_channel(512)` mpsc. Multiple connections to the same file are safe.

**AsyncDatabase shutdown semantics.** `shutdown()` kills the inner DB thread. Clones sharing the same sender keep the thread alive. When team agents finish and their clones drop, the parent's sender persists the thread. `delegate_task` explicitly shuts down its clone after the delegated agent completes.

**`register_agent` idempotency.** Uses `INSERT OR IGNORE`, so calling it multiple times for the same agent (at startup, during team runs, during delegation) is safe.

**Agent-scoped queries unchanged.** Every DB method already scopes with `WHERE agent_id = ?`. The `agent_id` is set on `AsyncDatabase` at construction and auto-injected into closures.

## Prevention & Best Practices

### Rules

1. **Always use `home::container_db_path()`** for DB access. Never construct DB paths from agent/team home directories.
2. **`AsyncDatabase` must be created via `new_with_agent()`** to ensure proper agent scoping.
3. **Task engine and dispatcher receive, never create, their DB handle** from the caller.

### Code Review Checklist

- [ ] No ad-hoc DB path construction — search for `.join("data").join("mika.db")`
- [ ] No new `Database::open()` calls that bypass `container_db_path()`
- [ ] Agent/team home directories used only for config, skills, and file I/O
- [ ] No SQLite pragmas that conflict with WAL shared access

### Key Invariants

1. **Single DB file** — `~/.mika/data/mika.db` for the entire container
2. **Agent scoping via `AsyncDatabase.agent_id`** — row-level isolation, not file-level
3. **WAL mode** — `PRAGMA journal_mode=WAL` + `busy_timeout=5000ms` on every connection
4. **No `.db` files in agent or team home directories** — only config, skills, workspace

## Related Documentation

- **Origin brainstorm:** [docs/brainstorms/2026-03-04-unified-task-engine-brainstorm.md](../../brainstorms/2026-03-04-unified-task-engine-brainstorm.md)
- **Implementation plan:** [docs/plans/2026-03-06-refactor-consolidate-single-database-per-container-plan.md](../../plans/2026-03-06-refactor-consolidate-single-database-per-container-plan.md)
- **Callback/resume lifecycle:** [docs/solutions/architecture/callback-resume-agent-lifecycle.md](../architecture/callback-resume-agent-lifecycle.md)
- **ADR-004:** [docs/adr/004-multi-agent-teams-orchestration.md](../../adr/004-multi-agent-teams-orchestration.md)
