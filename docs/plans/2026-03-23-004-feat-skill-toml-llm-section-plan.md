---
title: "feat(skills): [llm] section in skill.toml for per-skill provider and model override"
type: feat
status: completed
date: 2026-03-23
issue: "#242"
---

# feat(skills): [llm] section in skill.toml for per-skill provider and model override

## Overview

Add an optional `[llm]` section to `skill.toml` enabling skills to declare a preferred LLM provider and/or model. When a skill with `[llm]` is matched, the agent loop constructs a per-skill `LlmProvider` instance from the agent's per-provider config and uses it for that turn. This integrates with the existing variant resolution system (`resolve_prompt`, `effective_timeout`) so skills automatically get their provider-tuned prompts and timeouts.

**Prerequisites (all merged):**
- #241 — Per-provider variant directories (PR #245)
- #246 — Model-level variant granularity (PR #247)
- #239 — Per-provider agent LLM config (PR #240)

## Problem Statement / Motivation

Every skill currently uses the agent's global active provider. A `web-search` skill performing best with a fast cheap model (OpenAI `gpt-4o-mini`) is forced to use whatever the agent has configured (e.g., `claude-sonnet-4-6`). A `code-review` skill wanting the strongest reasoning model gets the same treatment. The `[llm]` section gives skill authors declarative control over which provider/model is optimal for their skill.

## Proposed Solution

### New `[llm]` section in root `skill.toml`

```toml
[skill]
name = "web-search"
description = "Search the web"

[triggers]
keywords = ["search", "look up"]

[llm]
provider = "openai"           # optional — overrides agent active provider
model    = "gpt-4o-mini"      # optional — overrides provider default model
```

Both fields are optional. Omitting `[llm]` entirely means "use agent active config" — zero changes to existing skills.

### `LlmOverride` struct in `manifest.rs`

```rust
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LlmOverride {
    pub provider: Option<String>,
    pub model: Option<String>,
}
```

Added to `SkillManifest`:
```rust
pub struct SkillManifest {
    pub skill: SkillInfo,
    #[serde(default)]
    pub triggers: Triggers,
    #[serde(default)]
    pub llm: LlmOverride,
}
```

### Resolution order

```
1. skill.toml [llm].provider + [llm].model   (explicit skill override)
2. Per-provider agent config (e.g., anthropic.model from #239)
3. Agent global llm_provider + provider default model
```

### Settings helper: `make_provider_for`

Add a new method to `Settings` that constructs a provider for any `ProviderKind` + optional model override:

```rust
impl Settings {
    pub fn make_provider_for(
        &self,
        provider: ProviderKind,
        model_override: Option<&str>,
    ) -> anyhow::Result<Arc<dyn LlmProvider>> {
        let (model_field, api_key, base_url) = self.provider_fields(provider);
        let model = model_override
            .map(String::from)
            .or_else(|| model_field.map(String::from))
            .unwrap_or_else(|| provider.default_model().to_string());
        let spec = ModelSpec { provider, model, base_url: base_url.map(String::from), api_key: api_key.map(String::from) };
        create_provider(&spec, self.llm_max_tokens)
    }
}
```

### Agent loop integration

In all three agent loop variants (conversation, silent, team), after matching skills:

```rust
let matched = params.skills.match_message(params.user_message);

// Resolve per-skill LLM override (if any matched skill declares [llm])
let (effective_llm, provider, model) = resolve_skill_llm_override(
    &matched, params.settings, llm
)?;
let provider_name = effective_llm.provider_name();
let model_name = effective_llm.model_name();

let mut skill_tool_defs =
    inject_skills_and_resolve_tools(&matched, tools, &mut system, provider_name, model_name);
```

The `resolve_skill_llm_override` function:
1. Collects all matched skills that have `[llm]` overrides
2. If none → return the default `llm` (no change)
3. If one → construct a per-skill provider via `Settings::make_provider_for`
4. If multiple with **conflicting** overrides → warn and fall back to default provider
5. If multiple with **same** override → use it (deduplicated)

## Technical Considerations

### Scope: Turn-level override

The override applies to the entire `run_loop()` invocation (all steps, up to 10). This is the only feasible approach given the architecture — `run_loop()` takes a single `&dyn LlmProvider`. Per-tool-call switching would require a major refactor that's out of scope.

### Multi-skill conflict resolution

When multiple matched skills have different `[llm]` overrides, the agent falls back to the default provider with a `warn!` log. This is the safe default — ambiguous intent should not silently pick a winner. Keyword-triggered skills and always_on skills are treated equally for conflict detection.

### `--model` CLI override precedence

`--model` is an explicit per-invocation user directive and takes precedence over `[llm]`. Resolution:
1. If `--model` is set → ignore all skill `[llm]` overrides
2. Otherwise → apply the normal skill `[llm]` resolution

### Compaction isolation

Compaction always uses the agent's default provider, never a skill override. The compaction call happens outside the skill-matched turn context.

### No provider instance caching (v1)

Provider instances are constructed fresh per turn. `create_provider()` is cheap (creates a `reqwest::Client`). Caching can be added later if profiling shows it's needed.

### Same-provider short-circuit

If the resolved `[llm]` provider and model match the agent's active provider and model, skip construction and use the existing provider instance.

## System-Wide Impact

- **Interaction graph**: `match_skills()` → `resolve_skill_llm_override()` (new) → `inject_skills_and_resolve_tools()` → `run_loop()`. No new callbacks or observers.
- **Error propagation**: `make_provider_for()` can fail (missing base_url for non-Anthropic). Failure falls back to default provider with a `warn!` — never crashes the agent.
- **State lifecycle risks**: None. Provider instances are ephemeral (per-turn). No persistent state changes.
- **API surface parity**: Three agent loop variants (conversation, silent, team) + delegate_task all need the same override logic. Extract into a shared function.
- **Integration test scenarios**: (1) Skill with `[llm].provider` triggers provider switch, variant prompt resolved correctly. (2) Two skills with conflicting `[llm]` fall back to default. (3) `--model` override suppresses skill `[llm]`.

## Acceptance Criteria

### Functional Requirements

- [ ] `SkillManifest` parses `[llm]` section with `provider` and `model` fields, both optional (`manifest.rs`)
- [ ] All existing `skill.toml` files parse without change (`#[serde(default)]`)
- [ ] `Settings::make_provider_for(provider, model_override)` constructs a provider from per-provider config (`config.rs`)
- [ ] `resolve_skill_llm_override()` extracts the effective provider from matched skills (`agent.rs`)
- [ ] Conflict detection: multiple different `[llm]` overrides → warn + fallback to default
- [ ] Same-provider short-circuit: skip provider construction when override matches active
- [ ] Resolved provider/model passed to `resolve_prompt()` and `effective_timeout()` (variant integration)
- [ ] All three agent loop variants (conversation, silent, team) apply the override
- [ ] Startup warning when `[llm].provider` is set but API key is missing
- [ ] `mika skills validate` reports `[llm]` config issues (invalid provider name, `[llm]` in variant dirs)
- [ ] `mika skills list` shows `[llm: provider/model]` badge
- [ ] `mika skills info` shows `[llm]` section details
- [ ] `docs/skills.md` manifest reference table updated with `[llm]` section

### Non-Functional Requirements

- [ ] `cargo clippy` passes clean
- [ ] `cargo test` passes — unit tests for all permutations

### Quality Gates

- [ ] Tests cover: no `[llm]` (fallback), provider only, model only, provider + model, conflict, missing API key warning

## Implementation Phases

### Phase 1: Data Model (`manifest.rs`, `index.rs`)

**Files:** `crates/mika-agent/src/skills/manifest.rs`, `crates/mika-agent/src/skills/index.rs`

1. Add `LlmOverride` struct to `manifest.rs` with `provider: Option<String>` and `model: Option<String>`
2. Add `#[serde(default)] pub llm: LlmOverride` field to `SkillManifest`
3. Add `pub llm: LlmOverride` field to `SkillEntry` in `index.rs`
4. Populate `llm` from manifest during `scan_skills_dir()` scan
5. Add unit tests for parsing `[llm]` section (all permutations)

### Phase 2: Settings Helper (`config.rs`)

**Files:** `crates/mika-common/src/config.rs`

1. Add `make_provider_for(&self, provider: ProviderKind, model_override: Option<&str>) -> Result<Arc<dyn LlmProvider>>` to `Settings`
2. Reuse `provider_fields()` for credential lookup
3. Add unit test verifying construction with model override

### Phase 3: Agent Loop Integration (`agent.rs`)

**Files:** `crates/mika-agent/src/agent.rs`

1. Add `resolve_skill_llm_override()` function:
   - Takes `&[&SkillEntry]`, `&Settings`, `&dyn LlmProvider` (default)
   - Returns `Result<Option<Arc<dyn LlmProvider>>>` — `None` means use default
   - Conflict detection with `warn!` on mismatch
   - Same-provider short-circuit
2. Integrate into conversation agent loop (`run_agent`)
3. Integrate into silent agent loop (`run_silent_agent`)
4. Integrate into team agent loop (`run_team_agent`)
5. Pass the effective provider's `provider_name()` and `model_name()` to `inject_skills_and_resolve_tools()`
6. Thread `settings` through to where the override is resolved (it's already available via `AgentParams` or passed directly)

### Phase 4: Validation (`index.rs`)

**Files:** `crates/mika-agent/src/skills/index.rs`

1. In `validate_skill()`: parse `[llm]` section and validate:
   - `provider` is a valid `ProviderKind` (case-insensitive `FromStr`)
   - `model` is a non-empty string if present
   - Warn if `[llm]` appears in variant `skill.toml` (check during variant dir scanning)
2. Startup warning (in agent init, not just validate): when a loaded skill has `[llm].provider` set but the provider's API key is not configured in `Settings`

### Phase 5: CLI Display (`list.rs`, `info.rs` / CLI skill commands)

**Files:** `crates/mika-cli/src/commands/skills.rs` (or wherever `mika skills list/info` lives)

1. `mika skills list`: show `[llm: openai/gpt-4o-mini]` badge after skill name
2. `mika skills info`: show `LLM Override` section with provider and model

### Phase 6: Documentation (`docs/skills.md`)

**Files:** `docs/skills.md`

1. Add `[llm]` section to the manifest reference table alongside `[skill]` and `[triggers]`
2. Add "Per-skill LLM Assignment" section with examples:
   - No override (fallback)
   - Provider only
   - Provider + model
   - Interaction with variant directories

## Sources & References

### Internal References

- `crates/mika-agent/src/skills/manifest.rs` — `SkillManifest`, `ProviderSkillOverride`
- `crates/mika-agent/src/skills/index.rs` — `SkillEntry`, `resolve_prompt()`, `effective_timeout()`, `validate_skill()`, `scan_skills_dir()`
- `crates/mika-agent/src/agent.rs` — Agent loop, `inject_skills_and_resolve_tools()`, `max_skill_timeout()`
- `crates/mika-common/src/config.rs` — `Settings`, `provider_fields()`, `make_llm_provider()`, `ActiveLlmConfig`
- `crates/mika-common/src/llm/mod.rs` — `ProviderKind`, `ModelSpec`, `create_provider()`, `LlmProvider` trait
- `docs/skills.md` — Current manifest documentation
- `docs/solutions/architecture-patterns/per-provider-skill-variant-directories.md`
- `docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md`

### Related Work

- #241 — Per-provider variant directories (PR #245)
- #246 — Model-level variant granularity (PR #247)
- #239 — Per-provider agent LLM config (PR #240)
