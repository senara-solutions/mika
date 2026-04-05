---
title: "fix: provider autocomplete picker shows hardcoded defaults instead of configured models"
type: fix
status: completed
date: 2026-04-05
---

# fix: provider autocomplete picker shows hardcoded defaults instead of configured models

## Overview

The TUI `/provider <Tab>` autocomplete picker shows hardcoded `default_model()` values (e.g., `anthropic/claude-sonnet-4` for openrouter) instead of the user's configured `{provider}_model` from Settings. After selecting a provider, the actual model resolves differently — confusing the user.

This is the remaining unfixed part of #451. The first commit (c46837b) fixed worker propagation and the `/provider` listing, but the autocomplete picker was missed.

## Root Cause

`complete_provider()` in `completers.rs:75-89` ignores `CompletionContext` (parameter named `_ctx`) and uses hardcoded `p.default_model()`. The reference implementation in the `/provider` listing handler (lines 649-668) correctly loads Settings and calls `provider_fields(p).0` — the autocomplete should do the same.

The pattern already exists in the same file: `complete_model()` (lines 46-47) loads Settings via `ctx.global_home` and `ctx.home_dir`.

## Acceptance Criteria

- [x] `complete_provider()` loads Settings and shows the user's configured model per provider
- [x] Falls back to `default_model()` only when no `{provider}_model` is configured
- [x] If Settings load fails, falls back to hardcoded defaults (graceful degradation)
- [x] Existing tests pass; add test for configured model display if testable

## MVP

### `crates/mika-cli/src/tui/commands/completers.rs`

```rust
/// `/provider <tab>` — LLM provider names with configured models.
pub fn complete_provider(
    arg_text: &str,
    _arg_index: usize,
    ctx: &CompletionContext,         // was _ctx — now used
) -> (Vec<CompletionItem>, &'static str) {
    use mika_common::llm::ProviderKind;

    // Load settings to show configured models (same pattern as complete_model)
    let settings =
        mika_common::config::Settings::load_for_agent(ctx.global_home, ctx.home_dir).ok();

    let items: Vec<CompletionItem> = ProviderKind::ALL
        .iter()
        .map(|p| {
            let model = settings
                .as_ref()
                .and_then(|s| s.provider_fields(*p).0.map(String::from))
                .unwrap_or_else(|| p.default_model().to_string());
            CompletionItem {
                value: p.to_string(),
                description: Some(model),
            }
        })
        .collect();
    (filter_by_prefix(items, arg_text), " Providers ")
}
```

## Sources

- Related issue: #451
- First fix commit: c46837b
- Reference pattern: `completers.rs:46-47` (`complete_model` loads Settings)
- Reference pattern: `handlers.rs:652-664` (`/provider` listing loads Settings)
