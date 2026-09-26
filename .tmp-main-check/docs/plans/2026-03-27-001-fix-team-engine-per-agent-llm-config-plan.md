---
title: "fix: Team engine uses global LLM provider, ignores per-agent config"
type: fix
status: completed
date: 2026-03-27
issue: 285
---

# fix: Team engine uses global LLM provider, ignores per-agent config

## Overview

The team engine constructs a single shared `LlmProvider` from global `~/.mika/config.toml` and passes `settings: None` to all `TeamAgentParams`, ignoring per-agent `config.toml` files entirely. This means all agents in a team run use the same LLM provider/model regardless of their individual configurations.

## Problem Statement

**Observed in team run `fd7ef7ef` (inner-circle, trace `2b0d5e5b`):**
- All 29 LLM calls used `MiniMax-M2.7` (from global config)
- All 5 agents had `llm_provider = "anthropic"`, `anthropic_model = "claude-sonnet-4-6"` in per-agent `config.toml`
- Per-agent configs work correctly in non-team contexts (`mika ask --agent`)

## Root Cause

Two-part bug in `crates/mika-agent/src/teams/engine.rs`:

### Part 1: Single shared LLM provider from global config

`init_resources()` (line 115) constructs ONE `Arc<dyn LlmProvider>` from the global `Settings`:
```rust
let llm = settings.make_llm_provider()?;
```

The callers pass `Settings::load(global_home)` which calls `load_for_agent(global_home, global_home)` — no per-agent config layer. This single provider is stored in `EngineResources.llm` and shared across all agents.

### Part 2: `settings: None` blocks per-skill LLM overrides

Both `TeamAgentParams` construction sites pass `settings: None`:
- **Line 962** — parallel agent spawn in `execute_tasks()`
- **Line 1309** — `run_agent()` method (orchestrator/critic)

`resolve_skill_llm_override()` requires `Settings` to call `make_provider_for()`. With `None`, per-skill `[llm]` overrides silently fail.

### Why non-team paths work

CLI (`init.rs:54`) correctly loads per-agent config:
```rust
let settings = Settings::load_for_agent(&global_home, &agent_home)
```

`Settings::load_for_agent()` (`config.rs:863`) implements the cascade: defaults < global `config.toml` < per-agent `config.toml` < env vars.

## Proposed Solution

Load per-agent `Settings` and construct per-agent `LlmProvider` in `init_resources()`, storing both in `AgentResources`. Remove the shared team-level `llm` field.

### Changes

#### 1. Expand `AgentResources` struct (`engine.rs`)

Add `settings: Settings` and `llm: Arc<dyn LlmProvider>` fields:

```rust
struct AgentResources {
    home_dir: PathBuf,
    db: AsyncDatabase,
    skills: Vec<SkillEntry>,
    settings: Settings,        // NEW: per-agent cascaded config
    llm: Arc<dyn LlmProvider>, // NEW: per-agent LLM provider
}
```

#### 2. Load per-agent config in `init_resources()` loop (`engine.rs`)

Inside the per-agent loop (lines 91-113), load `Settings::load_for_agent()` and construct per-agent LLM:

```rust
// Per-agent config cascade: per-agent > global > default
let agent_settings = Settings::load_for_agent(global_home, &home_dir)?;
let agent_llm: Arc<dyn LlmProvider> = Arc::from(agent_settings.make_llm_provider()?);
```

Keep the function parameter `settings: &Settings` for shared resources only (embedding client, brave_api_key, github_token).

#### 3. Remove `llm` from `EngineResources` and `TeamEngine`

- Remove `llm: Arc<dyn LlmProvider>` from `EngineResources` struct
- Remove `llm` field from `TeamEngine` struct
- Update `TeamEngine::new()` and `new_for_resume()` destructuring

#### 4. Update `TeamAgentParams` construction — parallel spawn (`engine.rs` ~line 962)

```rust
// Before:
llm: self.llm.clone(),
settings: None,

// After:
llm: resources.llm.clone(),
settings: Some(&resources.settings),
```

#### 5. Update `TeamAgentParams` construction — `run_agent()` (`engine.rs` ~line 1309)

Same change: look up agent's `AgentResources` and use its `llm` and `settings`.

#### 6. Update resume path (`teams/mod.rs`)

`resume_team_run()` (line 77) calls `Settings::load(global_home)`. This can remain as-is because `init_resources()` now loads per-agent settings internally. The global settings parameter is only used for shared resources.

### Files to modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/teams/engine.rs` | `AgentResources` expansion, `init_resources()` per-agent loading, remove `TeamEngine.llm`, update both `TeamAgentParams` construction sites |
| `crates/mika-agent/src/teams/mod.rs` | No changes needed — `init_resources()` handles per-agent loading internally |

### What stays unchanged

- `Settings::load_for_agent()` — already correct
- `make_llm_provider()` / `make_provider_for()` — already correct
- Embedding client — stays shared (single OpenAI key)
- `brave_api_key`, `github_token` — shared container-level secrets
- Checkpoint serialization — `AgentResources` is runtime-only, not serialized

## Acceptance Criteria

- [x] Each team agent uses its own per-agent `config.toml` LLM provider/model
- [x] Agents without per-agent config fall back to global config (existing cascade)
- [x] Per-skill `[llm]` overrides work in team context (`settings` is `Some`)
- [x] Orchestrator and critic agents also use their per-agent config
- [x] Resume path loads per-agent config correctly
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## Sources

- **Institutional learning:** `docs/solutions/architecture-patterns/per-skill-llm-override-via-toml-section.md` — documents `settings: None` as known v1 limitation
- **Institutional learning:** `docs/solutions/integration-issues/agent-team-management-tools-integration.md` — per-agent vs global home_dir confusion precedent
- **Institutional learning:** `docs/solutions/runtime-errors/team-agent-max-steps-exhaustion-no-output.md` — `TeamAgentParams` under-specification precedent
- Related issue: #285
