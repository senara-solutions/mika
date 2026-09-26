---
title: Team engine ignores per-agent LLM provider config
category: logic-errors
date: 2026-03-27
tags: [team-engine, config, llm-provider, per-agent, settings]
issue: 285
---

# Team engine ignores per-agent LLM provider config

## Problem

All agents in a team run used the global `~/.mika/config.toml` LLM provider/model settings, ignoring per-agent `config.toml` files at `~/.mika/agents/<name>/config.toml`. Observed in team run `fd7ef7ef` where all 29 LLM calls used `MiniMax-M2.7` (global config) despite all 5 agents having `llm_provider = "anthropic"` in their per-agent configs.

Per-agent configs worked correctly in non-team contexts (`mika ask --agent`).

## Root Cause

Two-part bug in `crates/mika-agent/src/teams/engine.rs`:

1. **Single shared LLM provider from global config**: `init_resources()` constructed ONE `Arc<dyn LlmProvider>` via `settings.make_llm_provider()` from global `Settings::load(global_home)` — no per-agent config layer. This single provider was stored in `EngineResources.llm` and shared across all agents.

2. **`settings: None` on TeamAgentParams**: Both `TeamAgentParams` construction sites (parallel spawn in `execute_tasks()` and synchronous `run_agent()`) passed `settings: None`, which blocked per-skill `[llm]` overrides via `resolve_skill_llm_override()`.

The non-team CLI path correctly used `Settings::load_for_agent(&global_home, &agent_home)` which implements the cascade: per-agent config.toml > global config.toml > env vars.

## Solution

Moved LLM provider construction from the engine level to the per-agent level:

1. **Added `settings: Settings` and `llm: Arc<dyn LlmProvider>` to `AgentResources`** — each agent now holds its own cascaded config and provider.

2. **Load per-agent config in `init_resources()` loop**: Inside the existing per-agent iteration, call `Settings::load_for_agent(global_home, &home_dir)` and `agent_settings.make_llm_provider()`.

3. **Removed shared `llm` from `EngineResources` and `TeamEngine`** — no more single shared provider.

4. **Updated both `TeamAgentParams` construction sites** to use `resources.llm.as_ref()` and `settings: Some(&resources.settings)` instead of `self.llm.as_ref()` and `settings: None`.

The global `settings` parameter on `init_resources()` is retained only for shared resources (embedding client construction).

```rust
// In init_resources() loop — per agent:
let agent_settings = Settings::load_for_agent(global_home, &home_dir)?;
let agent_llm = agent_settings.make_llm_provider()?;

agents.insert(ta.name.clone(), AgentResources {
    db: async_db,
    skills,
    home_dir,
    embedding_client: embedding_client.clone(),
    settings: agent_settings,
    llm: agent_llm,
});
```

## Prevention

- **Config resolution should follow the same path in all agent execution contexts.** The CLI, silent mode, and team engine should all use `Settings::load_for_agent()` when constructing per-agent resources. A shared LLM provider at the engine level is a design smell in a multi-agent system.

- **When `Option<&Settings>` is used as a "v1 limitation" placeholder**, track it as tech debt. The `settings: None` pattern in `TeamAgentParams` was documented in `docs/solutions/architecture-patterns/per-skill-llm-override-via-toml-section.md` but never prioritized until a user-facing bug surfaced.

- **Test with heterogeneous agent configs.** If team agents can have different configs, tests should exercise that path — not just the homogeneous case.

## Related

- `docs/solutions/architecture-patterns/per-skill-llm-override-via-toml-section.md` — documents `settings: None` as known v1 limitation
- `docs/solutions/integration-issues/agent-team-management-tools-integration.md` — prior per-agent vs global home_dir confusion
- `docs/solutions/architecture-patterns/simplified-config-4-source-model.md` — config cascade documentation
