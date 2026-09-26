---
title: "Centralize test Settings and use builder DI for optional agent dependencies"
date: 2026-04-23
category: best-practices
module: eval-harness
problem_type: best_practice
component: testing_framework
severity: medium
applies_when:
  - Adding new fields to Settings that break scattered test helpers
  - Writing eval tests that need optional agent dependencies (embedding, MCP, GitHub, Brave)
  - Extending MockLlmProvider with new configurable behaviors
tags:
  - eval-harness
  - settings
  - test-defaults
  - dependency-injection
  - mock-provider
  - test-utils
---

# Centralize test Settings and use builder DI for optional agent dependencies

## Context

The eval harness (`crates/mika-agent/tests/eval/`) exercises `run_agent()` with a `MockLlmProvider` for deterministic integration testing. Two friction points emerged during Phase 1 (#330):

1. **Settings duplication** -- `dummy_settings()` was duplicated in both `harness.rs` and `test_utils.rs`, each explicitly listing 60+ fields. Any new `Settings` field required updating both copies or hitting a compile error in the wrong place.

2. **Hardcoded None dependencies** -- `EvalHarness::run()` hardcoded `embedding_client: None`, `brave_api_key: None`, `github_token: None`, and `mcp_manager: None` in the `AgentParams` construction. Tests that needed these dependencies (e.g., Layer 3 hybrid search, GitHub tool paths) had no way to inject them.

## Guidance

### Settings::test_defaults()

Place the canonical test `Settings` constructor on `Settings` itself in `mika-common`, gated behind `#[cfg(any(test, feature = "test-utils"))]`:

```rust
// crates/mika-common/src/config.rs
impl Settings {
    #[cfg(any(test, feature = "test-utils"))]
    pub fn test_defaults() -> Self {
        Self {
            llm_provider: ProviderKind::Anthropic,
            llm_max_tokens: 4096,
            // ... all fields explicitly listed
        }
    }
}
```

Existing call sites (like `test_utils::dummy_settings()`) delegate to `Settings::test_defaults()` for backward compatibility.

### Builder DI for optional dependencies

Add builder methods on `EvalHarnessBuilder` that mirror the `AgentParams` fields:

```rust
pub fn embedding_client(mut self, client: EmbeddingClient) -> Self { ... }
pub fn brave_api_key(mut self, key: impl Into<String>) -> Self { ... }
pub fn github_token(mut self, token: impl Into<String>) -> Self { ... }
pub fn mcp_manager(mut self, mgr: McpManager) -> Self { ... }
```

Store as `Option<T>` on `EvalHarness`. Thread through to `AgentParams` in `run()` using `as_ref()` / `as_deref()`. Default = `None` preserves existing test behavior.

### MockLlmProvider::health_error()

For new mock behaviors, add builder methods on `MockLlmProviderBuilder` that configure `MockProviderConfig`:

```rust
pub fn health_error(mut self, error: LlmError) -> Self {
    self.config.health_error = Some(error);
    self
}
```

The `LlmProvider` trait method reads from config:

```rust
async fn check_health(&self) -> Result<(), LlmError> {
    match &self.config.health_error {
        Some(err) => Err(err.clone()),
        None => Ok(()),
    }
}
```

## Why This Matters

- **Compile-time field protection**: `Settings::test_defaults()` on the real struct means any new field addition produces a compile error at the canonical location, not in a distant test file.
- **Single source of truth**: One constructor instead of N copies eliminates silent drift between test helpers.
- **Unblocks downstream**: DI builders are the prerequisite for multi-provider eval (#338), KG scenario testing (#740), and grounding scenario testing (#741).
- **Consistent builder pattern**: All four DI methods follow the same `Option<T>` + `as_ref()`/`as_deref()` pattern as existing harness builders (`.tools()`, `.skills()`, `.session_id()`).

## When to Apply

- Adding a new field to `Settings` -- add it to `test_defaults()` and all test code updates automatically
- Writing an eval test that needs an optional `AgentParams` dependency -- use the corresponding builder method
- Adding a new mockable behavior to `MockLlmProvider` -- add a config field + builder method, not a separate mock struct

## Examples

**Before** (D2 problem -- adding a new Settings field broke two files):
```rust
// harness.rs -- copy 1
fn dummy_settings() -> Settings {
    Settings { /* 60+ fields */ }
}

// test_utils.rs -- copy 2
pub fn dummy_settings() -> Settings {
    Settings { /* 60+ fields, possibly drifted */ }
}
```

**After** (single source of truth):
```rust
// mika-common/src/config.rs -- canonical
Settings::test_defaults()

// test_utils.rs -- delegates
pub fn dummy_settings() -> Settings {
    Settings::test_defaults()
}

// harness.rs -- uses directly
let settings = Settings::test_defaults();
```

**DI builder usage** (for downstream tests):
```rust
let harness = EvalHarness::builder()
    .brave_api_key("test-key")
    .github_token("ghp_test")
    .responses(vec![...])
    .build()
    .await?;
```

## Related

- #340 -- Eval harness Phase 1 follow-ups (this PR)
- #330 -- Phase 1 eval harness (original)
- #338 -- Phase 2 multi-provider eval matrix (uses DI builders)
- #740 -- KG golden dataset testing (uses embedding_client DI)
- #741 -- Grounding scenario testing (uses github_token, health_error)
