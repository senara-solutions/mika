---
title: "Team runs stored as disconnected TOML files with no agent response tracking"
category: database-issues
component: crates/mika-agent/src/teams
tags: [sqlite, teams, persistence, graph, migration, toml, verbose-mode]
date_identified: 2026-03-03
date_resolved: 2026-03-03
severity: medium
affected_modes: [cli-chat, cli-teams]
---

# Team Runs Stored as Disconnected TOML Files with No Agent Response Tracking

## Problem

Team run history was persisted as individual TOML files in `{team_dir}/history/`, completely disconnected from the team's SQLite database. Individual agent responses during team execution were discarded entirely — only the final deliverable survived. There was no way to see what each team member contributed, no graph structure linking goals to task assignments to agent responses, and team errors were not persisted.

### Symptoms

- `mika teams log <name>` showed run history from TOML files, but no agent details
- `get_team_status` management tool read from TOML with no message graph
- Agent responses were ephemeral — lost on TUI exit
- Failed runs showed "Team completed with no deliverable" instead of the actual error
- No way to trace goal -> orchestrator decomposition -> agent responses -> critic feedback -> deliverable

## Root Cause

The original team implementation (ADR-004) used filesystem-based TOML history as the simplest path. The `conversations` table was agent-scoped (flat append-only log) and unsuitable for team message graphs. `run_team_agent()` explicitly did NOT save agent outputs to any database. The `ProgressCallback` was a simple `Box<dyn Fn(String)>` that only passed opaque progress strings.

## Solution

### 1. New SQLite tables (migration v11)

Added two tables to the per-team SQLite database (`{team_dir}/data/mika.db`):

```sql
CREATE TABLE team_runs (
    id TEXT PRIMARY KEY,
    team_name TEXT NOT NULL,
    goal TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',
    failure_reason TEXT,
    iteration INTEGER NOT NULL DEFAULT 0,
    max_iterations INTEGER NOT NULL DEFAULT 3,
    deliverable TEXT,
    started_at INTEGER NOT NULL,
    ended_at INTEGER
);

CREATE TABLE team_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES team_runs(id),
    parent_id INTEGER REFERENCES team_messages(id),
    agent_name TEXT,
    message_type TEXT NOT NULL,
    content TEXT NOT NULL,
    iteration INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
```

Messages form a tree via `parent_id` (not a DAG — single parent per node). Message types: `goal`, `orchestrator`, `assignment`, `agent_response`, `critic`, `deliverable`, `error`.

### 2. Typed event callbacks

Replaced `ProgressCallback = Box<dyn Fn(String)>` with `TeamEventCallback = Box<dyn Fn(TeamEvent) + Send + Sync>` where `TeamEvent` is an enum carrying structured data (agent names, responses, critic feedback, errors).

### 3. Engine persistence

The team engine now:
- Creates a `team_runs` row at start (status=running)
- Inserts `team_messages` rows at each phase with correct `parent_id` links
- Captures agent responses from `run_team_agent()` return values
- Updates run status on completion/failure
- Marks still-Running tasks as Failed after JoinSet completes (JoinError cleanup)

### 4. TUI verbose mode

- `/verbose` command toggles display of individual agent responses
- `TeamResponse::AgentMessage` variant carries agent name + content to TUI
- Normal mode shows only progress + deliverable (backward compatible)

### 5. TOML removal

- Deleted `crates/mika-agent/src/teams/history.rs`
- Updated `get_team_history` and `get_team_status` tools to read from team DB
- Updated CLI `teams status` and `teams log` to use DB queries

## Key Insight

When multiple persistence mechanisms exist for the same data (TOML files + SQLite), they inevitably diverge. The fix was consolidating on a single source of truth (SQLite) rather than keeping both in sync. Since no backward compatibility was needed, the TOML code was deleted entirely rather than maintained alongside DB writes.

## Files Changed

- `crates/mika-agent/src/db.rs` — migration v11, team DB query methods
- `crates/mika-agent/src/teams/engine.rs` — DB persistence, typed callbacks, JoinError fix
- `crates/mika-agent/src/teams/types.rs` — `TeamEvent` enum, `TeamEventCallback` type
- `crates/mika-agent/src/teams/mod.rs` — updated `run_team()` signature
- `crates/mika-agent/src/teams/history.rs` — DELETED
- `crates/mika-agent/src/tools/get_team_history.rs` — reads from team DB
- `crates/mika-agent/src/tools/get_team_status.rs` — reads from team DB
- `crates/mika-cli/src/tui/app.rs` — verbose mode, `TeamResponse::AgentMessage`
- `crates/mika-cli/src/commands/chat.rs` — team worker maps `TeamEvent` to `TeamResponse`, RunStatus check
- `docs/slash-commands.md` — added `/verbose` command documentation
