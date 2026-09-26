---
title: "Server dispatch uses global LLM provider instead of per-agent config"
category: logic-errors
date: 2026-03-30
tags: [llm-provider, per-agent-config, server-mode, callback, heartbeat, silent-agent, settings]
issue: "#323"
related:
  - docs/solutions/logic-errors/team-engine-ignores-per-agent-llm-config.md
  - docs/solutions/architecture-patterns/per-skill-llm-override-via-toml-section.md
  - docs/solutions/architecture-patterns/simplified-config-4-source-model.md
---

# Server dispatch uses global LLM provider instead of per-agent config

## Problem

In server mode, all agent execution paths — message handling, A2A requests, and all silent dispatch paths (callback, heartbeat, reflection, skill_run) — used a single global `LlmProvider` constructed from `Settings::load(&home_dir)`. This ignored per-agent `config.toml` overrides entirely.

An agent configured with `llm_provider = "deepseek"` in its `config.toml` would still use the global default (e.g., `minimax/MiniMax-M2.7`) for all callback and background turns.

Additionally, all four `SilentAgentParams` construction sites in `TaskDispatcher` passed `settings: None`, which blocked per-skill `[llm]` overrides via `resolve_skill_llm_override()`.

**Symptom:** LLM call logs for callback sessions showed the wrong provider/model compared to the agent's configured values.

## Root cause

Two compounding issues:

1. **`run_server()` built one global LLM provider.** `Settings::load(&home_dir)` calls `Settings::load_for_agent(home_dir, home_dir)` — since `global_home == agent_home`, the per-agent config layer was never applied. The single `Arc<dyn LlmProvider>` was stored on `AppState` and shared across all agents.

2. **`TaskDispatcher` passed `settings: None`.** All four dispatch methods (`dispatch_resume_agent`, `dispatch_skill_by_name`, `dispatch_heartbeat`, `dispatch_reflection`) constructed `SilentAgentParams` with `settings: None`, blocking per-skill LLM overrides.

This is the same bug class as the team engine fix (#285), now manifesting in the server dispatcher and HTTP handler paths.

## Solution

Moved LLM provider ownership from shared `AppState` to per-agent `AgentState`, following the team engine fix pattern:

### 1. Added per-agent state to `AgentState`

```rust
// server/state.rs
pub struct AgentState {
    // ... existing fields ...
    pub settings: Settings,           // NEW: per-agent config
    pub llm: Arc<dyn LlmProvider>,    // NEW: per-agent provider
}
```

### 2. Refactored `init_agent()` to build per-agent LLM

```rust
async fn init_agent(
    agent_name: &str,
    agent_home: &Path,
    global_home: &Path,
    // removed: llm: &Arc<dyn LlmProvider>,
    // ...
) -> Result<AgentState> {
    let agent_settings = Settings::load_for_agent(global_home, agent_home)?;
    let agent_llm = agent_settings.make_llm_provider()?;
    // ... uses agent_llm for TaskDispatcher and AgentState
}
```

### 3. Added `settings` to `TaskDispatcher`

```rust
pub struct TaskDispatcher {
    // ... existing fields ...
    pub settings: Settings,
}
```

All four `SilentAgentParams` sites changed from `settings: None` to `settings: Some(&self.settings)`.

### 4. Updated HTTP handlers

- `handle_message`: `s.llm` → `a.llm`, `s.settings` → `a.settings`
- `run_a2a_agent`: `state.llm` → `agent_state.llm`, `state.settings` → `agent_state.settings`
- Compaction: `s.llm` → `a.llm`

### 5. Removed `AppState.llm`

The global `llm` field was removed from `AppState` to prevent accidental misuse. The investigation panel now uses the default agent's LLM provider directly.

### 6. Fixed CLI `TaskDispatcher`

Added `settings: ctx.settings.clone()` to the CLI `TaskDispatcher` construction, enabling per-skill LLM overrides in CLI background tasks.

## Affected paths

| Path | Before | After |
|------|--------|-------|
| `handle_message` | Global `AppState.llm` | Per-agent `AgentState.llm` |
| `run_a2a_agent` | Global `AppState.llm` | Per-agent `AgentState.llm` |
| `dispatch_resume_agent` (callback) | Global LLM, `settings: None` | Per-agent LLM, `settings: Some(...)` |
| `dispatch_heartbeat` | Global LLM, `settings: None` | Per-agent LLM, `settings: Some(...)` |
| `dispatch_reflection` | Global LLM, `settings: None` | Per-agent LLM, `settings: Some(...)` |
| `dispatch_skill_by_name` | Global LLM, `settings: None` | Per-agent LLM, `settings: Some(...)` |
| CLI `TaskDispatcher` | Correct LLM, `settings: None` | Correct LLM, `settings: Some(...)` |

## Prevention

1. **Config resolution rule:** All agent execution contexts (CLI, server, silent, team, callback) must use `Settings::load_for_agent(global_home, agent_home)` — never `Settings::load(home_dir)` alone.

2. **No shared `LlmProvider` across agents.** Each agent constructs its own provider from its own settings. Shared state (`AppState`) should only hold non-agent-scoped values.

3. **`settings: None` is tech debt.** When adding `Option<&T>` params for new features, decide upfront whether all code paths will supply it. If not, document the limitation.

4. **Grep for the pattern:** When fixing a provider resolution bug in one execution context, search for the same pattern in all other contexts (CLI, server, team engine, silent dispatch).

## Key files

- `crates/mika-agent/src/server/state.rs` — `AgentState` struct (added `settings`, `llm`; removed `llm` from `AppState`)
- `crates/mika-agent/src/server/mod.rs` — `init_agent()` (per-agent settings loading), `run_server()` (removed global LLM)
- `crates/mika-agent/src/server/handlers.rs` — `handle_message` (switched to per-agent)
- `crates/mika-agent/src/server/a2a.rs` — `run_a2a_agent` (switched to per-agent)
- `crates/mika-agent/src/task_engine/dispatcher.rs` — All four `SilentAgentParams` sites
- `crates/mika-cli/src/commands/chat.rs` — CLI `TaskDispatcher` settings threading
