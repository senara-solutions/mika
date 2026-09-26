---
title: "Extract TUI slash commands into shared CLI subcommands"
category: cli-features
date: 2026-04-05
tags: [cli, provider, model, extraction, clap, shared-logic]
modules: [mika-cli]
---

# Extract TUI slash commands into shared CLI subcommands

## Problem

`/provider` and `/model` were TUI-only slash commands with no CLI equivalent. The only non-interactive path (`mika config set llm_provider`) didn't validate, pre-fetch models, or update the model field atomically.

## Root Cause

The validation, persistence, and model-fetching logic was tightly coupled to the TUI `App` struct in `handlers.rs`. No shared functions existed for CLI consumption.

## Solution

Extract shared logic into `commands/provider.rs` and `commands/model.rs` with structured return types, then wire as top-level CLI subcommands.

**Key pattern:** Shared functions return data structs (e.g., `ProviderSwitchResult`, `ModelSwitchResult`). TUI handlers call these, then apply TUI-specific state updates (`app.model`, `AgentRequest::SetModel`, `app.needs_redraw`). CLI `run()` functions call the same shared functions, then format output for stdout.

**TUI/CLI divergence for model pre-fetch:** `switch_provider()` takes a `prefetch_models: bool` parameter. CLI passes `true` (synchronous fetch before exit). TUI passes `false` and spawns its own `tokio::spawn` fire-and-forget.

**Worker vs. display model format (#451):** `format_worker_model()` always includes `provider/model` prefix so the worker can switch atomically. `format_display_model()` omits the prefix for Anthropic (convention match). Both are in `commands/provider.rs` — shared by TUI and CLI.

**OpenRouter slash handling (#452):** `parse_provider_model()` checks `model_names_contain_slash()` before interpreting slashes as cross-provider switches. Without this, `qwen/qwen-plus` on OpenRouter would switch to Qwen provider.

## Key Files

- `crates/mika-cli/src/commands/provider.rs` — shared provider logic + CLI run
- `crates/mika-cli/src/commands/model.rs` — shared model logic + CLI run
- `crates/mika-cli/src/cli.rs` — `ProviderArgs`, `ModelArgs`, `ProviderSubcommand`
- `crates/mika-cli/src/tui/commands/handlers.rs` — refactored TUI handlers

## Pitfalls

- **Swapped `global_home`/`home_dir` arguments:** Both are `&Path`, so the compiler doesn't catch misorder. `Settings::load_for_agent(global_home, home_dir)` — always `global_home` first.
- **TUI provider switch duplication:** When extracting TUI logic, verify BOTH handlers call the shared function. Initial implementation left the provider handler inline while model was properly extracted.
- **Worker model must include provider prefix:** Always use `format_worker_model()` for `AgentRequest::SetModel`, never the display format. Without the prefix, the worker can't detect provider switches (#451).
- **Rebase before PR:** If upstream merges fix-related PRs (#452), rebase and rebuild — don't just resolve conflicts. The shared functions must incorporate the upstream fixes from day one.

## Prevention

- For functions with multiple `&Path` parameters of the same type, maintain consistent `(global_home, home_dir)` ordering across all functions in a module.
- When extracting TUI logic, always check that ALL handlers (not just one) call the shared functions.
- Test worker model format explicitly (`test_format_helpers`).
