---
title: Per-agent config.toml override via WellKnownAgent spec
date: 2026-04-22
last_updated: 2026-06-30
category: architecture-patterns
module: startup, config
problem_type: best_practice
component: agent_provisioning
severity: low
applies_when:
  - Adding a well-known agent that needs a different LLM model
  - Swapping an existing well-known agent's base model (post-calibration)
  - Controlling per-agent cost by assigning cheaper models to simple tasks
tags:
  - agent-provisioning
  - config-override
  - llm-model
  - well-known-agents
  - cost-optimization
  - model-swap
---

# Per-agent config.toml override via WellKnownAgent spec

## Context

Well-known agents previously shared the same default `config.toml` (Anthropic, default model, 4096 max tokens). When mika-relay was added (#721) — a single-purpose agent handling only permission classification — it needed haiku instead of the default mid-tier model. The existing per-skill LLM override (`skill_overrides` table) was considered but rejected: it relies on keyword matching to activate, which is fragile for a single-skill agent where the activation keyword (`[claude-pilot]`) appears in every input.

## Guidance

The `WellKnownAgent` struct has an optional `config_toml: Option<&'static str>` field. When `Some`, `provision_well_known_agents()` overwrites the default `config.toml` after `bootstrap_agent()` creates it. The config string must be valid TOML compatible with the `Settings` deserializer.

### When to use config_toml

Use it when the agent's **entire** config profile differs from the default — different model, different max_tokens, different log level. This is a filesystem-level override written once at agent creation, not a runtime override.

### When to use per-skill LLM override instead

Use `skill_overrides.llm_provider/llm_model` when only specific skills should use a different model while the agent's default model serves other skills normally.

### Key behavior: reconcile updates existing agents (updated 2026-06-30)

`config_toml` is written on first creation, **and** `reconcile_well_known_config()` (added with the mika-dev swap, #1633) re-writes a deployed agent's `config.toml` to match the spec on every provision pass when the spec defines a config. This is what makes a base-model swap take effect on an already-provisioned agent after `make deploy` — no delete-and-re-provision needed. (The earlier "existing agents are never overwritten" guidance predated reconcile and is no longer true.) Operators can still set `MIKA_DISABLE_AGENT_PROVISIONING=1` to freeze a hand-edited runtime config across deploys.

### Swapping the base model of an existing agent (#1633 mika-dev, #1670 mika-qa)

A model swap on a *well-known* agent is gated by the calibration framework (#1190): never change `config_toml`'s provider/model without a passing `make calibrate-<role>` run, and commit the artifacts under `docs/eval/calibration/<role>-<ticket>/<role>-<model>-post-<ticket>.{json,md}` (mirror the directory + filename convention, not a looser ad-hoc name). Two non-obvious traps:

1. **Match the provider the calibration run exercised — not the precedent's provider.** `MIKA_DEV_CONFIG` still uses `llm_provider = "openrouter"` + `openrouter_model = "z-ai/glm-5.2"` (it predates the native Z.AI provider, #1657, and its source has drifted from its runtime). When mika-qa swapped to the same model, copying mika-dev's openrouter shape would route through a *different provider* than the one the mika-qa calibration validated. Use the native form the artifact records: `llm_provider = "zai"` + `zai_model = "glm-5.2"`.

2. **Re-anchor the "None-config exemplar" reconcile test.** `test_reconcile_config_skips_agents_without_spec_config` proves reconcile leaves a `config_toml: None` agent's hand-written config untouched. It picks one concrete well-known agent as the exemplar. Each time that exemplar agent *gains* a spec config, the test's premise breaks (reconcile now overwrites it) and the test must be re-pointed at another agent still carrying `config_toml: None`. Order so far: the test used mika-qa until #1670 moved it to `mika-test`. The remaining `None`-config well-known agents are the only valid exemplars — verify the candidate is still `None` before re-anchoring.

## Examples

```rust
pub static MIKA_RELAY: WellKnownAgent = WellKnownAgent {
    name: "mika-relay",
    // ...
    config_toml: Some(MIKA_RELAY_CONFIG),
};

const MIKA_RELAY_CONFIG: &str = r#"
llm_provider = "anthropic"
anthropic_model = "claude-haiku-4-5-20251001"
llm_max_tokens = 1024
log_level = "info"
"#;
```

A test validates that the config string parses as valid TOML:
```rust
#[test]
fn test_relay_config_toml_is_valid_toml() {
    let config: toml::Table = toml::from_str(MIKA_RELAY_CONFIG).unwrap();
    assert_eq!(config.get("anthropic_model").and_then(|v| v.as_str()),
               Some("claude-haiku-4-5-20251001"));
}
```

## Related

- Issue #721 — mika-relay agent (first user of config_toml)
- Issue #1633 — mika-dev base-model swap to glm-5.2; added `reconcile_well_known_config()`
- Issue #1670 — mika-qa base-model swap to native zai/glm-5.2; re-anchored the reconcile-skip test on mika-test
- Issue #1190 — model calibration framework (swap gate)
- Issue #1657 — native Z.AI provider (`zai`/`zai_model`)
- `docs/solutions/architecture-patterns/well-known-agent-provisioning-dev-mode.md` — parent pattern
- `docs/solutions/best-practices/model-calibration-framework-structural-assertions-2026-05-18.md` — calibration suite design
