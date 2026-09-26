---
title: Multi-provider eval matrix architecture for agent loop testing
date: 2026-04-23
category: best-practices
module: mika-agent
problem_type: best_practice
component: testing_framework
severity: medium
applies_when:
  - Adding real-provider integration tests to the eval harness
  - Extending the multi-provider matrix with new providers or scenarios
  - Testing provider-specific behavioral divergences (schema validation, tool call formats)
tags:
  - eval-harness
  - multi-provider
  - integration-testing
  - llm-provider
  - mock-provider
  - calibration
---

# Multi-provider eval matrix architecture for agent loop testing

## Context

Phase 1 of the eval harness (#329) proved that `MockLlmProvider` + `EvalHarness` works for deterministic CI testing. Phase 2 (#338) extended it to support real LLM provider testing and cover additional scenarios. The motivating incident: `inject_task_id_field` produced duplicate entries in a JSON schema `required` array, latent for six weeks. kimi-k2.5 tolerated the duplicate; Anthropic and DeepSeek rejected on provider switch. A provider swap exposed a bug that mocks couldn't catch.

## Guidance

The eval harness supports dual-mode testing: mock (CI) and real-provider (on-demand matrix).

### Real-provider injection

`EvalHarnessBuilder` accepts either mock responses or a real provider via `.llm_provider(Arc<dyn LlmProvider>)`. The `EvalHarness` struct holds both:

- `llm: Arc<dyn LlmProvider>` -- the active provider (mock or real)
- `mock_provider: Option<Arc<MockLlmProvider>>` -- present only for mock-based tests

When using a real provider, `captured_requests` is empty (no mock to intercept). Use `llm_calls` DB table for provider attribution instead.

### Environment gating (two structural layers)

1. **`#[ignore]` on every real-API test** -- `cargo test` never burns API budget accidentally
2. **`MIKA_EVAL_REAL_PROVIDERS` env var** -- comma-separated provider list or `all`. Unknown names hard-fail with a listing of valid providers (prevents silent CI green-while-testing-nothing).

Run real tests: `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai cargo test -p mika-agent --test eval -- --ignored`

### Provider construction

`providers.rs` provides `parse_real_providers()` (parses env var via `parse_provider_list()`) and `create_real_provider(kind)` (constructs from env API keys, returns `None` when key missing). Uses `ProviderKind::from_str()` for case-insensitive parsing and `create_provider()` with `ModelSpec` for construction.

### Three-layer bug detection

1. **Request well-formedness** (`test_request_wellformedness.rs`) -- catches malformed schemas before they leave the process. Runs in CI. Frozen regression fixture reproduces the exact pre-fix `task_id` duplicate shape.
2. **Response rejection handling** (`test_schema_divergence.rs`) -- mock-based tests verify Mika handles out-of-schema responses correctly. Runs in CI.
3. **Provider tolerance drift** (`test_real_provider_matrix.rs`) -- real-API tests detect silent provider-side changes. On-demand via `#[ignore]` + env.

### Calibration mode

Set `MIKA_EVAL_CALIBRATE=1` alongside `MIKA_EVAL_REAL_PROVIDERS`. The matrix runner writes JSON artifacts to `target/eval-calibration/{timestamp}.json`. Use `diff_calibrations(old, new)` to detect tolerance drift (pass->fail or fail->pass).

### Multi-turn testing

`EvalHarness::run_turn(message, responses)` runs additional turns on the same session with fresh mock responses via `MockLlmProvider::clear_and_set()`. Each turn gets its own `trace_id` for scoped DB queries.

## Why This Matters

Provider switches are contract-validation events, not seamless swaps. The eval matrix makes divergences discoverable before they reach production. Without it, bugs masked by one provider's tolerance surface unpredictably on migration.

## When to Apply

- When adding a new LLM provider to `ProviderKind`
- When modifying tool schema construction (any code that emits `ToolDefinition.input_schema`)
- When changing the agent loop's response parsing or tool dispatch paths
- When migrating the active provider for an agent (e.g., Anthropic -> Kimi)

## Examples

Adding a new provider to the matrix:

1. Add the provider to `ProviderKind` enum in `mika-common`
2. Set `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai,kimi,groq,newprovider`
3. Run `cargo test -p mika-agent --test eval -- --ignored`
4. The matrix runner automatically includes it; per-provider convenience test can be added to `test_real_provider_matrix.rs`

Adding a new well-formedness assertion:

1. Add the assertion function in `test_request_wellformedness.rs`
2. Add it to `assert_schema_wellformed()` to run against all tool schemas
3. No `#[ignore]` -- runs in CI on every push

## Related

- Phase 1 harness: `docs/solutions/architecture-patterns/agent-eval-testing-harness-mock-provider.md`
- Multi-provider LLM trait: `docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md`
- Per-turn dedup guard: `docs/solutions/architecture-patterns/per-turn-tool-use-dedup-guard.md`
- Issue: #338
- Phase 1 issue: #329
