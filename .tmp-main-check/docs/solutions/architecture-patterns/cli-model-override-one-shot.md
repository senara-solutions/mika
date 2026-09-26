---
title: "CLI --model flag: one-shot LLM override without config persistence"
category: architecture-patterns
date: 2026-03-17
tags: [cli, model, llm, clap, override, alias]
modules: [mika-cli]
related:
  - docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md
  - docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md
  - docs/solutions/architecture-patterns/simplified-config-4-source-model.md
---

## Problem

No way to override the LLM model per-invocation from the CLI. The model was determined solely by config (`llm_model` in config.toml or `MIKA_LLM_MODEL` env var). Use case: relay supervisor (mika-dev) answering callbacks via `mika ask` needs to specify "use Sonnet for this call" without changing persistent config.

## Root Cause

Design gap. The TUI had `/model` for mid-session switching (which persists to config.toml), but non-interactive paths (`mika ask`, `mika chat`) had no equivalent.

## Solution

Added `--model <model>` optional arg to both `AskArgs` and `ChatArgs` in clap. Applied as a one-shot override after `Settings::load()` but before `make_llm_provider()`.

### Key design decisions

1. **Override via `AppContext::override_model()`** — encapsulates the two-step invariant (mutate `settings.llm_model`, then rebuild provider). Defined in `init.rs` so both `ask.rs` and `chat.rs` use a single-line call.

2. **Shared alias table** — `MODEL_ALIASES` is a single `pub const` 3-tuple array `(shorthand, full_id, display_name)` in `cli.rs`. Both the CLI `resolve_model_alias()` function and the TUI `/model` handler import it. Avoids drift between two separate tables.

3. **`conflicts_with = "team"`** — `--model` is mutually exclusive with `--team` on both `ask` and `chat`. Team runs use the configured model for all agents; a per-invocation override for the orchestrator only would be confusing.

4. **Pass-through for unknown values** — `resolve_model_alias("openai/gpt-4o")` returns the input unchanged, supporting provider-prefixed model specs without exhaustive enumeration.

### Code pattern

```rust
// cli.rs — single source of truth for aliases
pub const MODEL_ALIASES: &[(&str, &str, &str)] = &[
    ("sonnet", "claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("opus",   "claude-opus-4-6",   "Claude Opus 4.6"),
    ("haiku",  "claude-haiku-4-5",  "Claude Haiku 4.5"),
];

pub fn resolve_model_alias(input: &str) -> String {
    let lower = input.to_lowercase();
    for &(alias, full_id, _) in MODEL_ALIASES {
        if lower == alias || lower == full_id {
            return full_id.to_string();
        }
    }
    input.to_string()
}

// init.rs — encapsulated override
impl AppContext {
    pub fn override_model(&mut self, model: &str) -> Result<()> {
        let resolved = crate::cli::resolve_model_alias(model);
        self.db_ctx.settings.llm_model = resolved;
        self.llm = self.db_ctx.settings.make_llm_provider()?;
        Ok(())
    }
}

// ask.rs / chat.rs — one-line call site
if let Some(model) = model_override {
    ctx.override_model(model)?;
}
```

## Prevention

- **When adding CLI flags that duplicate TUI slash commands**: extract shared logic (alias tables, resolution functions) to a common location from the start. The TUI `/model` handler had an inline alias table that was later duplicated for the CLI flag.
- **When adding flags to subcommands with mutual exclusions**: use `conflicts_with` in clap to enforce at parse time rather than silently ignoring incompatible combinations.
- **Override-then-reconstruct pattern**: when a config field controls object construction (like `llm_model` → `LlmProvider`), encapsulate the mutate+rebuild sequence in a method to prevent callers from forgetting the second step.
