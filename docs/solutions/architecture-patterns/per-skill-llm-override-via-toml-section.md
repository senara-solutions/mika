---
title: "Per-skill LLM override via [llm] section in skill.toml"
category: architecture-patterns
date: 2026-03-23
tags: [skills, llm, provider, manifest, agent-loop, variant-system]
modules: [mika-agent/skills, mika-agent/agent, mika-common/config]
related_issues: ["#242", "#241", "#246", "#239"]
---

# Per-skill LLM override via [llm] section in skill.toml

## Problem

Every skill used the agent's global active provider, regardless of whether the skill would perform better with a different provider/model. A `web-search` skill optimized for `gpt-4o-mini` was forced to use `claude-sonnet-4-6` if that was the agent's active config. No way to express per-skill LLM preferences declaratively.

## Root Cause

The agent loop constructed a single `LlmProvider` from `Settings::make_llm_provider()` and passed it to all agent paths. Skills could customize prompts and timeouts via variant directories, but had no way to influence which provider/model was used for the LLM call itself.

## Solution

Added an optional `[llm]` section to `skill.toml` with `provider` and `model` fields. When a matched skill declares `[llm]`, `resolve_skill_llm_override()` constructs a per-skill `LlmProvider` instance via `Settings::make_provider_for()` and uses it for the entire `run_loop()` turn.

### Key design decisions

1. **Turn-level scope**: The override applies to the entire `run_loop()` invocation (up to 10 tool steps). `run_loop()` takes a single `&dyn LlmProvider` — per-tool-call switching would require a major refactor.

2. **Conflict resolution**: When multiple matched skills have different `[llm]` overrides, the agent falls back to the default provider with a `warn!`. Same overrides are deduplicated. This is the safe default — ambiguous intent should not silently pick a winner.

3. **Same-provider short-circuit**: If the resolved override matches the active provider and model, no new instance is constructed.

4. **`manifest.llm` not duplicated on `SkillEntry`**: Initially added a separate `llm` field on `SkillEntry`, but review found this duplicated `manifest.llm` unnecessarily. Removed in favor of `entry.manifest.llm` — follows the pattern that all manifest data is accessed via `entry.manifest.*`.

5. **`settings: Option<&Settings>`**: Threading `Settings` through all three param structs (`AgentParams`, `SilentAgentParams`, `TeamAgentParams`). Made `Option` because `TaskDispatcher` and `TeamEngine` don't currently hold Settings. In v1, these paths pass `None` (override silently skipped with warn log).

### Key files

- `crates/mika-agent/src/skills/manifest.rs` — `LlmOverride` struct, `SkillManifest.llm` field
- `crates/mika-agent/src/agent.rs` — `resolve_skill_llm_override()`, integration in all 3 agent loops
- `crates/mika-common/src/config.rs` — `Settings::make_provider_for()` helper
- `crates/mika-agent/src/skills/index.rs` — Validation, `warn_missing_llm_api_keys()`

### Integration with variant system

The resolved provider/model from `[llm]` feeds into the existing `inject_skills_and_resolve_tools(provider, model)`, which calls `resolve_prompt(provider, model)` and `effective_timeout(provider, model)`. So `[llm].provider = "openai"` automatically picks up `openai/system_prompt.md` variants if they exist.

## Prevention / Best Practices

- **New optional `SkillManifest` fields must use `#[serde(default)]`** to preserve backward compatibility with existing skill.toml files.
- **Avoid duplicating manifest data on `SkillEntry`** — access via `entry.manifest.*` instead.
- **When adding `Option<&T>` params for new features**, decide upfront whether all code paths will supply it or document the limitation for paths that pass `None`.
- **Validation should cover both startup (warn) and `mika skills validate` (diagnostic)** for config issues.
