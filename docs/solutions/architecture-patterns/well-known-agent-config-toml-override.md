---
title: Per-agent config.toml override via WellKnownAgent spec
date: 2026-04-22
category: architecture-patterns
module: startup, config
problem_type: best_practice
component: agent_provisioning
severity: low
applies_when:
  - Adding a well-known agent that needs a different LLM model
  - Controlling per-agent cost by assigning cheaper models to simple tasks
tags:
  - agent-provisioning
  - config-override
  - llm-model
  - well-known-agents
  - cost-optimization
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

### Key constraint

`config_toml` is only written on first creation (when `!agent_exists()`). Existing agents are never overwritten. To update a deployed agent's config, either delete and re-provision, or edit `~/.mika/agents/<name>/config.toml` directly.

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
- `docs/solutions/architecture-patterns/well-known-agent-provisioning-dev-mode.md` — parent pattern
