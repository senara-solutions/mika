---
title: Agent and Team Management Tools — Feature Integration with PromptContext Bug Fix
date: 2026-03-02
category: integration-issues
tags:
  - multi-agent
  - team-management
  - tool-registry
  - prompt-context
  - delegation
  - dead-code-activation
  - code-review
modules:
  - crates/mika-agent/src/tools
  - crates/mika-agent/src/agent.rs
  - crates/mika-agent/src/prompt.rs
  - crates/mika-agent/src/teams/history.rs
  - crates/mika-agent/src/server
  - crates/mika-cli/src/commands
severity: high
symptoms:
  - System prompt referenced team/agent management but no working tools were wired
  - list_teams and run_team existed as dead code in team_agent_tools()
  - No way to delegate tasks to peer agents or inspect team run history
  - global_home_dir passed agent-specific path instead of global Mika home
  - run_team accepted arbitrary team names with no validation
  - run_team had no timeout override, inheriting 30s default for multi-minute operations
root_cause: >
  Dead code never registered in tool registries, combined with a path confusion
  bug where AgentParams.home_dir (per-agent) was forwarded as global_home_dir
  in PromptContext, causing team/agent discovery to silently fail in multi-agent
  layouts.
---

# Agent and Team Management Tools — Feature Integration

## Problem

The conversation agent's system prompt referenced team and agent management
capabilities, but no working tools were available at runtime. Two tools
(`list_teams`, `run_team`) existed as fully implemented and tested code in
`team_agent_tools()` but were never registered in any tool registry — pure dead
code. The agent could not delegate tasks to peer agents, inspect team run
history, or list available agents.

Additionally, a latent bug in `agent.rs` line 578 passed `params.home_dir`
(the per-agent directory, e.g. `~/.mika/agents/main/`) as `global_home_dir`
in `PromptContext`. The correct global Mika home (`~/.mika/`) is required for
`team::list_teams()` and `agent::list_agents()` to discover resources. This
bug was masked in the single-agent case because both paths resolve identically.

## Investigation

1. **Identified dead code**: `team_agent_tools()` in `tools/mod.rs` defined
   `ListTeamsTool` and `RunTeamTool` but was never called. All three wiring
   sites (`chat.rs`, `ask.rs`, `server/mod.rs`) called only `default_tools()`.

2. **Traced the path confusion**: `AgentParams` carried only `home_dir`
   (per-agent). `PromptContext.global_home_dir` was set to `Some(params.home_dir)`
   — wrong in multi-agent layouts where `~/.mika/agents/main/` differs from
   `~/.mika/`.

3. **Confirmed existing patterns**: `ListTeamsTool` and `RunTeamTool` stored
   dependencies (`home_dir`, `settings`) as struct fields, not via `ToolContext`.
   All new tools followed the same pattern — no `ToolContext` changes needed.

4. **Identified timeout gap**: `run_team` orchestrates a full multi-agent loop
   (potentially minutes) but inherited the 30-second default tool timeout.

5. **Identified recursion risk**: A delegated agent calling `delegate_task` could
   spawn unbounded agent chains without structural prevention.

## Root Cause

Three interrelated causes:

1. **Dead code never wired**: `team_agent_tools()` was never called at any
   tool registry construction site.

2. **Path confusion in PromptContext**: `global_home_dir: Some(params.home_dir)`
   passed the agent-specific path rather than the global Mika home, causing
   `list_teams()` and `list_agents()` to scan the wrong directory.

3. **No per-tool timeout mechanism**: The `execute_tool` loop hard-coded
   `TOOL_TIMEOUT_SECS` (30s) for all tools.

## Solution

### 1. Add `global_home_dir` to `AgentParams`

```rust
pub struct AgentParams<'a> {
    // ... existing fields ...
    pub global_home_dir: Option<&'a Path>,
}
```

Fix `run_agent_inner()`:

```rust
// Before (bug):
global_home_dir: Some(params.home_dir),
// After:
global_home_dir: params.global_home_dir,
```

Wire at all three construction sites (`chat.rs`, `ask.rs`, `handlers.rs`).
Add `global_home_dir: PathBuf` and `settings: Settings` to server `AppState`.

### 2. Per-tool timeout via `Tool` trait default method

```rust
fn timeout_secs(&self) -> Option<u64> {
    None // uses TOOL_TIMEOUT_SECS (30s) by default
}
```

Update `execute_tool()` to use it:

```rust
let timeout = tool.timeout_secs().unwrap_or(TOOL_TIMEOUT_SECS);
```

Override in `RunTeamTool` (300s) and `DelegateTaskTool` (120s).

### 3. Four new tools

- **`list_agents`**: Calls `agent::list_agents()`, loads identity and soul hint
  for each. No parameters.
- **`delegate_task`**: Runs a peer agent via `run_team_agent()` with
  `default_tools()` only (recursion prevention). Opens its own `AsyncDatabase`,
  calls `async_db.shutdown()` after completion (thread leak prevention).
  No MCP connections (`mcp_manager: None`).
- **`get_team_status`**: Loads latest or specific run by ID. UTF-8 safe
  deliverable truncation via `is_char_boundary()` walk-back.
- **`get_team_history`**: Uses `list_runs_limited()` to avoid unbounded file
  reads. Default limit 5, max 20.

### 4. `management_tools_if_needed()` DRY wrapper

```rust
pub fn management_tools_if_needed(home_dir: &Path, settings: &Settings) -> Vec<Box<dyn Tool>> {
    let agents = mika_common::agent::list_agents(home_dir);
    let teams = mika_common::team::list_teams(home_dir);
    if agents.len() > 1 || !teams.is_empty() {
        management_tools(home_dir, settings)
    } else {
        Vec::new()
    }
}
```

All three wiring sites call this single function. Single-agent installs with
no teams see zero changes.

### 5. System prompt "Agents & Teams" section

Replaces the old "Teams" section. Lists available agents with identities and
teams with agent counts. Includes self-delegation guard:

```
You are {emoji} {name}. Do not delegate tasks to yourself.
```

## P1 Review Fixes

Three critical bugs found during code review and fixed immediately:

1. **UTF-8 truncation panic** (`get_team_status.rs`): `&deliverable[..500]`
   panics on multi-byte characters. Fixed with `is_char_boundary()` walk-back.

2. **Missing `validate_team_name()`** (`run_team.rs`): Team name reached
   filesystem operations without validation, allowing path traversal.

3. **Missing timeout override** (`run_team.rs`): 30s default for a tool that
   orchestrates multi-minute team workflows. Fixed with `Some(300)`.

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Struct fields, not ToolContext | Existing tools used struct fields for `home_dir`/`settings`. Avoids broadening a widely-shared struct. |
| `Option<&'a Path>` for global_home_dir | Borrowed reference avoids clone. `None` for team sub-agents and silent agents. |
| Recursion prevention via tool set | Structural enforcement (delegated agents get `default_tools()` only), not runtime checks. |
| No MCP for delegated agents | Per-delegation MCP connections too expensive. `mcp_manager: None`. |
| `async_db.shutdown()` after delegation | Prevents OS thread leak from `AsyncDatabase`'s dedicated thread. |
| `list_runs_limited()` | Stops reading files after limit reached. `list_runs()` delegates to `list_runs_limited(_, usize::MAX)`. |

## Prevention Strategies

### Tool validation checklist

Every tool must validate string inputs in this sequence:

1. `.as_str().unwrap_or("")` — extract with safe default
2. `is_empty()` check — return `ToolOutput::error`
3. `.len() > MAX_INPUT_LEN` check — including optional parameters
4. `validate_agent_name()` or `validate_team_name()` — for filesystem-bound names
5. `agent_exists()` or `team_exists()` — only after naming rules pass

### Timeout rule

If a tool directly or indirectly calls `run_agent`, `run_team_agent`, or makes
outbound Claude API calls, it **must** override `timeout_secs()`. The override
value should match or exceed the inner timeout budget.

### String truncation rule

Never use `&s[..N]` directly. Always use `is_char_boundary()` walk-back or
`truncate_summary()` for preview/summary truncation.

### DRY registration rule

The condition `agents.len() > 1 || !teams.is_empty()` lives in
`management_tools_if_needed()`. Future wiring sites must call this function,
never reproduce the condition inline.

## Test Coverage

- 703 tests pass (up from 686), 0 clippy warnings
- All new tools have input validation tests (empty, too long, invalid name)
- `get_team_history` tests limit enforcement
- `get_team_status` tests latest run lookup
- `teams/history` tests roundtrip save/load and `list_runs_limited`
- Test helpers consolidated in `test_utils::test_helpers` (`dummy_settings`,
  `test_team_run`, `TestHarness`)

### Known test gaps

- No successful `delegate_task` test (requires running Claude API)
- `run_id` lookup path in `get_team_status` untested
- `timeout_secs()` override values not asserted
- No UTF-8 multibyte content test for deliverable truncation
- `management_tools_if_needed` condition boundary untested

## Update: Team CRUD Tools (2026-03-09)

Three new tools expanded the management suite from 7 to 10 tools:

### `create_team` (always-on)

Creates a team definition at runtime. Registered alongside `create_agent` and
`list_agents` in the always-on tier — agents can bootstrap teams even from a
single-agent setup.

Validation: name normalization (trim + lowercase), `validate_team_name()`
(1-32 chars, lowercase + digits + hyphens), duplicate check via
`team_exists()`, minimum 2 agents, all agents must exist, orchestrator must
be in agents list, `max_iterations` in 1-10 range (defaults to 3).

Creates `{home_dir}/teams/{name}/team.toml` and workspace directory.

### `delete_team` (conditional)

Deletes team directory and all data via `remove_dir_all()`. Irreversible.
Minimal validation: name normalization and existence check.

### `update_team` (conditional)

Partial updates — only provided fields are changed. At least one of
orchestrator/agents/max_iterations must be present. Agents updated before
orchestrator validation to ensure consistency. Reports changes made.

### Serde Defaults Pattern

`TeamDefinition` and its sub-structs use `#[serde(default)]` for flexible
parsing. Minimal TOML (just name, orchestrator, and agent names) parses
successfully — missing `role`, `mandate`, `flow` fields get default values.
The `create_team` tool validates these at creation time (non-empty required),
but `update_team` and file-based definitions allow empty strings.

### Registration Update

`management_tools_if_needed()` now has 3 always-on tools:

```rust
let mut tools: Vec<Box<dyn Tool>> = vec![
    Box::new(CreateAgentTool { ... }),
    Box::new(CreateTeamTool { ... }),   // NEW: always-on
    Box::new(ListAgentsTool { ... }),
];
```

`delete_team` and `update_team` are conditional (same gate as other team tools).

## Related Documentation

- [ADR-004: Multi-Agent Teams Orchestration](../../adr/004-multi-agent-teams-orchestration.md)
- [Architecture: Multi-Agent Support](../../architecture.md#13-multi-agent-support)
- [Architecture: Tools](../../architecture.md#6-tools)
- [Implementation Plan](../../plans/2026-03-02-feat-agent-team-management-tools-plan.md)
- [Investigation Panel](../architecture/investigation-panel-sse-agent-loop.md)
