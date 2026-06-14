---
title: "fix: callback turns use global LLM provider instead of agent config"
type: fix
status: completed
date: 2026-03-30
issue: "#323"
---

# fix: callback turns use global LLM provider instead of agent config

## Overview

Callback sessions (and all silent agent runs — heartbeat, reflection, skill_run) in server mode use the global `config.toml` LLM provider instead of the per-agent `config.toml` override. An agent configured with `deepseek/deepseek-chat` gets `minimax/MiniMax-M2.7` (the global default) during callback turns.

This is the same bug class as the team engine fix (#285, see `docs/solutions/logic-errors/team-engine-ignores-per-agent-llm-config.md`), now manifesting in the server dispatcher and HTTP handler paths.

## Problem Statement

### Root Cause

Two compounding issues:

1. **Server loads global-only Settings:** `mika-spirit.rs` calls `Settings::load(&home_dir)` which internally calls `Settings::load_for_agent(home_dir, home_dir)` — since `global_home == agent_home`, the per-agent config layer is never applied. The single `LlmProvider` built from this is shared across ALL agents.

2. **TaskDispatcher passes `settings: None`:** All four `SilentAgentParams` construction sites in `dispatcher.rs` set `settings: None`, blocking `resolve_skill_llm_override()` for per-skill `[llm]` overrides in silent mode.

### Affected Paths (Server Mode)

| Path | `llm` source | `settings` | Status |
|------|-------------|------------|--------|
| `handle_message` (Telegram inbound) | Global `AppState.llm` | Global `AppState.settings` | **Broken** |
| `run_a2a_agent` (A2A inbound) | Global `AppState.llm` | Global `AppState.settings` | **Broken** |
| `dispatch_resume_agent` (callback) | Global `TaskDispatcher.llm` | `None` | **Broken** |
| `dispatch_heartbeat` | Global `TaskDispatcher.llm` | `None` | **Broken** |
| `dispatch_reflection` | Global `TaskDispatcher.llm` | `None` | **Broken** |
| `dispatch_skill_by_name` | Global `TaskDispatcher.llm` | `None` | **Broken** |
| `dispatch_invoke_orchestrator` (team) | Per-agent (team engine fix) | Per-agent | OK |

### Affected Paths (CLI Mode)

| Path | `llm` source | `settings` | Status |
|------|-------------|------------|--------|
| CLI `chat`/`ask` (foreground) | Per-agent via `AppContext` | `Some(&settings)` | OK |
| CLI `TaskDispatcher` (heartbeat/reflection/callback) | Per-agent via `AppContext` | `None` | **Partial** — provider correct, per-skill override broken |

## Proposed Solution

Follow the team engine fix pattern (`AgentResources` with per-agent `Settings` + `LlmProvider`):

### Phase 1: Per-agent state on `AgentState`

Add `settings: Settings` and `llm: Arc<dyn LlmProvider>` fields to `AgentState` (`server/state.rs`).

**`crates/mika-agent/src/server/state.rs`:**
```rust
pub struct AgentState {
    pub db: AsyncDatabase,
    pub skills: Vec<SkillEntry>,
    pub dispatcher: Arc<TaskDispatcher>,
    pub settings: Settings,                    // NEW
    pub llm: Arc<dyn LlmProvider>,             // NEW
}
```

### Phase 2: Refactor `init_agent()` to build per-agent LLM

**`crates/mika-agent/src/server/mod.rs`:**

Change `init_agent()` to accept `global_home: &Path` and `global_settings: &Settings` instead of `llm: &Arc<dyn LlmProvider>`. Internally:

```rust
fn init_agent(
    global_home: &Path,
    global_settings: &Settings,
    agent_home: &Path,
    // ... other params unchanged
) -> Result<AgentState> {
    let agent_settings = Settings::load_for_agent(global_home, agent_home)?;
    let agent_llm = agent_settings.make_llm_provider()?;
    // ... build TaskDispatcher with agent_llm and agent_settings
}
```

### Phase 3: Thread `settings` through `TaskDispatcher`

Add `pub settings: Settings` to `TaskDispatcher`. Update all four `SilentAgentParams` construction sites:

- `dispatch_resume_agent` (line ~333)
- `dispatch_skill_by_name` (line ~245)
- `dispatch_heartbeat` (line ~513)
- `dispatch_reflection` (line ~654)

Change `settings: None` → `settings: Some(&self.settings)` in each.

### Phase 4: Update HTTP handlers to use per-agent state

**`crates/mika-agent/src/server/handlers.rs` — `handle_message`:**
```rust
// Before: llm: s.llm.as_ref(), settings: Some(&s.settings)
// After:  llm: agent_state.llm.as_ref(), settings: Some(&agent_state.settings)
```

**`crates/mika-agent/src/server/a2a.rs` — `run_a2a_agent`:**
```rust
// Before: llm: state.llm.as_ref(), settings: Some(&state.settings)
// After:  llm: agent_state.llm.as_ref(), settings: Some(&agent_state.settings)
```

### Phase 5: Keep `AppState.settings` for global values, remove `AppState.llm`

`AppState.settings` is legitimately used for global values (`brave_api_key`, `github_token`, `gateway_url`, `internal_token`, `dashboard_token`). Keep it.

Remove `AppState.llm` to prevent accidental misuse. Any remaining non-agent-scoped LLM usage (investigation panel) should construct its own provider from global settings.

### Phase 6: Fix CLI `TaskDispatcher` settings

**`crates/mika-cli/src/commands/chat.rs`:**

Add `settings` to `TaskDispatcher` construction (line ~120). The CLI already has correct per-agent settings in `AppContext` — just thread it through.

## System-Wide Impact

- **Interaction graph:** `handle_message` → `run_agent()` → LLM call now uses per-agent provider. `TaskDispatcher.dispatch_*` → `run_silent_agent()` → LLM call now uses per-agent provider. No new callbacks or side effects introduced.
- **Error propagation:** `Settings::load_for_agent()` failure during `init_agent()` triggers existing error path — agent is skipped with warning log. No change to error handling.
- **State lifecycle risks:** None — `Settings` and `LlmProvider` are immutable after construction. No partial-failure orphan risk.
- **API surface parity:** All six server dispatch paths + two HTTP handler paths share the same fix pattern. CLI gets the `settings` threading fix.
- **Integration test scenarios:** (1) Multi-agent server with different `llm_provider` configs — verify callback uses correct provider. (2) Per-skill `[llm]` override in a callback turn. (3) Malformed per-agent config does not crash server startup.

## Acceptance Criteria

- [x] Server callback turns use the agent's `llm_provider`/`llm_model` config, not the global default
- [x] Server heartbeat/reflection/skill_run turns use per-agent LLM provider
- [x] Server `handle_message` and `run_a2a_agent` use per-agent LLM provider
- [x] `SilentAgentParams.settings` is `Some(...)` in all server and CLI dispatch paths
- [x] Per-skill `[llm]` overrides work in silent/callback mode
- [x] `AppState.llm` removed (or scoped to non-agent use only)
- [x] `AppState.settings` retained for global config values
- [x] Malformed per-agent config.toml skips agent with warning, does not crash server
- [x] CLI `TaskDispatcher` passes settings through
- [x] Existing tests pass (`cargo test`)
- [x] `cargo clippy` clean

## Dependencies & Risks

- **Low risk:** `Settings` already derives `Clone` (line 460 of `config.rs`), so storing owned copies in `AgentState` and `TaskDispatcher` is trivial.
- **Precedent:** Team engine fix (#285) successfully applied this exact pattern. The same solution structure applies here.
- **Breaking changes:** `init_agent()` signature changes (internal API, not public). `AppState.llm` removal requires updating all call sites. Both are contained within `mika-agent` crate.

## Key Files

| File | Change |
|------|--------|
| `crates/mika-agent/src/server/state.rs` | Add `settings`, `llm` to `AgentState` |
| `crates/mika-agent/src/server/mod.rs` | Refactor `init_agent()`, remove global `llm` construction |
| `crates/mika-agent/src/server/handlers.rs` | Use `agent_state.llm`/`agent_state.settings` |
| `crates/mika-agent/src/server/a2a.rs` | Use `agent_state.llm`/`agent_state.settings` |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | Add `settings: Settings`, pass `Some(&self.settings)` |
| `crates/mika-cli/src/commands/chat.rs` | Pass `settings` to `TaskDispatcher` |
| `crates/mika-common/src/config.rs` | No changes (already has `load_for_agent`) |

## Sources

- **Precedent fix:** `docs/solutions/logic-errors/team-engine-ignores-per-agent-llm-config.md` — exact same bug pattern in team engine
- **Per-skill LLM doc:** `docs/solutions/architecture-patterns/per-skill-llm-override-via-toml-section.md` — flagged `settings: None` as known tech debt
- **Config cascade:** `docs/solutions/architecture-patterns/simplified-config-4-source-model.md`
- Related issue: #323
