---
title: "TUI provider switch worker propagation"
severity: high
issues: [451]
date: 2026-04-05
---

# TUI Provider Switch Doesn't Propagate to Agent Worker

## Problem

When switching providers via `/provider <name>` in the TUI, the agent worker continued using the **previous provider**. The TUI footer showed the correct provider/model, but actual LLM calls went to the old provider.

Additionally, `/provider` (no args) listed hardcoded `default_model()` for each provider instead of the user's configured `{provider}_model`. The Tab autocomplete picker for `/provider` had the same issue — showing hardcoded defaults while the actual switch resolved to configured models.

## Root Cause

The `AgentRequest::SetModel { model }` message from handler to worker used a `/`-delimited string for provider detection. Anthropic models were sent without the provider prefix (e.g., `"claude-sonnet-4-6"` instead of `"anthropic/claude-sonnet-4-6"`), so the worker's `model.find('/')` returned `None` and it set the model on the **current** provider instead of switching.

Three call sites duplicated the same broken pattern:
```rust
let full_id = if provider == ProviderKind::Anthropic {
    model.clone()  // no prefix — worker can't detect provider switch
} else {
    format!("{provider}/{model}")
};
```

## Solution

### Decouple display from wire format

Extracted two helpers:

- `format_worker_model(provider, model)` — always includes provider prefix (`"anthropic/claude-sonnet-4-6"`)
- `format_display_model(provider, model)` — no prefix for Anthropic (matches `Settings::active_model_display()`)

`apply_model_switch()` sends `format_worker_model()` to the worker and `format_display_model()` to `app.model`. The worker's existing `/`-based parsing handles the rest correctly.

### Fix provider listing and autocomplete picker

Loaded `Settings` once and used `provider_fields(p).0` to show the configured model, falling back to `default_model()`. Applied the same pattern to `complete_provider()` in `completers.rs` — which had ignored `CompletionContext` (`_ctx`) and used hardcoded `default_model()`. The fix mirrors `complete_model()` in the same file, which already loaded Settings correctly.

## Key Insight

When a single value serves two purposes (display and inter-process communication), they eventually diverge. The fix was to make the divergence explicit: `app.model` is the display value, `SetModel.model` is the wire value. The worker doesn't need to know about display conventions.

When multiple UI surfaces show the same data (listing, autocomplete, handler), they must share the same resolution logic. The autocomplete picker was missed because it used a different code path (`completers.rs` vs `handlers.rs`) with no shared abstraction — both independently resolve "what model does this provider use?" but the completer took a shortcut.

## Files Changed

- `crates/mika-cli/src/tui/commands/handlers.rs` — 3 call sites fixed, 2 helpers added, listing enhanced, 5 tests added
- `crates/mika-cli/src/tui/commands/completers.rs` — `complete_provider()` loads Settings via `CompletionContext` (same pattern as `complete_model()`)

## Related

- #442: Previous provider/model config state fix (same file, same pattern class)
- #342-344: Original `/provider` validation-first pattern
