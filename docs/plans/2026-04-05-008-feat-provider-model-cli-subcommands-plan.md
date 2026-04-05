---
title: "feat: add mika provider and mika model CLI subcommands"
type: feat
status: completed
date: 2026-04-05
---

# feat: add mika provider and mika model CLI subcommands

## Overview

Extract the TUI `/provider` and `/model` slash command logic into shared functions and wire them into new top-level CLI subcommands. This gives CI, automation, and the `mika-dev` agent non-interactive provider/model management with full validation, config persistence, and model pre-fetching.

## Problem Statement

`/provider` and `/model` are TUI-only. The only CLI path for switching providers is `mika config set llm_provider <value>`, which:
- Does not pre-validate the provider (missing API key, invalid name)
- Does not update the model field atomically (leaves stale `{old_provider}_model` as active)
- Does not pre-fetch the model list for the new provider
- Does not warn about max_tokens limits

## Proposed Solution

### CLI Surface

```bash
mika provider                         # list providers with current marker
mika provider <name>                  # switch provider (validate, persist, pre-fetch)
mika provider set model <value>       # set model for current provider
mika provider set api-key             # set API key (interactive prompt, no CLI arg)
mika provider set base-url <value>    # set base_url for current provider

mika model                            # list models for current provider
mika model <name>                     # switch model (validate, persist)
```

All commands support `--agent <name>` and `--format text|json`.

### Key Design Decisions

1. **API key security:** `provider set api-key` always prompts via `dialoguer::Password`. Rejects CLI arg explicitly (shell history risk).
2. **Model pre-fetch:** Synchronous in CLI (process exits after), fire-and-forget `tokio::spawn` in TUI.
3. **OpenRouter slash format:** `parse_provider_model()` respects `model_names_contain_slash()` — `qwen/qwen-plus` on OpenRouter stays on OpenRouter.
4. **Worker model format:** Always `provider/model` via `format_worker_model()` so worker can switch atomically (#451).
5. **Display model format:** No prefix for Anthropic via `format_display_model()`.
6. **Provider list:** Shows configured models (not hardcoded defaults) via Settings.

## Acceptance Criteria

- [x] `mika provider` lists all 11 providers with current marker and configured model
- [x] `mika provider <name>` validates, persists, and pre-fetches models (synchronous)
- [x] `mika provider set model <value>` persists model for current provider in config.toml
- [x] `mika provider set api-key` prompts interactively (no CLI arg), writes to global `.env`
- [x] `mika provider set base-url <value>` persists base_url in config.toml
- [x] `mika model` lists models from cache/API with current marker and aliases
- [x] `mika model <name>` validates and persists, supports aliases and `provider/model` format
- [x] `--agent <name>` scopes reads/writes to that agent's home_dir
- [x] `--format json` produces structured JSON for all commands
- [x] TUI `/provider` and `/model` handlers refactored to call shared functions
- [x] Incorporates #452 fixes (format_worker_model, model_names_contain_slash, configured models)

## Sources & References

- TUI handlers: `crates/mika-cli/src/tui/commands/handlers.rs`
- CLI structure: `crates/mika-cli/src/cli.rs`
- Config persistence: `crates/mika-cli/src/commands/config.rs`
- Model cache: `crates/mika-common/src/llm/models.rs`
- ProviderKind: `crates/mika-common/src/llm/mod.rs`
- Related: #442, #444, #445, #451, #452
