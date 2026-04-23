---
title: "feat: Agent eval harness Phase 2 — multi-provider eval matrix and real API calibration"
type: feat
status: active
date: 2026-04-23
origin: docs/plans/338-phase-2-multi-provider.md
issue: "#338"
---

# Agent Eval Harness Phase 2 — Multi-Provider Eval Matrix and Real API Calibration

## Overview

Extend the Phase 1 eval harness (MockLlmProvider + EvalHarness, 16 integration tests) to support real LLM provider testing and cover remaining scenarios. Ships the multi-provider matrix runner, JSON-schema divergence tests, per-skill provider override testing, and calibration mode plumbing. Does NOT ship the full scenario catalog (#339) or KG-specific scenarios (#740).

## Problem Frame

The motivating incident: `inject_task_id_field` produced duplicate entries in the `required` array (latent since commit `440a9d59`). kimi-k2.5 tolerated the duplicate; Anthropic and DeepSeek rejected on provider switch. A provider swap exposed a weeks-old bug. This ticket makes provider swaps a tested event — three bug-class layers: (1) request well-formedness, (2) response rejection handling, (3) provider tolerance drift.

(see origin: `docs/plans/338-phase-2-multi-provider.md`)

## Requirements Trace

- R1. Real-API tests gated behind `MIKA_EVAL_REAL_PROVIDERS` env var + `#[ignore]` structural gate
- R2. Matrix runner covering Anthropic, OpenAI, Kimi, Groq — scenarios as functions, providers as runtime parameter
- R3. JSON-schema strict-mode divergence tests (required-array dedup, enum exhaustiveness, unknown fields, missing required)
- R4. Request-side JSON-schema well-formedness assertions (catches `task_id`-class bugs)
- R5. Per-skill provider override test with two distinguishable mock providers
- R6. Max-steps + continuation turn test
- R7. Multi-turn conversation persistence test
- R8. Real API calibration mode with ephemeral artifact output and diff tool
- R9. Required-tools gate already covered in Phase 1 (`test_required_tools_gate.rs`) — not duplicated

## Scope Boundaries

- In scope: matrix runner, env gating, JSON-schema divergence tests, per-skill override test, max-steps continuation test, multi-turn test, calibration harness stub, request well-formedness assertions
- Not in scope: golden dataset curation (→ #339), KG-specific scenarios (→ #740), grounding/fabrication scenarios (→ #741), committed calibration baseline (→ #742), Langfuse integration

### Deferred to Separate Tasks

- Weekly calibration CI + drift-detection PR automation → #742
- Golden dataset end-to-end quality testing → #339
- KG scenario coverage → #740
- Grounding regression scenarios → #741

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/tests/eval/` — 16 existing eval tests, `EvalHarness` builder, `AgentTrace`, assertion helpers
- `crates/mika-agent/tests/eval/harness.rs` — `EvalHarnessBuilder` with `responses()`, `tools()`, `skills()`, `provider_name()`, `model_name()`, `message_sender()` builders. `AgentParams.llm` currently hardcoded to `self.mock_provider.as_ref()`
- `crates/mika-common/src/llm/mod.rs` — `LlmProvider` trait, `ProviderKind` enum (11 variants), `create_provider(spec, max_tokens)`, `ProviderKind::ALL`
- `crates/mika-common/src/llm/mock.rs` — `MockLlmProvider` with sequence-based responses, `captured_requests()`, builder, helper constructors
- `crates/mika-common/src/config.rs` — `Settings::make_provider_for(provider, model_override)` creates real providers from config
- `crates/mika-agent/src/agent.rs` — `AgentParams` (22 fields), `run_agent()`, `MAX_TOOL_STEPS = 20`, `attempt_continuation_turn()`, `resolve_skill_llm_override()`
- `crates/mika-agent/src/skills/index.rs:1337` — `inject_task_id_field()` with dedup guard (already fixed)
- `crates/mika-agent/src/skills/mod.rs` — `SkillRegistry::from_test_entries()`, `SkillEntry` construction pattern
- `crates/mika-agent/tests/eval/test_required_tools_gate.rs` — example of constructing `SkillEntry` with constraints for tests

### Institutional Learnings

- Per-turn tool_use dedup guard (#582) — kimi-k2.5 emits duplicate tool_use blocks; guard uses `HashMap<(name, args)>` cache per `process_tool_calls()` invocation
- XML tool calls not executed — non-Anthropic providers emit tool calls as XML text; two-layer defense (`extract_xml_tool_calls` + `detect_text_based_tool_call`)
- Prose-style tool call leaks — LLMs emit `tool_name({"key": "value"})` prose; regex detection gated by registered tool names
- Skill LLM override must filter by MatchReason::Keyword (#265/#463) — always_on skills don't contribute constraints
- Tool field aliases for LLM tokenization quirks — MiniMax M2.7 emits `"reason"` instead of `"reasoning"`; narrow engine-side alias
- Per-skill LLM override is DB-only via `skill_overrides` table (schema v20); `[llm]` in skill.toml is `#[serde(skip)]` since #504
- Three-tier resolution: DB override > manifest `[llm]` > agent default (see `docs/solutions/architecture-patterns/skill-llm-override-db-layer-and-linked-unblock.md`)

### External References

- Phase 1 plan: `docs/plans/2026-03-30-001-feat-agent-eval-testing-harness-plan.md`
- Phase 1 brainstorm: `docs/brainstorms/2026-03-30-agent-eval-harness-brainstorm.md`
- KG retrospective: `docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`

## Key Technical Decisions

- **D1 — Single env gate:** `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai,kimi,groq` (comma-separated). Unknown names hard-fail. Empty/unset → real-provider tests stay `#[ignore]`-respected. One var beats per-provider flags for ergonomics.
- **D2 — Four providers:** Anthropic + OpenAI (strict validators) + Kimi + Groq (permissive with distinct profiles). Two strict + two permissive covers orthogonal validation behavior.
- **D3 — Scenarios as functions:** `async fn scenario_X(provider, ...) -> Result<ScenarioOutcome>`. Matrix runner iterates providers × scenarios. Per-provider `#[test] #[ignore]` tests preserve `cargo test --list` discoverability.
- **D4 — Schema divergence split:** Mock-based harness tests for Mika's handling of rejection; separate opt-in real-API calibration for provider tolerance drift.
- **D5 — Per-skill override:** Two mock providers with distinct names in same session; DB `skill_overrides` row routes the test skill to `skill-mock`. Trace asserts provider attribution per LLM call.
- **D6 — Additional scenarios mock-based:** Max-steps continuation (21 ToolUse responses), multi-turn persistence (two runs on same session). Required-tools gate already has Phase 1 coverage.
- **D7 — Calibration ephemeral:** Artifacts to `target/eval-calibration/{timestamp}.json` (gitignored). `eval-diff` binary diffs two artifacts, includes per-scenario token counts. Committed baseline deferred to #742.
- **D8 — Structural cost control:** Every real-API test carries `#[ignore]`. No plan-level dollar estimate; workflow-controlled via CI job timeout.
- **D9 — Request well-formedness:** Required-array dedup, properties/required coherence, enum validity, reserved-name shadowing. Frozen regression fixture reproducing the pre-fix `task_id` schema.

## Open Questions

### Resolved During Planning

- **Is #340 still a blocker?** No — #340 is CLOSED. DI builder methods are merged. The harness already has `provider_name()` and `model_name()` builders. Real-provider injection requires extending `EvalHarness` with an `llm_provider(Arc<dyn LlmProvider>)` builder path.
- **Does `inject_task_id_field` still have the dedup bug?** No — line 1349 of `skills/index.rs` now has `if !required.contains(&task_id_val)`. The D9a frozen fixture tests against the pre-fix shape to ensure the guard stays active.
- **Does `run_turn` exist for multi-turn tests?** No — the harness currently only has `run(message)`. Unit 5 adds multi-turn support.

### Deferred to Implementation

- Exact `ScenarioOutcome` struct fields — depends on what real-API runs surface
- Whether `eval-diff` should be a binary or a test helper function — implementation discovery
- Kimi provider routing details (direct vs via OpenRouter) — depends on available API keys at test time

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
Test entry points:
  cargo test -p mika-agent --test eval           # mock tests only (CI)
  cargo test -p mika-agent --test eval -- --ignored  # + real-API tests (with env)

Three bug-class layers:
  Layer 1 (D9): Request well-formedness     → test_request_wellformedness.rs  (always CI)
  Layer 2 (D4): Response rejection handling → test_schema_divergence.rs       (mock, always CI)
  Layer 3 (D4): Provider tolerance drift    → test_real_provider_matrix.rs    (#[ignore] + env)

Matrix runner shape:
  for provider in configured_providers:
    for scenario in SCENARIOS:
      let outcome = scenario(provider).await;
      results.push((provider, scenario.name, outcome));
  report(results)

Calibration mode (MIKA_EVAL_CALIBRATE=1):
  Same matrix run → writes JSON artifact to target/eval-calibration/
  eval-diff old.json new.json → exit 1 if tolerance changed
```

## Implementation Units

- [ ] **Unit 1: Real-provider env gate and provider construction helpers**

  **Goal:** Implement the `MIKA_EVAL_REAL_PROVIDERS` env var parser and helper to construct real providers from environment.

  **Requirements:** R1, R2

  **Dependencies:** None

  **Files:**
  - Create: `crates/mika-agent/tests/eval/providers.rs`
  - Modify: `crates/mika-agent/tests/eval.rs` (add `pub mod providers`)

  **Approach:**
  - `parse_real_providers()` → parses `MIKA_EVAL_REAL_PROVIDERS` env var, returns `Vec<ProviderKind>`. Hard-fails on unknown names with listing of valid providers. Returns empty vec when unset.
  - `is_real_provider_enabled(kind: ProviderKind)` → convenience check
  - `create_real_provider(kind: ProviderKind)` → constructs provider using `create_provider()` with API key from env. Skips (returns `None`) when key not configured.
  - `skip_unless_real_providers!()` macro or helper fn — early-returns from tests when no real providers configured
  - Reuse `ProviderKind::from_str()` (case-insensitive) for parsing; `ProviderKind::ALL` for error listing

  **Patterns to follow:**
  - `crates/mika-common/src/llm/mod.rs` — `create_provider()`, `ProviderKind::from_str()`
  - `crates/mika-common/src/config.rs` — `Settings::provider_fields()` for API key env var names

  **Test scenarios:**
  - Happy path: `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai` parses to `[Anthropic, OpenAi]`
  - Happy path: `MIKA_EVAL_REAL_PROVIDERS=all` returns all 11 providers
  - Edge case: empty/unset env var returns empty vec
  - Error path: `MIKA_EVAL_REAL_PROVIDERS=anthropic,foobar` panics with error listing valid providers
  - Edge case: case-insensitive parsing (`Anthropic` = `anthropic` = `ANTHROPIC`)

  **Verification:** `cargo test -p mika-agent --test eval` passes with and without the env var set.

- [ ] **Unit 2: EvalHarness real-provider injection path**

  **Goal:** Extend `EvalHarnessBuilder` to accept a real `Arc<dyn LlmProvider>` instead of always creating a `MockLlmProvider`.

  **Requirements:** R1, R2

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/tests/eval/harness.rs`
  - Modify: `crates/mika-agent/tests/eval/trace.rs`

  **Approach:**
  - Add `.llm_provider(Arc<dyn LlmProvider>)` builder method to `EvalHarnessBuilder`
  - When set, `build()` uses it instead of constructing `MockLlmProvider`
  - `EvalHarness` gains `llm: Arc<dyn LlmProvider>` field (replaces current `mock_provider`). Keep `mock_provider: Option<Arc<MockLlmProvider>>` for backward compat — mock-based tests still use `captured_requests()`
  - `AgentParams.llm` references the new `llm` field
  - `AgentTrace::from_run()` handles `None` mock provider gracefully — `captured_requests` is empty for real providers
  - Existing mock-based `run()` path unchanged — `responses()` builder still creates `MockLlmProvider` as before

  **Patterns to follow:**
  - Current `EvalHarnessBuilder::build()` mock construction pattern
  - `AgentParams` llm field: `llm: &'a dyn LlmProvider`

  **Test scenarios:**
  - Happy path: existing mock-based tests still pass unchanged
  - Happy path: builder with `.llm_provider(arc)` uses the provided provider
  - Edge case: setting both `.responses()` and `.llm_provider()` — last-wins or error (design choice during implementation)

  **Verification:** All 16 existing eval tests pass. New builder path compiles and is usable from test code.

- [ ] **Unit 3: Request-side JSON-schema well-formedness assertions (D9)**

  **Goal:** Catch `task_id`-class bugs at request construction time. Frozen regression fixture for the pre-fix duplicate-required-entry schema.

  **Requirements:** R4

  **Dependencies:** None (can run in parallel with Units 1-2)

  **Files:**
  - Create: `crates/mika-agent/tests/eval/test_request_wellformedness.rs`
  - Create: `crates/mika-agent/tests/eval/fixtures/task_id_duplicate_required.json`
  - Modify: `crates/mika-agent/tests/eval.rs` (add module)

  **Approach:**
  - Well-formedness assertion functions: `assert_no_duplicate_required()`, `assert_required_in_properties()`, `assert_enum_valid()`, `assert_no_reserved_name_shadowing()`
  - Test against `default_tools()` — iterate all tool definitions, run all assertions
  - Test against synthetic skill tools with `inject_task_id_field` — construct a schema, inject, assert no duplicates
  - D9a frozen fixture: committed JSON file with the exact pre-fix `task_id` duplicate schema shape. Test MUST fail against fixture when dedup assertion removed; MUST pass with it
  - D9b emitter enumeration: doc comment enumerating covered schema emitters (default_tools, inject_task_id_field, skill tools). MCP tool schemas if accessible from test context
  - D9c additional rules: required/properties coherence, enum validity, reserved-name check

  **Patterns to follow:**
  - `crates/mika-agent/src/skills/index.rs:1337` — `inject_task_id_field` for schema manipulation
  - `crates/mika-agent/src/tools/mod.rs` — `ToolDefinition`, `default_tools()`

  **Test scenarios:**
  - Happy path: `default_tools()` schemas all pass well-formedness checks
  - Regression: frozen fixture with duplicate `task_id` in `required` fails `assert_no_duplicate_required()`
  - Happy path: schema after `inject_task_id_field` (current code with dedup) passes
  - Edge case: schema with empty `required` array passes
  - Edge case: schema with `required` entry not in `properties` fails coherence check
  - Edge case: schema with duplicate enum values fails

  **Verification:** `cargo test -p mika-agent --test eval test_request_wellformedness` green. No `#[ignore]` — runs in CI on every push.

- [ ] **Unit 4: JSON-schema divergence response-handling tests (D4)**

  **Goal:** Verify Mika correctly handles provider rejection of malformed schemas and out-of-schema responses.

  **Requirements:** R3

  **Dependencies:** None

  **Files:**
  - Create: `crates/mika-agent/tests/eval/test_schema_divergence.rs`
  - Modify: `crates/mika-agent/tests/eval.rs` (add module)

  **Approach:**
  - Four mock-based test scenarios, each using `MockLlmProvider` with crafted responses:
    1. **Required-array dedup (response-side):** Mock an LLM response where tool_call input has a field name matching `required` — verify dispatch handles it
    2. **Enum exhaustiveness:** Mock response with tool_call argument containing an out-of-schema enum value — verify structured error surfaces at tool dispatch
    3. **Unknown fields:** Mock response with extra fields in tool_call input — verify parse path handles gracefully (documents per-adapter tolerance)
    4. **Missing required field:** Mock response where tool_call input omits a declared-required field — verify downstream parse surfaces structured error or agent retries
  - Each test constructs a `ToolRegistry` with a synthetic test tool that has a strict schema, then uses `MockLlmProvider` to inject the malformed response

  **Patterns to follow:**
  - `crates/mika-agent/tests/eval/test_tool_calling.rs` — tool call mock setup
  - `crates/mika-agent/tests/eval/test_required_tools_gate.rs` — custom skill/tool construction

  **Test scenarios:**
  - Happy path: well-formed tool call input processes correctly
  - Error path: out-of-enum value surfaces error in tool output or agent retries
  - Error path: missing required field produces structured error
  - Edge case: extra fields tolerated by Rust serde deserialization (documents behavior)

  **Verification:** `cargo test -p mika-agent --test eval test_schema_divergence` green. All mock-based, no `#[ignore]`.

- [ ] **Unit 5: Multi-turn conversation persistence test (D6)**

  **Goal:** Add `run_turn()` method to `EvalHarness` for multi-turn testing. Verify conversation history carries across turns on the same session.

  **Requirements:** R7

  **Dependencies:** Unit 2 (harness changes)

  **Files:**
  - Modify: `crates/mika-agent/tests/eval/harness.rs` (add `run_turn` and `clear_and_set` integration)
  - Create: `crates/mika-agent/tests/eval/test_multi_turn_persistence.rs`
  - Modify: `crates/mika-agent/tests/eval.rs` (add module)

  **Approach:**
  - `EvalHarness::run_turn(message, responses)` — runs another turn on the same session with fresh mock responses (uses `MockLlmProvider::clear_and_set()` to replace sequence)
  - Returns new `AgentTrace` for the turn
  - Test: Turn 1 sends "Hello", Turn 2 sends "Remember what I said?" — assert Turn 2's `captured_requests` include Turn 1's messages in conversation history
  - Verify `messages` table has entries from both turns with the same `session_id`

  **Patterns to follow:**
  - `MockLlmProvider::clear_and_set()` — already exists for sequence replacement
  - `crates/mika-agent/tests/eval/trace.rs` — `AgentTrace::from_run()` DB query pattern

  **Test scenarios:**
  - Happy path: two-turn conversation — Turn 2 request contains Turn 1 history
  - Happy path: three-turn with tool call in Turn 1 — Turn 2+ sees tool call in history
  - Edge case: empty message in Turn 2 — still carries history

  **Verification:** `cargo test -p mika-agent --test eval test_multi_turn` green.

- [ ] **Unit 6: Max-steps + continuation turn test (D6)**

  **Goal:** Test that the agent correctly triggers a continuation turn when max tool steps are exceeded, and produces a summary.

  **Requirements:** R6

  **Dependencies:** Unit 5 (multi-turn method useful but not strictly required; uses `run()`)

  **Files:**
  - Create: `crates/mika-agent/tests/eval/test_max_steps_continuation.rs`
  - Modify: `crates/mika-agent/tests/eval.rs` (add module)

  **Approach:**
  - Mock 21 consecutive `ToolUse` responses (exceeds `MAX_TOOL_STEPS = 20`) followed by a text response for the continuation turn
  - Assert continuation turn fires: the final LLM call has tools disabled
  - Assert trace step count equals `MAX_TOOL_STEPS + 1` (20 tool steps + 1 continuation)
  - Assert output contains summary text from the continuation turn
  - Second test: mock continuation turn failure (mock returns error) — assert structured fallback with last 5 tool names

  **Patterns to follow:**
  - `crates/mika-agent/src/agent.rs` — `attempt_continuation_turn()`, `MAX_TOOL_STEPS`
  - `crates/mika-agent/tests/eval/test_multi_step.rs` — multi-step mock pattern

  **Test scenarios:**
  - Happy path: 20 tool calls + continuation turn → text summary output
  - Error path: 20 tool calls + continuation turn failure → structured fallback with tool names
  - Integration: continuation turn request has tools disabled (verify via `captured_requests`)
  - Edge case: step-awareness nudge injected at step 18 (`MAX_TOOL_STEPS - 2`) — verify via captured request content

  **Verification:** `cargo test -p mika-agent --test eval test_max_steps` green.

- [ ] **Unit 7: Per-skill provider override test (D5)**

  **Goal:** Prove that skill-level LLM override routes to the correct provider while the agent's other turns use the default.

  **Requirements:** R5

  **Dependencies:** Unit 2 (harness with dual-provider support)

  **Files:**
  - Create: `crates/mika-agent/tests/eval/test_per_skill_provider_override.rs`
  - Modify: `crates/mika-agent/tests/eval.rs` (add module)

  **Approach:**
  - Construct a `SkillEntry` with an LLM override (provider B)
  - Seed `skill_overrides` in the test DB with provider/model for the test skill
  - Harness uses MockLlmProvider with name "agent-mock" as default
  - The skill override mechanism should route to a different provider — but since `resolve_skill_llm_override` calls `Settings::make_provider_for()` which needs real credentials, the test may need to verify at the routing-decision level rather than full provider swap
  - Alternative: test `resolve_skill_llm_override()` directly — construct `MatchedSkill` with `MatchReason::Keyword`, verify it returns the correct override decision
  - Trace must show provider attribution per LLM call via `llm_calls` table `model` column

  **Patterns to follow:**
  - `crates/mika-agent/tests/eval/test_required_tools_gate.rs` — `SkillEntry` construction
  - `crates/mika-agent/src/agent.rs` — `resolve_skill_llm_override()` function signature
  - `crates/mika-agent/src/skills/mod.rs` — `SkillRegistry::from_test_entries()`

  **Test scenarios:**
  - Happy path: skill with override triggers correct provider routing decision
  - Happy path: agent without matched skill uses default provider
  - Edge case: `MatchReason::AlwaysOn` skill with override does NOT trigger override (Keyword filter)
  - Edge case: conflicting overrides from multiple keyword-matched skills → fallback to default with warning

  **Verification:** `cargo test -p mika-agent --test eval test_per_skill_provider` green.

- [ ] **Unit 8: Matrix runner and scenario framework**

  **Goal:** Build the matrix runner that iterates providers × scenarios, reporting per-tuple outcomes.

  **Requirements:** R2, R1

  **Dependencies:** Units 1, 2

  **Files:**
  - Create: `crates/mika-agent/tests/eval/scenarios.rs` (scenario trait/function definitions)
  - Create: `crates/mika-agent/tests/eval/test_real_provider_matrix.rs`
  - Modify: `crates/mika-agent/tests/eval.rs` (add modules)

  **Approach:**
  - `ScenarioOutcome` struct: `{ name, provider, model, success, error, response_text, token_usage, latency_ms }`
  - Scenario functions: `async fn scenario_basic_conversation(provider: &dyn LlmProvider, db: &AsyncDatabase, ...) -> Result<ScenarioOutcome>`
  - Matrix runner: `#[test] #[ignore] fn real_provider_matrix()` — parses `MIKA_EVAL_REAL_PROVIDERS`, constructs providers, iterates scenarios, reports results table
  - Per-provider convenience tests: `#[test] #[ignore] fn anthropic_matrix()`, `#[test] #[ignore] fn openai_matrix()`, etc.
  - Initial scenarios: basic conversation (text response), single tool call, multi-step chain
  - Results printed as table to stdout for human review

  **Patterns to follow:**
  - `crates/mika-agent/tests/eval/harness.rs` — `EvalHarness::builder()` pattern
  - `crates/mika-agent/tests/eval/test_basic_conversation.rs` — simple scenario structure

  **Test scenarios:**
  - Happy path: matrix runner with mock provider (for CI validation of the runner itself) produces expected outcome table
  - Integration: with `MIKA_EVAL_REAL_PROVIDERS=anthropic` + `--ignored`, Anthropic provider returns valid response
  - Edge case: provider with missing API key → skip with clear message, not failure
  - Error path: provider returns error → `ScenarioOutcome.success = false` with error details

  **Verification:** `cargo test -p mika-agent --test eval test_real_provider_matrix` passes (ignored tests skip cleanly). With env + keys + `--ignored`, matrix produces results.

- [ ] **Unit 9: Calibration mode and eval-diff**

  **Goal:** Implement calibration artifact output and diff tool for tracking provider behavior drift.

  **Requirements:** R8

  **Dependencies:** Unit 8

  **Files:**
  - Create: `crates/mika-agent/tests/eval/calibration.rs` (artifact schema, writer)
  - Create: `crates/mika-agent/tests/eval/test_calibration.rs` (test the calibration output)
  - Modify: `crates/mika-agent/tests/eval/test_real_provider_matrix.rs` (integrate calibration write)
  - Modify: `.gitignore` (add `target/eval-calibration/`)

  **Approach:**
  - `CalibrationArtifact` struct matching the JSON schema from D7 (timestamp, providers map with model + scenarios map)
  - `write_calibration(artifact, path)` → serializes to `target/eval-calibration/{timestamp}.json`
  - `diff_calibrations(old, new)` → returns list of changes (tolerated→rejected, rejected→tolerated), includes per-scenario token counts
  - Gated on `MIKA_EVAL_CALIBRATE=1` env var
  - `eval-diff` as a test helper function initially (can become binary later if needed)
  - Integration with matrix runner: when calibrate mode active, matrix writes artifact after completion

  **Patterns to follow:**
  - Standard serde JSON serialization/deserialization
  - `target/` directory for build artifacts (already gitignored at top level)

  **Test scenarios:**
  - Happy path: calibration artifact serializes/deserializes round-trip
  - Happy path: diff of two identical artifacts → no changes
  - Happy path: diff with tolerance change → reports the change with details
  - Edge case: diff with new provider added → reports addition, not failure
  - Happy path: per-scenario token counts present in diff output

  **Verification:** Calibration unit tests pass. With `MIKA_EVAL_CALIBRATE=1` + real providers, artifact file is written.

## System-Wide Impact

- **Interaction graph:** New test infrastructure only — no production code paths modified except the harness extension in `harness.rs`. `EvalHarness.run()` → `run_agent()` → provider's `send_message()`. Real-provider tests make actual HTTP calls to LLM APIs.
- **Error propagation:** Real-provider test failures surface as test failures with provider-specific error details. Mock-based test failures propagate through `anyhow::Result` as before.
- **State lifecycle risks:** Each test creates its own in-memory SQLite DB — no cross-test contamination. `TempDir` auto-cleans on drop. Real-provider tests are stateless (no persistent state across runs).
- **API surface parity:** No API changes. All providers implement `LlmProvider` trait — mock and real share the same interface.
- **Integration coverage:** The matrix runner is the primary cross-layer integration test — exercises provider construction, request building, API communication, response parsing, and trace collection end-to-end.
- **Unchanged invariants:** Existing 16 eval tests, `MockLlmProvider` interface, `AgentTrace` fields (extended but not breaking), `run_agent()` API.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Real-API tests are flaky due to provider outages | `#[ignore]` gate + per-provider skip on missing API key. Matrix reports failures without failing CI. |
| `EvalHarness` extension breaks existing tests | Unit 2 maintains full backward compatibility — `responses()` path unchanged |
| Calibration artifact schema evolves | Version field in artifact JSON; `eval-diff` handles version mismatches |
| Cost of real-API runs | Structural `#[ignore]` gate + env var. No accidental runs from `cargo test`. |
| `inject_task_id_field` dedup guard removed in future refactor | D9a frozen fixture catches this — test fails if dedup assertion is removed |

## Sources & References

- **Origin document:** [docs/plans/338-phase-2-multi-provider.md](docs/plans/338-phase-2-multi-provider.md)
- Phase 1 plan: `docs/plans/2026-03-30-001-feat-agent-eval-testing-harness-plan.md`
- Phase 1 brainstorm: `docs/brainstorms/2026-03-30-agent-eval-harness-brainstorm.md`
- Phase 1 PR: #330
- DI builders (closed): #340
- KG retrospective: `docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`
- Related issues: #339, #740, #741, #742
