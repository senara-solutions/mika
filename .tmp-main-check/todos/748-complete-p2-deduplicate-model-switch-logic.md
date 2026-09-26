---
status: pending
priority: p2
issue_id: "748"
tags: [code-review, quality]
dependencies: []
---

# Duplicated apply-model-switch logic in handle_model

## Problem Statement

The alias path (lines ~506-578) and direct-name path (lines ~580-642) in `handle_model` duplicate ~60 lines of identical logic: validate, optionally switch provider, persist model, update app state, send to worker, format output.

## Findings

- **File:** `crates/mika-cli/src/tui/commands/handlers.rs` — `handle_model()`
- Both paths share: `validate_provider_switch_for()`, `write_config_toml()` for provider and model, `app.model`/`app.provider` mutation, `AgentRequest::SetModel` send, `persist_warning`/`provider_note` formatting
- Only difference: how `target_provider`, `model_name`, `full_id`, and display label are determined

## Proposed Solution

Extract a shared helper:

```rust
fn apply_model_switch(
    app: &mut App<'_>,
    target_provider: ProviderKind,
    model_name: &str,
    full_id: String,
    display: &str,
) -> String { ... }
```

Both the alias path and direct-name path resolve their inputs, then call this single function.

**Effort:** Small (~30 min)

## Acceptance Criteria

- [ ] Single `apply_model_switch` helper replaces duplicated logic
- [ ] All existing handler tests pass
- [ ] No behavioral change
