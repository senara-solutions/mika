---
title: "fix: provider switch doesn't propagate to agent worker"
type: fix
status: completed
date: 2026-04-05
issue: 451
---

# Fix: Provider Switch Doesn't Propagate to Agent Worker

## Overview

Two related bugs in the TUI `/provider` command cause the agent worker to use the wrong LLM provider after switching. The TUI footer shows the correct provider/model, but actual LLM calls go to the old provider.

## Problem Statement

### Bug 1: Worker stays on old provider

When `/provider anthropic` is run (from openrouter), the handler sends `AgentRequest::SetModel { model: "claude-sonnet-4-6" }` — no provider prefix. The worker at `chat.rs:409` does `model.find('/')` → `None`, so it sets the model on the **current** `worker_settings.llm_provider` (still openrouter). OpenRouter receives `"claude-sonnet-4-6"` and maps it to `qwen/qwen3-max`.

### Bug 2: Listing shows wrong models

`/provider` (no args) shows `p.default_model()` for each provider (e.g., `openrouter — anthropic/claude-sonnet-4`). But the user has `openrouter_model = "qwen/qwen-plus"` configured. The listing should show the configured model, falling back to the default.

### Root Cause

The `full_id` variable conflates two concerns — display value and worker message — through a single string. Anthropic gets special treatment (no prefix), which breaks the worker's `/`-based provider detection. This pattern is duplicated in 3 call sites.

## Proposed Solution

### 1. Extract display/worker formatting helpers

Two new functions in `handlers.rs`:

```rust
/// Format model string for the agent worker — always includes provider prefix.
fn format_worker_model(provider: ProviderKind, model: &str) -> String {
    format!("{provider}/{model}")
}

/// Format model string for TUI display — no prefix for Anthropic.
fn format_display_model(provider: ProviderKind, model: &str) -> String {
    if provider == ProviderKind::Anthropic {
        model.to_string()
    } else {
        format!("{provider}/{model}")
    }
}
```

### 2. Fix `apply_model_switch()` (`handlers.rs:575`)

Decouple display from worker message:

- `app.model` gets `format_display_model(target_provider, model_name)`
- `AgentRequest::SetModel` gets `format_worker_model(target_provider, model_name)`
- Rename `full_id` param to `display_id` for clarity

### 3. Fix `handle_model()` direct path (`handlers.rs:564`)

Replace:
```rust
let full_id = if target_provider == ProviderKind::Anthropic {
    model_name.to_string()
} else {
    format!("{target_provider}/{model_name}")
};
```
With:
```rust
let display_id = format_display_model(target_provider, model_name);
```

### 4. Fix `handle_provider()` switch path (`handlers.rs:731`)

Replace the inline `full_id` construction with `format_display_model()` for the display value. The `apply_model_switch` call (if refactored to use it) or direct `SetModel` send uses `format_worker_model()`.

### 5. Fix `handle_provider()` set model path (`handlers.rs:674`)

Same pattern — use `format_display_model()` for `app.model`, `format_worker_model()` for the SetModel message.

### 6. Fix `/provider` listing (`handlers.rs:630-637`)

Load settings once and show configured models:

```rust
let settings = mika_common::config::Settings::load_for_agent(&app.global_home, &app.home_dir).ok();
for &p in ProviderKind::ALL {
    let model = settings.as_ref()
        .and_then(|s| s.provider_fields(p).0.map(String::from))
        .unwrap_or_else(|| p.default_model().to_string());
    let _ = write!(out, "\n  {} — {}{}", p, model, marker);
}
```

## Acceptance Criteria

- [x] `/provider anthropic` from openrouter actually switches the worker to Anthropic
- [x] `/provider openrouter` from anthropic sends `openrouter/qwen/qwen-plus` (not `openrouter/anthropic/claude-sonnet-4`) to worker
- [x] `/provider` listing shows user's configured model per provider, falling back to default
- [x] `app.model` (footer display) stays user-friendly: no `anthropic/` prefix for Anthropic
- [x] All existing tests pass
- [x] New tests verify worker message always contains provider prefix
- [x] New test verifies `/provider` listing shows configured model

## MVP

### `crates/mika-cli/src/tui/commands/handlers.rs`

**New helpers** (near `apply_model_switch`):

```rust
fn format_worker_model(provider: ProviderKind, model: &str) -> String {
    format!("{provider}/{model}")
}

fn format_display_model(provider: ProviderKind, model: &str) -> String {
    if provider == ProviderKind::Anthropic {
        model.to_string()
    } else {
        format!("{provider}/{model}")
    }
}
```

**`apply_model_switch()`** — change `full_id` to `display_id`, send `format_worker_model()` to worker:

```rust
fn apply_model_switch(
    app: &mut App<'_>,
    target_provider: ProviderKind,
    model_name: &str,
    display_id: String,
    display: &str,
) -> String {
    // ... persist logic unchanged ...
    
    let worker_model = format_worker_model(target_provider, model_name);
    app.model = display_id;
    if app.agent_tx.send(AgentRequest::SetModel { model: worker_model }).is_err() {
        return WORKER_NOT_RESPONDING.to_string();
    }
    // ...
}
```

**`handle_model()` direct path** (line 564):

```rust
let display_id = format_display_model(target_provider, model_name);
apply_model_switch(app, target_provider, model_name, display_id.clone(), &display_id)
```

**`handle_provider()` set model** (line 674):

```rust
let display_id = format_display_model(app.provider, value);
let worker_model = format_worker_model(app.provider, value);
app.model = display_id;
// send worker_model to worker
```

**`handle_provider()` switch** (line 731):

```rust
let display_id = format_display_model(new_provider, &model);
let worker_model = format_worker_model(new_provider, &model);
app.model = display_id;
// send worker_model to worker
```

**`handle_provider()` listing** (line 630-637):

```rust
let settings = mika_common::config::Settings::load_for_agent(&app.global_home, &app.home_dir).ok();
for &p in ProviderKind::ALL {
    let model = settings.as_ref()
        .and_then(|s| s.provider_fields(p).0.map(String::from))
        .unwrap_or_else(|| p.default_model().to_string());
    let marker = if p == app.provider { " (current)" } else { "" };
    let _ = write!(out, "\n  {} — {}{}", p, model, marker);
}
```

### Tests

```rust
#[tokio::test]
async fn test_provider_switch_worker_always_gets_provider_prefix() {
    let (mut app, mut rx, tmp) = test_app().await;
    // Setup: start on anthropic, switch to deepseek
    // Verify SetModel contains "deepseek/" prefix
    // Then switch back to anthropic
    // Verify SetModel contains "anthropic/" prefix
}

#[tokio::test]
async fn test_provider_listing_shows_configured_model() {
    let (mut app, _rx, tmp) = test_app().await;
    // Write config with openrouter_model = "qwen/qwen-plus"
    // Run handle_provider(&mut app, "")
    // Assert output contains "qwen/qwen-plus" not "anthropic/claude-sonnet-4"
}
```

## Sources

- Prior fix: [docs/solutions/ui-bugs/tui-provider-model-config-state-divergence.md](docs/solutions/ui-bugs/tui-provider-model-config-state-divergence.md) — #442
- Prior fix: [docs/solutions/ui-bugs/tui-slash-command-reliability-clear-provider-model.md](docs/solutions/ui-bugs/tui-slash-command-reliability-clear-provider-model.md) — #342-344
- Existing helper: `Settings::active_model_display()` at `config.rs:866` — reference for display format
- Existing helper: `Settings::provider_fields()` at `config.rs:771` — reads configured model per provider
- Related issue: #451
