---
title: "Golden dataset eval: mock-based unit tier tests verify invocation patterns, not tool success"
date: 2026-04-23
category: best-practices
module: eval-harness
problem_type: best_practice
component: testing_framework
severity: medium
applies_when:
  - Writing golden dataset scenarios that call tools requiring pre-existing DB state
  - Adding new scenarios to the eval golden directory
  - Interpreting unit-tier test failures vs integration-tier failures
  - Debugging why a golden scenario passes with mocks but fails with real providers
tags:
  - eval-harness
  - golden-dataset
  - mock-provider
  - three-tier-testing
  - tool-invocation
  - scoring-framework
---

# Golden dataset eval: mock-based unit tier tests verify invocation patterns, not tool success

## Context

The golden dataset (#339) adds 25 curated end-to-end scenarios across memory, tool selection, conversation quality, and skill-specific capabilities. Each scenario runs in three tiers: unit (mock), integration (real provider), and calibration (artifact capture).

A key design constraint emerged during implementation: unit-tier tests run the real tool dispatch pipeline (store_fact, search_memory, update_fact, etc.) against an **empty in-memory SQLite database**. The `MockLlmProvider` controls what the LLM "says" (which tools to call, what text to respond with), but the actual tool handlers execute against the DB.

This creates a structural gap: tools that require pre-existing data (e.g., `update_fact` with a `fact_id`) will return tool errors at the unit tier, but the test may still pass because most assertions check tool *invocation* (`assert_tools_include`) rather than tool *success* (`assert_no_tool_errors`).

## Guidance

**Unit-tier golden scenarios test agent wiring, not tool correctness.** The mock sequence defines a "script" — the agent loop follows it regardless of actual tool results. This is by design: the unit tier proves the agent calls the right tools in the right order; the integration tier proves the tools produce correct results with real providers.

When authoring new scenarios:

1. **Do not pre-seed the DB.** `AsyncDatabase` has no `store_fact()` method accessible from tests. The correct pattern is to let the mock sequence control the agent's behavior.

2. **Use `assert_tools_include` / `assert_tool_order` for invocation checks.** These verify the mock-driven script ran as expected.

3. **Use `assert_no_tool_errors` only when the tool doesn't need pre-existing DB state.** For example, `store_fact` succeeds on an empty DB (it creates new rows), but `update_fact` fails (it needs an existing row).

4. **Document the limitation in scenario comments** when a tool call is expected to fail at the unit tier. See `memory_fact_update.rs` for the pattern.

5. **Set `expected_tokens` in `GoldenScenarioMeta` based on complexity class**, not exact measurements. Class averages: memory ~$0.02, tool selection ~$0.02, conversation quality ~$0.03, skill-specific ~$0.04 per scenario per provider.

## Why This Matters

Without understanding this three-tier distinction, scenario authors may:
- Waste time trying to pre-seed facts in the DB (no API for it from tests)
- Add `assert_no_tool_errors` to scenarios where tool failure is expected and harmless
- Misinterpret unit-tier passes as proof of end-to-end correctness
- Miss the fact that tool success is only verified at the integration tier

The scoring framework reinforces this: `GoldenOutcome` has separate `hard_assertions` (pass/fail, gating) and `soft_tags` (quality:* namespace, observability-only). The `GoldenOutcome::from_params` constructor panics on empty `hard_assertions` to prevent vacuous-truth passes.

## When to Apply

- Adding scenarios to `crates/mika-agent/tests/eval/golden/`
- Interpreting calibration artifacts from `target/eval-calibration/`
- Debugging test failures after prompt changes or model upgrades
- Reviewing PRs that modify golden scenario files

## Examples

**Correct pattern — tool invocation test (unit tier):**

```rust
// memory_fact_update.rs — update_fact will fail against empty DB, but
// we're testing that the agent calls search_memory BEFORE update_fact
let harness = EvalHarness::builder()
    .responses(vec![
        tool_call_response("search_memory", json!({"query": "deadline"})),
        tool_call_response("update_fact", json!({"fact_id": "1", "content": "..."})),
        text_response("Updated!"),
    ])
    .build().await.unwrap();

let trace = harness.run("Update the deadline").await.unwrap();
assert_tool_order(&trace, &["search_memory", "update_fact"]); // invocation order
// NOT assert_no_tool_errors — update_fact fails on empty DB
```

**Correct pattern — tool success test (where tool doesn't need pre-existing state):**

```rust
// store_fact creates new rows, so it succeeds on empty DB
let harness = EvalHarness::builder()
    .responses(vec![
        tool_call_response("store_fact", json!({"category": "person", ...})),
        text_response("Stored!"),
    ])
    .build().await.unwrap();

let trace = harness.run("Remember that John is an engineer").await.unwrap();
assert_tools_include(&trace, &["store_fact"]);
assert_no_tool_errors(&trace); // safe — store_fact succeeds on empty DB
```
