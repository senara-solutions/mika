---
title: "fix: /model command misinterprets OpenRouter model names as provider switches"
type: fix
status: completed
date: 2026-04-05
---

# fix: /model command misinterprets OpenRouter model names as provider switches

## Overview

Multiple bugs in the TUI `/model` and `/provider` commands around OpenRouter model name handling and user experience:

1. **`/model` on OpenRouter misinterprets models as provider switches**: `qwen/qwen-plus` is a valid OpenRouter model name, but the handler parses `qwen/` as a ProviderKind and switches to the Qwen provider instead of setting `openrouter_model = "qwen/qwen-plus"`.

2. **`/model` completer uses stale provider**: `complete_model()` reads `settings.llm_provider` from disk config, but should use `app.provider` (the TUI's current in-session provider state) to show the correct model list.

3. **"Note" messages are confusing**: "Note: openrouter_model is still set (kept for switching back)" adds noise that confuses users without providing clear value.

## Root Cause

OpenRouter models use `provider/model` format (e.g., `qwen/qwen-plus`, `anthropic/claude-sonnet-4`). This collides with the TUI's cross-provider switching syntax which also uses `provider/model`. The `/model` handler at `handlers.rs:549-557` always interprets a valid ProviderKind prefix as a provider switch, regardless of context.

## Proposed Solution

### Fix 1: Don't interpret slashes as provider switches when on OpenRouter

In `handle_model()` (handlers.rs:549-557), when the current provider (`app.provider`) is OpenRouter, treat the entire input as a model name for OpenRouter — don't parse the slash as a provider switch. OpenRouter is the only provider that uses slash-separated model names.

Add `ProviderKind::model_names_contain_slash() -> bool` returning `true` for OpenRouter. Use this in both `handle_model()` and the worker's `SetModel` handler to skip provider prefix parsing when appropriate.

**Before:** `/model qwen/qwen-plus` on OpenRouter → switches to Qwen, writes `qwen_model`
**After:** `/model qwen/qwen-plus` on OpenRouter → stays on OpenRouter, writes `openrouter_model = "qwen/qwen-plus"`

### Fix 2: Pass current TUI provider to model completer

`CompletionContext` needs the current `app.provider` so `complete_model()` shows models for the active TUI provider, not the config file's `llm_provider`.

### Fix 3: Remove "Note" messages from provider switch

Remove the "Note: {key} is still set (kept for switching back)" messages from `/provider` handler. The stale config keys don't affect behavior and the notes confuse users.

## Acceptance Criteria

- [x] `/model qwen/qwen-plus` on OpenRouter sets `openrouter_model = "qwen/qwen-plus"` (no provider switch)
- [x] `/model deepseek` (alias) on OpenRouter still switches to DeepSeek provider correctly
- [x] `/model` (no args) on OpenRouter shows OpenRouter's model list from cache
- [x] `/model` completer shows models for the TUI's current provider, not disk config
- [x] "Note: {key} is still set" messages are removed from `/provider` output
- [x] Worker correctly handles `openrouter/qwen/qwen-plus` format (first-slash = provider prefix)
- [x] Existing tests pass; update tests for new behavior

## Key Files

- `crates/mika-cli/src/tui/commands/handlers.rs` — `handle_model()` (lines 549-557), `apply_model_switch()`, `/provider` handler note messages
- `crates/mika-cli/src/tui/commands/completers.rs` — `complete_model()` provider source
- `crates/mika-cli/src/tui/commands/autocomplete.rs` — `CompletionContext` struct
- `crates/mika-common/src/llm/mod.rs` — `ProviderKind` (add `model_names_contain_slash()`)
- `crates/mika-cli/src/commands/chat.rs` — Worker `SetModel` handler (needs same OpenRouter awareness)
