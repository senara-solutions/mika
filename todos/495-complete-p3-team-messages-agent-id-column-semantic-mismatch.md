---
status: complete
priority: p3
issue_id: "495"
tags: [code-review, database, quality]
dependencies: []
---

# team_messages.agent_id Column Stores Agent Name, Not Agent ID

## Problem Statement

The schema defines `team_messages.agent_id TEXT REFERENCES agents(id)` (a foreign key to the
`agents` table's `id` column). However, `insert_team_message` in both `db.rs` and `async_db.rs`
accepts `agent_name: Option<&str>` and stores the agent name string directly. For the default
agent where `id = "main"` and `name = "main"`, this works by coincidence. In multi-agent
scenarios where agent IDs and display names diverge, the FK constraint could be violated or
the stored value would be semantically wrong (name stored in an ID column).

## Findings

- **Source**: architecture-strategist review
- **Location**: `crates/mika-agent/src/db.rs:519–529` (schema), `db.rs:2051–2065` (insert_team_message)
- The parameter is named `agent_name` in Rust but stored in a column named `agent_id` with an FK on `agents.id`

## Proposed Solutions

### Option A: Rename column to agent_name (Recommended)
Rename the column to `team_messages.agent_name TEXT` (drop the FK constraint since it's a
display name, not an ID). Update insert and query code to match.
- **Effort**: Small (schema v1 — no migration needed, just change the schema definition)
- **Risk**: None (schema v1 is clean-slate)

### Option B: Change insert_team_message to accept agent ID
Change the parameter to `agent_id: Option<&str>` and pass the actual agent ID throughout.
Ensure callers pass the ID not the name.
- **Effort**: Small | **Risk**: Low

## Acceptance Criteria

- [ ] Column name matches the actual data it stores (name vs ID)
- [ ] FK constraint (if kept) references a column the actual stored value satisfies
- [ ] No semantic mismatch between column name and parameter name

## Work Log

- 2026-03-06: Identified by architecture-strategist review of feat/unified-task-engine
