---
title: Agent & Team Management Tools
type: feat
status: active
date: 2026-03-02
---

# Agent & Team Management Tools

## Context

The conversation agent cannot manage agents or teams at runtime. Two tools (`list_teams`, `run_team`) exist as dead code in `team_agent_tools()` — defined and tested but never wired into any tool registry. The agent's system prompt mentions teams but provides no working tools to act on them.

This change activates the dead code, adds 4 new tools, and introduces `delegate_task` — lightweight single-agent delegation that lets Mika say "ask my researcher to look into X" without a full team workflow.

**Scope:** Read + execute only. No create/delete/clone via tools — that stays in CLI.

## Bug Fix

**Line 578 of `agent.rs`** passes `params.home_dir` (agent-specific, e.g. `~/.mika/agents/main/`) as `global_home_dir` in `PromptContext`. It should pass the global Mika home (`~/.mika/`) so `team::list_teams()` can discover teams at `{global_home}/teams/`. This works by accident in single-agent layout (paths are the same) but breaks in multi-agent layout.

## Design Decision: Struct Fields, Not ToolContext

The original draft proposed adding `global_home_dir` to `ToolContext`. This is **unnecessary**. Existing `ListTeamsTool` and `RunTeamTool` store `home_dir: PathBuf` and `settings: Settings` as struct fields on the tool itself. All new tools follow the same pattern — no ToolContext changes needed.

`AgentParams` still needs a new `global_home_dir` field to fix the PromptContext bug (Phase 1).

## Implementation

### Phase 1: Add `global_home_dir` to `AgentParams` + fix PromptContext bug

**`crates/mika-agent/src/agent.rs`**

1. Add field to `AgentParams` (after line 501):
   ```rust
   pub global_home_dir: Option<&'a Path>,
   ```

2. Fix line 578 in `run_agent_inner`:
   ```rust
   // Before (bug):
   global_home_dir: Some(params.home_dir),
   // After:
   global_home_dir: params.global_home_dir,
   ```

3. Update all `AgentParams` construction sites with `global_home_dir`:
   - `crates/mika-cli/src/commands/chat.rs` line 126: add `global_home_dir: Some(&worker_global_home)` — capture `let worker_global_home = ctx.global_home.clone();` into worker closure (around line 87)
   - `crates/mika-cli/src/commands/ask.rs` line 40: add `global_home_dir: Some(&ctx.global_home)`
   - `crates/mika-agent/src/server/handlers.rs` line 205: add `global_home_dir: Some(&s.global_home_dir)` — requires Phase 1b

**Phase 1b: Add `global_home_dir` to server `AppState`**

**`crates/mika-agent/src/server/state.rs`** — add to `AppState`:
```rust
pub global_home_dir: PathBuf,
pub settings: Settings,
```

**`crates/mika-agent/src/server/mod.rs`** — add to `AppState` construction (line 231):
```rust
global_home_dir: global_home.to_path_buf(),
settings: settings.clone(),
```

**`crates/mika-agent/src/server/mod.rs`** — update `test_state()` (line 293):
```rust
global_home_dir: PathBuf::from("/tmp/mika-test"),
settings: Settings::default(), // or test settings
```

### Phase 2: Add `timeout_secs()` to `Tool` trait

**`crates/mika-agent/src/tools/mod.rs`** — add default method to `Tool` trait (line 64):
```rust
fn timeout_secs(&self) -> Option<u64> { None }
```

**`crates/mika-agent/src/agent.rs`** — update `execute_tool` (line 904-906):
```rust
// Before:
std::time::Duration::from_secs(TOOL_TIMEOUT_SECS),
// After:
let timeout = tool.timeout_secs().unwrap_or(TOOL_TIMEOUT_SECS);
std::time::Duration::from_secs(timeout),
```
Also update the timeout error message to use the actual `timeout` value.

### Phase 3: New tool — `list_agents`

**New file: `crates/mika-agent/src/tools/list_agents.rs`**

```rust
pub struct ListAgentsTool {
    pub home_dir: PathBuf, // global home
}
```

- **Input:** none (empty object)
- **Implementation:**
  1. `mika_common::agent::list_agents(&self.home_dir)` for sorted names
  2. For each, load identity via `crate::prompt::load_identity(&agent_home)` and first line of `soul.md`
  3. Format: `- {name} ({emoji} {identity_name}): {role_hint}`
- **Output:** agent list or "No agents configured."
- **Reuse:** `mika_common::agent::list_agents()`, `agent::agent_dir()`, `prompt::load_identity()`

### Phase 4: New tools — `get_team_status` + `get_team_history`

**New file: `crates/mika-agent/src/tools/get_team_status.rs`**

```rust
pub struct GetTeamStatusTool { pub home_dir: PathBuf }
```

- **Input:** `team_name` (required), `run_id` (optional)
- **Validates:** non-empty, MAX_INPUT_LEN, `team::validate_team_name()`
- **Uses:** `teams::history::load_latest_run()` or finds by run_id in `list_runs()`
- **Output:** status, goal, iteration, timestamps, task summary table
- **Reuse:** `mika_common::team::history_dir()`, `crate::teams::history::{load_latest_run, list_runs}`

**New file: `crates/mika-agent/src/tools/get_team_history.rs`**

```rust
pub struct GetTeamHistoryTool { pub home_dir: PathBuf }
```

- **Input:** `team_name` (required), `limit` (optional, default 5)
- **Uses:** `teams::history::list_runs()`, truncated to limit
- **Output:** summary table of recent runs

### Phase 5: New tool — `delegate_task` (key feature)

**New file: `crates/mika-agent/src/tools/delegate_task.rs`**

```rust
pub struct DelegateTaskTool {
    pub home_dir: PathBuf,    // global home
    pub settings: Settings,   // needed to create ClaudeClient for delegate
}
```

- **Input:** `agent_name` (required), `task` (required)
- **Timeout:** `fn timeout_secs(&self) -> Option<u64> { Some(120) }`
- **Implementation:**
  1. Validate inputs (empty check, MAX_INPUT_LEN, `validate_agent_name()`)
  2. Check `agent_exists(&self.home_dir, agent_name)`
  3. Resolve agent home: `agent_dir(&self.home_dir, agent_name)`
  4. `crate::db::init_sqlite_vec()` then `Database::open(db_path)` + `AsyncDatabase::new(db)`
  5. Load skills: `SkillRegistry::from_dir(&agent_home.join("skills"))`
  6. Build tools: `tools::default_tools()` — **NO management_tools** (prevents recursion)
  7. Create `ClaudeClient` from `self.settings`
  8. Create optional `EmbeddingClient` from `self.settings.make_embedding_client()`
  9. Call `run_team_agent(&TeamAgentParams { ... })` with:
     - `team_context`: "You are being consulted by another agent. Provide a thorough, complete answer."
     - `mcp_manager: None` (too expensive per-delegation)
     - Session ID: new UUID
  10. **Shutdown DB** (`async_db.shutdown()`) — critical for thread cleanup
  11. Return text response via `ToolOutput::success()`
- **Reuse:** `run_team_agent()`, `AsyncDatabase`, `SkillRegistry::from_dir()`, `ClaudeClient::new()`, `tools::default_tools()`

### Phase 6: Rename `team_agent_tools` to `management_tools`

**`crates/mika-agent/src/tools/mod.rs`**

1. Add module declarations:
   ```rust
   mod delegate_task;
   mod get_team_history;
   mod get_team_status;
   mod list_agents;
   ```

2. Rename + expand `team_agent_tools()` → `management_tools()`:
   ```rust
   pub fn management_tools(home_dir: &Path, settings: &Settings) -> Vec<Box<dyn Tool>> {
       vec![
           Box::new(list_agents::ListAgentsTool { home_dir: home_dir.to_path_buf() }),
           Box::new(list_teams::ListTeamsTool { home_dir: home_dir.to_path_buf() }),
           Box::new(run_team::RunTeamTool { home_dir: home_dir.to_path_buf(), settings: settings.clone() }),
           Box::new(delegate_task::DelegateTaskTool { home_dir: home_dir.to_path_buf(), settings: settings.clone() }),
           Box::new(get_team_status::GetTeamStatusTool { home_dir: home_dir.to_path_buf() }),
           Box::new(get_team_history::GetTeamHistoryTool { home_dir: home_dir.to_path_buf() }),
       ]
   }
   ```

### Phase 7: Conditional registration at all 3 wiring sites

Register management tools only when `agents.len() > 1 || !teams.is_empty()`.

**`crates/mika-cli/src/commands/chat.rs`** (line 51):
```rust
let mut tool_registry = tools::default_tools();
let agents = mika_common::agent::list_agents(&ctx.global_home);
let teams = mika_common::team::list_teams(&ctx.global_home);
if agents.len() > 1 || !teams.is_empty() {
    for tool in tools::management_tools(&ctx.global_home, &ctx.settings) {
        tool_registry.register(tool);
    }
}
let tool_registry = Arc::new(tool_registry);
```

**`crates/mika-cli/src/commands/ask.rs`** (line 16) — same pattern with `&ctx.global_home`.

**`crates/mika-agent/src/server/mod.rs`** (line 140):
```rust
let mut tool_registry = tools::default_tools();
let agents_list = agent::list_agents(global_home);
let teams_list = mika_common::team::list_teams(global_home);
if agents_list.len() > 1 || !teams_list.is_empty() {
    for tool in tools::management_tools(global_home, settings) {
        tool_registry.register(tool);
    }
}
let tool_registry = Arc::new(tool_registry);
```

**NOT changed:** `teams/engine.rs` — team sub-agents keep `default_tools() + team_tools()` only.

### Phase 8: Update system prompt

**`crates/mika-agent/src/prompt.rs`** (lines 191-210)

Replace the existing "Teams" section with a combined "Agents & Teams" section:
```rust
if let Some(home_dir) = ctx.global_home_dir {
    let agents = agent::list_agents(home_dir);
    let teams = team::list_teams(home_dir);

    if agents.len() > 1 || !teams.is_empty() {
        prompt.push_str("\n## Agents & Teams\n");

        if agents.len() > 1 {
            prompt.push_str("You can delegate tasks to other agents using `delegate_task`. Available agents:\n");
            for name in &agents {
                let agent_home = agent::agent_dir(home_dir, name);
                let identity = load_identity(&agent_home);
                writeln!(prompt, "- {} ({} {})", name, identity.emoji, identity.name).unwrap();
            }
        }

        if !teams.is_empty() {
            prompt.push_str("You can run team workflows using `run_team`. Available teams:\n");
            for name in &teams {
                // ... same as existing code
            }
        }

        prompt.push_str("Use `list_agents` for details. Use `get_team_status`/`get_team_history` for run results.\n");
    }
}
```

Also add one line to the Tool Usage section (after line 241):
```
- You can delegate tasks to specialized agents with delegate_task when other agents are configured.
```

### Phase 9: Fix compilation everywhere

Every `AgentParams` construction needs the new `global_home_dir` field (see Phase 1 for the 3 sites).

`SilentAgentParams` and `TeamAgentParams` do **NOT** need changes — silent agents don't get management tools, and team agents already have `global_home_dir: None` in their PromptContext.

Test helpers that construct `AppState` need the new fields (Phase 1b).

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | Add `global_home_dir` to `AgentParams`, fix line 578, use `timeout_secs()` in `execute_tool` |
| `crates/mika-agent/src/tools/mod.rs` | Add `timeout_secs()` to Tool trait, add 4 module declarations, rename `team_agent_tools` → `management_tools` |
| `crates/mika-agent/src/tools/list_agents.rs` | **New file** |
| `crates/mika-agent/src/tools/delegate_task.rs` | **New file** |
| `crates/mika-agent/src/tools/get_team_status.rs` | **New file** |
| `crates/mika-agent/src/tools/get_team_history.rs` | **New file** |
| `crates/mika-agent/src/prompt.rs` | Replace Teams section with combined Agents & Teams |
| `crates/mika-agent/src/server/state.rs` | Add `global_home_dir: PathBuf` and `settings: Settings` to AppState |
| `crates/mika-agent/src/server/mod.rs` | Wire management tools, add fields to AppState construction + test_state |
| `crates/mika-agent/src/server/handlers.rs` | Add `global_home_dir` to AgentParams |
| `crates/mika-cli/src/commands/chat.rs` | Wire management tools, capture `global_home` for worker, add to AgentParams |
| `crates/mika-cli/src/commands/ask.rs` | Wire management tools, add `global_home_dir` to AgentParams |

## Existing Code to Reuse

- `mika_common::agent::{list_agents, agent_exists, validate_agent_name, agent_dir}` — `crates/mika-common/src/agent.rs`
- `mika_common::team::{list_teams, load_team, validate_team_name, history_dir}` — `crates/mika-common/src/team.rs`
- `crate::teams::history::{load_latest_run, list_runs}` — `crates/mika-agent/src/teams/history.rs`
- `crate::agent::run_team_agent()` — `crates/mika-agent/src/agent.rs:1177`
- `crate::async_db::AsyncDatabase::{open, new, shutdown}` — `crates/mika-agent/src/async_db.rs`
- `crate::skills::SkillRegistry::from_dir()` — `crates/mika-agent/src/skills/mod.rs`
- `crate::prompt::load_identity()` — `crates/mika-agent/src/prompt.rs:33`
- `crate::tools::default_tools()` — `crates/mika-agent/src/tools/mod.rs:215`
- `crate::db::init_sqlite_vec()` — `crates/mika-agent/src/db.rs`

## Verification

1. `cargo build` — compiles cleanly
2. `cargo test` — all ~686 existing tests pass + new tests
3. `cargo clippy` — no warnings
4. **Single agent, no teams:** `mika` works exactly as before — no management tools registered, no prompt changes
5. **Multi-agent:** Create second agent (`mika agents create researcher`), start `mika`. Verify:
   - Prompt shows "Agents & Teams" section
   - `list_agents` returns both agents
   - `delegate_task` to researcher works and returns a response
6. **Teams:** Create a team, verify `run_team` tool works, `get_team_status`/`get_team_history` show results
7. **Recursion prevention:** Delegated agent cannot call `delegate_task` or `run_team`
8. **DB cleanup:** No thread leaks after delegate_task (explicit `async_db.shutdown()`)
