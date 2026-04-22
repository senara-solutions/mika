# Plan — mika#338 — Eval harness Phase 2: multi-provider matrix + real API calibration

**Issue:** senara-solutions/mika#338
**Branch:** `feat/338/phase-2-multi-provider`
**Milestone:** Evaluation (#16)
**Blocked by:** #340 (DI builders needed to inject real providers via the harness)
**Status:** Groomed draft — pending Vincent review

## Context

Phase 1 (#329 / PR #330) proved the harness works with mocks. Phase 2 extends it to **real** providers — surfacing behavioral divergences that mocks can't show. The motivating incident is the `task_id` dedup bug (latent since commit `440a9d59` on Mar 11, `crates/mika-agent/src/skills/index.rs::inject_task_id_field`): kimi-k2.5 tolerated the duplicate entry in the JSON-schema `required` array; Anthropic and DeepSeek rejected on provider switch. A provider swap exposed a weeks-old bug. This ticket's job is to make provider swaps a tested event, not an adventure.

#338 is the **middle tier** of milestone #16's DAG — #340 (builders) unblocks it; #339 (golden dataset), #740 (KG scenarios), #741 (grounding regressions) all need the real-API machinery shipped here.

## Scope boundary

This ticket ships the **machinery**: matrix runner, real-provider gating, per-skill override injection, schema-divergence fixtures, calibration mode plumbing. It does NOT ship the full scenario catalog — that's #339's job. Keep #338 narrow or it will eat the whole milestone.

Inside scope: matrix runner, env gating, JSON-schema divergence tests, per-skill override test, max-steps continuation test, required-tools gate test, multi-turn persistence test, calibration harness stub.

Out of scope: golden dataset curation (→ #339), KG-specific scenarios (→ #740), grounding/fabrication scenarios (→ #741), Langfuse integration, dashboard.

## Decisions

### D1 — Real-provider gate: single env with provider subset

**Problem:** How do tests opt into real API calls?

**Decision:** One master env `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai,groq` (comma-separated list). Empty/unset → all real-provider tests `#[ignore]`-respected. `MIKA_EVAL_REAL_PROVIDERS=all` → every configured provider runs.

**Rationale:** A single var with a subset list beats per-provider flags (`MIKA_EVAL_ANTHROPIC=1` + `MIKA_EVAL_OPENAI=1`) because the common case is "run the matrix against the providers I have keys for today." The list form lets CI pick a subset per workflow without rewriting test lists. Parsing is one line: `env::var("MIKA_EVAL_REAL_PROVIDERS").split(',').filter_map(...)`.

**Rejected alternative:** Per-provider flags. Scales linearly with provider count; already 11 providers in `ProviderKind`, picking 3 via 3 flags is ergonomically worse than one list.

### D2 — Initial provider set: three, document the next two

**Problem:** 11 providers in `ProviderKind`; how many does Phase 2 cover?

**Decision:** Phase 2 ships matrix support for three — **Anthropic, OpenAI, Groq**. These cover: Anthropic prompt-cache path + strict JSON schema; OpenAI-compatible adapter path + different schema tolerance; Groq + speed-tier characteristics. Document that adding a fourth (Kimi, DeepSeek, OpenRouter) requires: (a) adding the provider to the matrix parameter table, (b) verifying its `create_provider` path, (c) one provider-calibration scenario. No code change needed per provider beyond config.

**Rationale:** Three providers cover the two adapter paths (native Anthropic + OpenAI-compatible) and one speed-tier data point. Adding the remaining eight adds CI cost linearly without covering new behavior classes at this layer (they're variations of the OpenAI-compatible adapter).

**Follow-up tickets to file post-merge:** one per additional provider-calibration run, each tracked separately; this ticket doesn't try to cover all eleven.

### D3 — Matrix shape: scenarios as functions, providers as runtime parameter

**Problem:** How do tests parameterize over providers? Test-per-tuple explodes; single runner loses `#[test]` discoverability.

**Decision:** Each scenario is a function `async fn scenario_X(provider: &dyn LlmProvider, ...) -> Result<ScenarioOutcome>`. A **matrix runner** test (`#[test]` gated on `MIKA_EVAL_REAL_PROVIDERS`) iterates configured providers × scenarios and reports per-(provider, scenario) outcome. Individual provider tests (`#[test] #[ignore] fn anthropic_matrix()`, etc.) exist for targeted runs. Same scenario functions, invoked from different entry points.

**Rationale:** Keeps scenarios authored once. Lets `cargo test anthropic` target a single provider without re-implementing anything. The matrix runner is the catch-all; individual `#[test]`s preserve `cargo test --list` discoverability.

**Tradeoff accepted:** Two places define the test entry point (per-provider fn + matrix fn). Less DRY but more findable — per-provider names show up in test output without post-processing.

**Rejected alternative:** `#[rstest]` / `#[test_case]` macros. Adds a dep, obscures failure attribution in test output, and conflicts with the `#[ignore]` gating story (macro-generated tests are awkward to gate).

### D4 — JSON-schema divergence: ship as harness tests with constructed tool schemas, NOT as real API assertions

**Problem:** Four divergence classes to test (required-array dedup, enum exhaustiveness, unknown `additionalProperties`, missing required field). Testing against real APIs means every CI run pays for them; testing against real APIs also means a provider's internal tolerance change (quietly) can break your test without any Mika code change.

**Decision:** Each divergence class gets a **harness-level test** that constructs a malformed `ToolDefinition`, invokes the provider's own request builder (`to_anthropic_request` for Anthropic, `to_openai_compatible_request` for the OpenAI adapter), and asserts serialization behavior. Cover:

- **Required-array dedup:** build a schema with `["task_id", "task_id"]` in `required`; assert per-provider serializer behavior (Anthropic request builder preserves dups as-is; OpenAI-compatible too; the provider API is what rejects). Document expected rejection behavior at the request-builder layer as a per-provider attribute test.
- **Enum exhaustiveness:** LLM emits an enum value not in schema — test the response parser, not the request. Inject a response with out-of-enum value via `MockLlmProvider`, verify downstream tool-call dispatch surfaces structured error (not silent accept).
- **Unknown fields (`additionalProperties`):** same as enum — response-side, mock an out-of-schema field; verify parse path.
- **Missing required:** mock response omits a declared-required field; verify retry path fires or structured error surfaces.

A **separate opt-in** real-API test (`#[test] #[ignore] fn real_api_schema_divergence()`) runs the request against each configured provider's endpoint with `MIKA_EVAL_REAL_PROVIDERS` set, captures the provider's actual reject/tolerate behavior, and records it to a **calibration artifact** (D7). This run is on-demand, not on every CI push.

**Rationale:** The value the tests need to deliver is "Mika correctly handles provider rejection." That's a Mika code path, testable with mocks. Real-API probing is a secondary calibration activity — valuable but not gatekeeping.

**Rejected alternative:** Run each divergence test against real APIs on every gated CI run. Triples cost without new coverage; also flakes on provider outages.

### D5 — Per-skill provider override: one test, two provider instances, same session

**Problem:** Skills can declare `[llm] provider = "openai"` via the DB `skill_overrides` table (legacy — moved from `skill.toml` per mika#504). The test needs to prove: agent uses provider A by default; when a matched skill has an override to provider B, the skill's LLM interaction uses B; the agent's other turns still use A.

**Decision:** One test `test_per_skill_provider_override.rs` — harness configures agent with `MockLlmProvider(name="agent-mock")` AND injects a second mock for a specific skill via the skill override mechanism. Seed DB with `skill_overrides` row pointing the test skill to `skill-mock`. Turn 1 (agent-only) asserts `agent-mock` was called. Turn 2 (triggers the override skill) asserts `skill-mock` was called. Trace must show provider/model attribution per LLM call.

**Rationale:** Two mocks with distinct names is the simplest test of the routing mechanism. No real APIs needed — this is about the agent loop's wiring, not about provider behavior.

**Dependency:** `MockLlmProvider::builder().provider_name(...)` already exists (Phase 1). No new harness surface needed.

### D6 — Additional scenarios: three tests, all mock-based

**Problem:** The issue lists three additional scenarios beyond the matrix: max-steps + continuation, required-tools gate, multi-turn conversation.

**Decision:** Three separate test files, all mock-based (no real API):

- `test_max_steps_continuation.rs`: mock 21 consecutive `ToolUse` responses; assert continuation turn fires with tool-disabled mode and summary text.
- `test_required_tools_gate.rs`: build a skill with `required_tools = ["X"]`, seed match, make the LLM EndTurn without calling X; assert gate rejects once; second response calls X; assert accept.
- `test_multi_turn_persistence.rs`: two sequential `harness.run("msg1")` / `harness.run("msg2")` calls on the same session; assert conversation history carries correctly.

**Rationale:** These test engine invariants, not provider behavior. Mocks are the right tool. Separating into three files keeps each under 150 lines, which is what the existing test files target.

**Cross-reference:** `test_max_steps_continuation.rs` exercises the same path as `attempt_continuation_turn()` helper; its existing coverage in `test_required_tools_gate.rs` is the eval-harness-level assertion that the engine's own tests don't provide.

### D7 — Calibration mode: JSON artifact, not a live dashboard

**Problem:** The issue describes "calibration mode" as comparing mock expectations vs real provider behavior. How does this operationally surface?

**Decision:** Calibration mode is a **write-mode** of the real-API test runner. When run with `MIKA_EVAL_CALIBRATE=1` alongside `MIKA_EVAL_REAL_PROVIDERS`, the test captures per-(provider, scenario) outcome into `target/eval-calibration/{timestamp}.json`. The file schema:

```json
{
  "timestamp": "2026-04-22T...",
  "providers": {
    "anthropic": {
      "model": "claude-sonnet-4-6",
      "scenarios": {
        "required_array_dedup": { "outcome": "rejected", "error_class": "schema_validation" },
        "enum_exhaustiveness": { "outcome": "tolerated", "error_class": null }
      }
    }
  }
}
```

Separately, a comparison command `cargo run --bin eval-diff -- {old.json} {new.json}` diffs two calibration artifacts and exits non-zero if a previously-tolerated divergence became a reject (or vice versa). This is the "provider drift" detection the issue asks for, delivered as a diff tool not a dashboard.

**Rationale:** File + diff tool is the simplest thing that could work. Langfuse / dashboard integration can come later (explicitly out of scope); file-based drift detection covers the weekly-regression use case.

**Rejected alternative:** Write to a DB table (complex migration, not needed), emit to stdout as a table (loses the comparison capability without manual parsing).

### D8 — Cost controls: documented budget, no runtime enforcement

**Problem:** Real-API runs cost money. How do we prevent accidental $50 runs?

**Decision:** Document estimated cost per run in the test module doc comment and in `crates/mika-agent/CLAUDE.md` eval section. No runtime enforcement (no token counting + early-exit). CI workflows that invoke real-API tests explicitly opt in via the `MIKA_EVAL_REAL_PROVIDERS` env and can set their own per-run budgets via workflow timeout.

Baseline estimate to include in docs: Phase 2 full matrix against Anthropic+OpenAI+Groq ≈ $0.30-0.50 per run (3 providers × ~5 scenarios × ~$0.02-0.03/scenario). Calibration mode doubles this if it also runs divergence probes.

**Rationale:** Runtime budget enforcement is a YAGNI for a test suite operators run intentionally. CI cost controls live at the workflow layer.

**Rejected alternative:** In-test token accumulator with hard cap. Adds plumbing for a risk that hasn't manifested.

## Acceptance Criteria

- [ ] D1: `MIKA_EVAL_REAL_PROVIDERS` env gate implemented; parses comma-separated list or `all`; empty/unset → all real-provider tests respect `#[ignore]`.
- [ ] D2: Matrix runs Anthropic + OpenAI + Groq; documentation includes "how to add a provider" section.
- [ ] D3: Scenarios as `async fn`, invoked from per-provider tests AND matrix runner. Both paths present and tested.
- [ ] D4: Harness tests (mock-based) for 4 schema divergence classes. Separate real-API calibration test `#[ignore]`-gated.
- [ ] D5: `test_per_skill_provider_override.rs` exists, uses two mock providers with distinct names, trace asserts provider attribution per call.
- [ ] D6: Three new test files — max-steps continuation, required-tools gate, multi-turn persistence — all mock-based.
- [ ] D7: Calibration artifact schema stable (versioned); `eval-diff` binary present and tested against fixture artifacts.
- [ ] D8: Cost baseline documented in `crates/mika-agent/CLAUDE.md` eval section and test module doc.
- [ ] Existing 13 Phase 1 tests still pass.
- [ ] `cargo test -p mika-agent --test eval` green (mock subset). Real-API tests pass when invoked with env + keys.
- [ ] `cargo clippy` clean.

## Dependencies

- Blocked by #340 — needs `EvalHarnessBuilder` DI builders (D1 of #340) to inject real providers. Specifically: the matrix runner constructs providers via `create_provider()` and passes them to the harness via a new `.llm_provider(Arc<dyn LlmProvider>)` builder that swaps `MockLlmProvider` for the real thing. This requires #340's DI surface to be final (the PR may need a small extension for real-provider injection; if so, fold into the same D1 work).

## Downstream (unblocked by this ticket)

- #339 — uses matrix runner for golden dataset scenarios
- #740 — uses matrix runner + DI for KG Stage 2 LLM disambiguation tests
- #741 — uses matrix runner + `health_error` (from #340 D5) for fabrication scenarios

## Cross-cutting notes

- Re-reads the KG-milestone lesson "a provider migration is a contract-validation event" (retrospective §"Track LLM provider strictness differences") — D4 operationalizes that principle.
- Matrix runner is not a new test runner — it's a `#[test]` function that iterates. Keeps `cargo test` the sole entry point.
- Calibration artifacts live in `target/eval-calibration/` (gitignored). The diff tool emits pass/fail based on semantic change, not byte-level diff.

## Open questions (for Vincent before dispatch)

1. **D1 env format:** comma-separated list good, or prefer `MIKA_EVAL_REAL_PROVIDERS=anthropic openai groq` (space-separated)? Shell escaping / CI workflow ergonomics.
2. **D2 initial set:** confirm Anthropic/OpenAI/Groq is the right three. Alternative: Anthropic/OpenAI/Kimi (since Kimi is the current mika-dev runtime provider — highest practical value for calibration).
3. **D7 calibration artifact format:** JSON file in `target/` works but is ephemeral across CI runs. Worth checking in a canonical calibration under `tests/fixtures/eval-baseline.json` so `eval-diff` has a stable baseline to compare against?
4. **D8 cost controls:** acceptable to have zero runtime enforcement, or want a soft warn (stderr log at 80% of estimated budget)?
