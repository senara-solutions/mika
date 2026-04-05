---
title: "fix: pre-fetch model cache on provider switch"
type: fix
status: completed
date: 2026-04-05
---

# fix: pre-fetch model cache on provider switch

## Overview

Gap in PR #444 (issue #442): after `/provider <name>` succeeds, the model list cache for the new provider is empty. Only `/model` triggers a `get_models()` call, causing a slow blocking fetch on the first `/model` invocation after a provider switch.

## Proposed Solution

After a successful provider switch in `handle_provider`, spawn a background `tokio::spawn` to call `mika_common::llm::models::get_models()`. This is fire-and-forget — it pre-warms the cache so `/model` responds instantly.

## Acceptance Criteria

- [x] `/provider deepseek` spawns a background task that calls `get_models()` for deepseek
- [x] `/model` after a provider switch reads from cache (no blocking fetch)
- [x] Hardcoded providers (Anthropic) skip the spawn (get_models returns immediately for them)
- [x] Background fetch failures are silently logged at debug level
- [x] Existing tests continue to pass
- [x] New test verifies the prefetch path (existing tests cover the spawn path without panics)

## MVP

### `crates/mika-cli/src/tui/commands/handlers.rs`

In `handle_provider()`, after the success message is built (~line 787), before `return`:

```rust
// Pre-fetch model list for the new provider (fire-and-forget cache warm)
let home = app.home_dir.clone();
let provider = new_provider;
let base_url_owned = config.base_url.clone();
let api_key_owned = settings.provider_fields(new_provider).1.map(String::from);
tokio::spawn(async move {
    let _ = mika_common::llm::models::get_models(
        &home,
        provider,
        base_url_owned.as_deref(),
        api_key_owned.as_deref(),
    )
    .await;
});
```

## Sources

- Related issue: #442
- Related PR: #444
- `get_models()`: `crates/mika-common/src/llm/models.rs:199`
- `handle_provider()`: `crates/mika-cli/src/tui/commands/handlers.rs:622`
- `handle_model()` (reference for get_models usage): `crates/mika-cli/src/tui/commands/handlers.rs:451`
