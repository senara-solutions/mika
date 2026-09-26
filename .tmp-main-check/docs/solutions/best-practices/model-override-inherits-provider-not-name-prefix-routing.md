---
title: "A one-shot --model override is a model-id override, not a provider selector"
date: 2026-06-27
category: best-practices
module: mika-cli, llm-providers, config
problem_type: best_practice
component: cli
severity: medium
applies_when:
  - Adding or reviewing a one-shot model override (a CLI flag, an API param, a skill override) that accepts a `provider/model`-shaped string
  - A model id is parsed by splitting on `/` and the prefix is used to pick the LLM provider
  - A model routes correctly through persistent config (config.toml `llm_provider`) but fails with HTTP 401 "no API key" when passed via a one-shot `--model` flag
  - Supporting a provider whose model ids are themselves vendor-prefixed (OpenRouter: `vendor/model`, e.g. `qwen/qwen3.7-max`)
tags:
  - model-override
  - provider-routing
  - openrouter
  - cli-flag
  - llm-provider
issue: mika#1591
---

## Context

`mika ask --model <id>` and `mika chat --model <id>` (the launch flag, handled by
`AppContext::override_model` in `crates/mika-cli/src/init.rs`) parsed the model id by
splitting on `/` and, when the prefix parsed to a known `ProviderKind`, **switched the
active `llm_provider` to that native provider** and stripped the prefix. So
`--model qwen/qwen3.7-max` re-dispatched to native Qwen — which usually has no API key —
and failed with a bare HTTP 401 "no API key", even though the *same* id routes correctly
through the agent's configured provider (e.g. OpenRouter, whose model ids are themselves
`vendor/model`). The diagnostic tell: the model works via persistent `config.toml`
(`llm_provider = "openrouter"`, `openrouter_model = "qwen/qwen3.7-max"`) but fails via the
one-shot `--model` flag with the identical id.

## Guidance

A one-shot model override overrides the **model id**, not the **provider**. Inherit the
provider from the agent's configured `llm_provider`; never re-dispatch based on the model
name's prefix. Strip a `prefix/` only when it names the configured provider itself.

```rust
// crates/mika-cli/src/init.rs — pure, unit-testable, provider always inherited
fn parse_model_override(model: &str, configured: ProviderKind) -> (ProviderKind, String) {
    let resolved = crate::cli::resolve_model_alias(model);
    if let Some((prefix, rest)) = resolved.split_once('/')
        && let Ok(parsed) = prefix.parse::<ProviderKind>()
        && parsed == configured            // strip ONLY when prefix == configured provider
    {
        return (configured, rest.to_string());
    }
    (configured, resolved)                 // otherwise keep the full id, inherit provider
}
```

- `openrouter` + `qwen/qwen3.7-max` → `(openrouter, "qwen/qwen3.7-max")` — full id preserved.
- `qwen` + `qwen/qwen3.7-max` → `(qwen, "qwen3.7-max")` — matching prefix stripped.
- `anthropic` + `claude-sonnet-4-6-...` → unchanged (no prefix).

Pair it with a pre-flight that names the provider and model when the inherited provider has
no key, instead of letting a bare downstream 401 surface:

```rust
// "Provider 'openrouter' has no API key configured. Cannot route model 'qwen/qwen3.7-max'."
fn check_provider_key(provider, api_key: Option<&str>, model_id) -> Result<()> { ... }
```

Local providers (Ollama, MikaModel — localhost) are exempt from the key check.

## Why This Matters

Name-prefix provider routing conflates two orthogonal concepts: *which model* and *which
provider*. For any provider whose model ids legitimately contain a `/` (OpenRouter is the
canonical case — its own default model is `anthropic/claude-sonnet-4`), prefix routing is
not just lossy, it's *actively wrong*: it both abandons the configured provider and mangles
the id by stripping a meaningful segment. The failure is silent until a key happens to be
missing on the misinferred native provider, then surfaces as an opaque 401 far from the
parsing bug.

## When to Apply

Any time a model override accepts a `provider/model`-shaped string and a provider is
already established elsewhere (agent config, session, persistent settings). Keep
cross-provider *switching* on the dedicated, explicit surfaces that own provider selection —
in mika that's the TUI `/model` slash command and the `mika model` subcommand, which are
separate code paths and intentionally retain `provider/model` switching. A transient,
per-request override should not silently change the provider.

## Examples

Before (`override_model`): prefix parses to a provider → `settings.llm_provider = provider`
+ strip prefix → request to native provider with no key → 401.

After: provider always = configured; prefix stripped only when it equals the configured
provider; missing key on a key-requiring provider yields a named error. Unit tests assert
the openrouter/qwen/anthropic cases plus alias resolution and degenerate inputs
(`""`, `"foo/"`, `"/bar"`).
