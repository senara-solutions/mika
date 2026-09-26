# Plan: Raise per-session task creation limit (#677)

## Problem

The task engine limits agent-created tasks to 5 per session (Guard 5 in `create_task.rs`). Milestone dispatch with 7+ issues hits this limit, forcing workarounds that add complexity. The guard exists to catch runaway loops, not to limit legitimate bulk creation.

## Solution

1. Raise default from 5 to 25
2. Make configurable via `max_agent_tasks_per_session` in agent config (config.toml / env var)

## Implementation Steps

### Step 1: Add config key to mika-common

- **File:** `crates/mika-common/src/config.rs`
- Add `ConfigKeyInfo` entry for `max_agent_tasks_per_session` (backend: `File`, env: `MIKA_MAX_AGENT_TASKS_PER_SESSION`)
- Add field `pub max_agent_tasks_per_session: i64` to `Settings` struct with `#[serde(default = "default_max_agent_tasks_per_session")]` defaulting to 25

### Step 2: Thread through ToolContext

- **File:** `crates/mika-agent/src/tools/mod.rs`
- Add `pub max_tasks_per_session: i64` field to `ToolContext`

### Step 3: Set field at all ToolContext construction sites

- **File:** `crates/mika-agent/src/agent.rs` — 3 construction sites (conversation, silent, team)
- Source from `params.settings.map_or(25, |s| s.max_agent_tasks_per_session)`
- **File:** `crates/mika-agent/src/test_utils.rs` — test helper uses default 25

### Step 4: Use configurable limit in create_task

- **File:** `crates/mika-agent/src/tools/create_task.rs`
- Remove `const MAX_TASKS_PER_SESSION: i64 = 5;`
- Replace usage with `ctx.max_tasks_per_session`

### Step 5: Update tests

- Update existing test in `create_task.rs` that validates the limit (line ~1161)
- Add test verifying custom limit is respected

### Step 6: Update documentation

- `docs/architecture.md` — update "Max 5 agent-created items per session" references
- `.env.example` — add `MIKA_MAX_AGENT_TASKS_PER_SESSION`

## Design Decisions

- **Field on ToolContext (not re-reading config per call):** Follows existing pattern (brave_api_key, github_token are resolved once and threaded). Avoids per-tool-call config lookups.
- **i64 type:** Matches `count_session_tasks()` return type.
- **Default 25:** Covers largest milestone (18 tickets = 19 tasks) with margin, still catches runaway loops.
- **File backend (config.toml):** Per-agent configurable without env vars. Env var override available for container deployments.
