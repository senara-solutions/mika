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

**Decision:** One master env `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai,kimi,groq` (comma-separated list). Empty/unset → all real-provider tests `#[ignore]`-respected. `MIKA_EVAL_REAL_PROVIDERS=all` → every configured provider runs.

**Unknown provider names hard-fail.** The parser rejects unknown names with an error listing valid providers. Silent-skip on unknowns is how CI workflows end up green while testing nothing — exactly the regression class #338 exists to catch. Explicit over implicit.

**Rationale:** A single var with a subset list beats per-provider flags (`MIKA_EVAL_ANTHROPIC=1` + `MIKA_EVAL_OPENAI=1`) because the common case is "run the matrix against the providers I have keys for today." The list form lets CI pick a subset per workflow without rewriting test lists. Parsing is ~5 lines: `env::var("MIKA_EVAL_REAL_PROVIDERS").split(',').map(parse_or_error)` — the `parse_or_error` returns `Err` on unknown names, propagated as a test-setup failure.

**Rejected alternative:** Per-provider flags. Scales linearly with provider count; already 11 providers in `ProviderKind`, picking 3 via 3 flags is ergonomically worse than one list.

### D2 — Initial provider set: four providers chosen for orthogonal validation profiles

**Problem:** 11 providers in `ProviderKind`; how many does Phase 2 cover?

**Decision:** Phase 2 ships matrix support for four — **Anthropic, OpenAI, Kimi, Groq**. Selection logic:

- **Anthropic** — native adapter path, strict JSON-schema validator, prompt-cache semantics. Non-negotiable: primary dev target.
- **OpenAI** — OpenAI-compatible adapter canonical, strict validator. Non-negotiable: baseline for adapter-path behavior.
- **Kimi** (via openrouter) — the current mika-dev runtime (per `project_mika_dev_model_switch.md`). Permissive validator; its tolerance is what masked the `task_id` incident. Including it makes "the bug Kimi hides" a matrix data point, not an exploratory encounter on the next migration.
- **Groq** — OpenAI-compatible adapter with a different server-side validation profile than both OpenAI and Kimi. Orthogonal permissive data point — different model family, different tolerance surface.

**Detection matrix shape:** two strict validators (Anthropic, OpenAI) + two permissive with distinct profiles (Kimi, Groq). Strict surfaces bugs; permissive-pair surfaces provider-specific drift without reducing to a single permissive point of failure. Adding the remaining seven providers doesn't cover new behavior classes at this layer (variations of OpenAI-compatible).

**Rationale for four not three:** The marginal cost of one more provider slot in a matrix already being built is small (~$0.10/run). The cost of picking three wrong — either excluding Kimi (bug-hiding profile un-tested) or excluding Groq (no orthogonal permissive data point) — is re-litigation in 6 weeks when a Kimi-specific incident or a Groq-tolerance drift lands in production. Four resolves the Kimi-vs-Groq tension cleanly.

**Rejected alternative:** Three providers (Anthropic, OpenAI, Groq) with Kimi added later via the documented extension path. Rejected because #338 is the middle tier of the milestone — downstream scenarios (#339, #740, #741) build on this matrix. Getting the matrix wrong here forces every downstream ticket to add Kimi individually.

**Follow-up tickets to file post-merge:** one per additional provider-calibration run if/when a specific need arises (DeepSeek, OpenRouter, Mistral, etc.); this ticket doesn't try to cover all eleven.

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

### D7 — Calibration mode: ephemeral artifact + diff CLI; committed baseline explicitly NOT in scope

**Problem:** The issue describes "calibration mode" as comparing mock expectations vs real provider behavior. How does this operationally surface, and where does the baseline live?

**Decision:** Calibration mode is a **write-mode** of the real-API test runner. When run with `MIKA_EVAL_CALIBRATE=1` alongside `MIKA_EVAL_REAL_PROVIDERS`, the test captures per-(provider, scenario) outcome into `target/eval-calibration/{timestamp}.json` (gitignored; ephemeral). The file schema:

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

A comparison command `cargo run --bin eval-diff -- {old.json} {new.json}` diffs two calibration artifacts and exits non-zero if a previously-tolerated divergence became a reject (or vice versa). **`eval-diff` output additionally includes per-scenario request token counts** so a PR reviewer adding a scenario can see when it dominates the matrix (e.g., "scenario X is 4.2× the matrix average at 47K tokens"). Observability, not enforcement — the goal is feedback-loop cost awareness for scenario authors, not a structural budget cap.

**Committed baseline explicitly deferred.** A versioned `tests/fixtures/eval-baseline.json` is the obvious next step — but committing a reference without a maintenance loop is "an artifact that lies": it looks authoritative while silently rotting the moment a provider changes tolerance and no one regenerates. Two paths:
- Ship the CI maintenance loop in #338 scope — ~40 lines of GitHub Actions YAML for a weekly calibration run that auto-opens a baseline-drift PR
- Keep #338 narrow to machinery + file a follow-up ticket for the maintenance loop

**This ticket takes the second path.** Follow-up tracked as **mika#742** (weekly calibration CI + drift-detection PR automation). Until #742 merges, calibration artifacts stay ephemeral (uploaded to CI run as workflow artifacts, not committed). Downstream scenarios (#339/#740/#741) that want regression gating on provider behavior can cite #742 as their gate blocker.

**Rationale:** Machinery ships now, trust-the-baseline ships separately. Preserves the #338 scope boundary ("ships the matrix runner + divergence harness; does NOT ship the scenario catalog or the trust infrastructure"). Friend review explicitly flagged committed-but-unmaintained as the worst of both worlds.

**Rejected alternatives:** (a) Commit the baseline with no maintenance loop — rejected as theater per friend review. (b) Ship the maintenance CI job in #338 scope — rejected to preserve scope narrowness; the job is meaningful enough work to warrant its own ticket (#742).

### D8 — Cost controls: structural intent gates only, no plan-level dollar estimate

**Problem:** Real-API runs cost money. How do we prevent accidental $50 runs from a developer laptop?

**Decision:** Two structural layers. No runtime budget enforcement. No plan-level dollar estimate.

- **Structural intent gate (Layer 1):** Every real-API test carries `#[ignore]`. The env var (`MIKA_EVAL_REAL_PROVIDERS`) alone does not trigger real API calls — a test invocation must explicitly pass `--ignored` (or `--include-ignored`). Accidentally running `cargo test` on a laptop with the env set still doesn't burn budget. This is the same principle as `feedback_prompt_enforcement_fragile.md`: structural > prompt-level. Cost protection is intent-based, not token-based.
- **Structural env gate (Layer 2):** `MIKA_EVAL_REAL_PROVIDERS` controls which providers actually attempt real calls. Tests for unconfigured providers skip with a clear message.

**No plan-level cost estimate.** A stated "$0.40-0.65/run" range in a plan doc is a commitment to accuracy the doc can't keep. Per-provider per-scenario pricing rots (Kimi via openrouter has non-obvious multi-layer billing; Anthropic output-token pricing shifts; scenarios can grow unexpectedly). A number that rots is doc-level cost enforcement — exactly the class of control rejected elsewhere via `feedback_prompt_enforcement_fragile.md`. Instead: **matrix cost is workflow-controlled via job timeout and scenario-count caps set in `.github/workflows/eval-matrix.yml`. Actual spend is observable on the CI run. No plan-level estimate.**

Observability for cost awareness lives in D7 (per-scenario token counts in `eval-diff` output), so PR reviewers see when a new scenario dominates the matrix.

**Rationale:** Cost enforcement via runtime token-counting is premature optimization for a risk that hasn't manifested. Intent-based structural gating (`#[ignore]` + env) makes accidental burn impossible without explicit operator opt-in. Dollar estimates in plan docs age into lies and teach operators to distrust plan-level guidance.

**Rejected alternatives:**
- In-test token accumulator with hard cap. Adds plumbing and changes test failure semantics (cost-exceeded ≠ test-failed).
- Per-provider cost table in `crates/mika-agent/CLAUDE.md`. Same rot problem as the dollar estimate; maintenance burden without structural value.
- Per-scenario max-tokens cap at the harness level (`EvalScenario.max_request_tokens`). Not YAGNI-worthy for #338; revisit as a follow-up if D7 observability surfaces actual budget blowouts.

### D9 — Request-side JSON-schema well-formedness assertions (the layer that catches `task_id`)

**Problem:** The `task_id` incident happened at the **request-construction** layer (`inject_task_id_field`). The damage manifested at the provider-validation layer. Mika's handling of the provider rejection was fine. A harness-level test of Mika's handling would NOT have caught `task_id` — because the handling wasn't the bug.

D4 as originally drafted covers two classes: Mika's handling of rejection (mock-based) and provider-tolerance drift (real-API calibration). It does not cover: **"our request is well-formed before it leaves the process."** That's the layer where `task_id` lived for six weeks.

**Decision:** Add request-side well-formedness assertions to the harness. Deterministic, fast, no API calls. Cover:

- **Required-array deduplication:** `assert_eq!(dedup(required), required)` on every tool definition emitted by the request builder. Would have caught `task_id`.
- **Schema-properties / required coherence:** every entry in `required` exists as a key in `properties`.
- **Enum membership:** each property declaring an `enum` has `len > 0` and no duplicates.
- **Reserved-name shadowing:** tool names don't collide with reserved builtins (e.g., `run_agent`).

Tests live in a new `test_request_wellformedness.rs` — one test per rule, exercised against `default_tools()` and a synthetic per-skill tool set. Runs in CI on every push (no `#[ignore]`, no env gate, no real API).

**Rationale:** Three layers, three bug classes:
1. **D9 (new, harness):** our request is well-formed. Catches `task_id`-class bugs.
2. **D4 part 1 (harness):** we handle provider rejection correctly. Catches dispatch-path bugs.
3. **D4 part 2 (real-API, opt-in):** provider tolerance drifts over time. Catches silent provider-side changes.

Collapsing any two of these into one misses a bug class. Friend review was explicit: *"A harness-level test of 'our request builder preserves dups as-is' would have passed on every provider switch; the actual defect was in `inject_task_id_field`."*

**Rejected alternative:** Add well-formedness assertions to `inject_task_id_field` itself (in-code invariant check via `debug_assert!`). Rejected because the point of eval harness tests is they run in CI on release builds, not only in debug. Also: other request-builders besides `inject_task_id_field` produce tool schemas — the harness test covers all of them uniformly.

## Acceptance Criteria

- [ ] D1: `MIKA_EVAL_REAL_PROVIDERS` env gate implemented; parses comma-separated list or `all`; **unknown provider names hard-fail with a listing of valid names**; empty/unset → all real-provider tests respect `#[ignore]`.
- [ ] D2: Matrix runs Anthropic + OpenAI + Kimi + Groq; documentation includes "how to add a provider" section explaining the three-step extension.
- [ ] D3: Scenarios as `async fn`, invoked from per-provider tests AND matrix runner. Both paths present and tested.
- [ ] D4: Harness tests (mock-based) for 4 schema divergence classes on the response-handling layer. Separate real-API calibration test `#[ignore]`-gated.
- [ ] D5: `test_per_skill_provider_override.rs` exists, uses two mock providers with distinct names, trace asserts provider attribution per call.
- [ ] D6: Three new test files — max-steps continuation, required-tools gate, multi-turn persistence — all mock-based.
- [ ] D7: Calibration artifact schema stable (versioned); `eval-diff` binary present and tested against fixture artifacts. Baseline is **ephemeral** (gitignored in `target/eval-calibration/`); committed baseline deferred to **mika#742**. **`eval-diff` output includes per-scenario request token counts** flagging scenarios that dominate the matrix (observability-only; no enforcement).
- [ ] D8: **Every real-API test carries `#[ignore]`** — intent-based structural gate. No plan-level cost estimate; workflow-level controls only (job timeout + scenario caps in `.github/workflows/eval-matrix.yml`). Documentation references workflow configuration, not dollar figures.
- [ ] **D9a — Frozen regression fixture:** `test_request_wellformedness.rs` includes a committed fixture that reproduces the pre-fix `task_id` schema output — the exact duplicate-entry-in-`required` shape from `inject_task_id_field` at commit `440a9d59`. Test MUST fail against the fixture when the dedup assertion is removed; MUST pass with it. Fixture is frozen (checked-in JSON), not regenerated from current code — so a future refactor of `inject_task_id_field` that narrows coverage does not silently retain a green test.
- [ ] **D9b — Emitter code-path enumeration:** Assertions run against schemas produced by *every* code path that emits tool schemas to the provider. At minimum: `default_tools()`, skill-injected tools via `inject_task_id_field`, MCP-exposed tools via `McpManager`. Test module enumerates covered emitters explicitly in a doc comment; adding a new emitter path elsewhere in the codebase without adding it here is a review-blocking oversight.
- [ ] D9c — Additional harness-level well-formedness rules: schema-properties/required coherence (`required` entries exist in `properties`), enum non-emptiness + no-duplicates, reserved-name shadowing (tool names don't collide with builtins like `run_agent`).
- [ ] Existing 13 Phase 1 tests still pass.
- [ ] `cargo test -p mika-agent --test eval` green (mock + D9 subsets). Real-API tests pass when invoked with `--ignored` + env + keys.
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

## Review log

**Vincent review + friend feedback (2026-04-22, relayed by Vincent):**

- **D1:** Comma-separated list confirmed. Added hard-fail on unknown provider names — silent-skip is how CI stays green while testing nothing.
- **D2:** Expanded from three providers to **four** — Anthropic / OpenAI / Kimi / Groq. Friend's detection argument: two strict + two orthogonal permissive profiles is the right regression-detection matrix. Kimi included because it's the current mika-dev runtime and its tolerance is what masked the `task_id` incident; Groq included for an orthogonal permissive data point. Tradeoff: ~$0.10/run extra cost vs the re-litigation cost of excluding the wrong one in 6 weeks.
- **D7:** Committed baseline explicitly deferred. Shipping committed-but-unmaintained was called out as theater. #338 ships ephemeral; maintenance loop tracked as **mika#742** (weekly calibration CI + drift-PR automation). Downstream scenarios citing regression gating must block on #742.
- **D8:** Added **structural intent gate** via `#[ignore]` on every real-API test. Matches `feedback_prompt_enforcement_fragile.md` — structural > documentation for intent protection. Cost baseline updated to four-provider estimate ($0.40-0.65/run).
- **D9 (new):** Request-side JSON-schema well-formedness assertions. Friend's sharpest push: the `task_id` bug lived at request construction; no response-handling test would have caught it. Three layers now (D9 well-formedness / D4.1 mock rejection handling / D4.2 real-API drift) — each catches a different bug class; collapsing any two misses a class.

**Friend principle adopted across amendments:** "what would have actually caught the motivating incident" > "what seems reasonable to test." D9 is the direct application.

**Second friend review pass (2026-04-22, relayed by Vincent):**

- **D8 cost estimate dropped.** A doc-stated "$0.40-0.65/run" is a rotting commitment to accuracy. Replaced with workflow-controlled language referencing `.github/workflows/eval-matrix.yml` — numbers live where rot is expected (CI logs, PR descriptions), not in plan docs that future tickets cite.
- **D9 AC split into D9a (frozen regression fixture) + D9b (emitter enumeration) + D9c (remaining rules).** Sharpest change: "would have caught `task_id`" is a claim; a committed fixture reproducing the pre-fix schema that fails without the assertion is a structural proof. Same structural-over-prompt principle driving the rest of the milestone. Bonus: the fixture becomes the natural landing place for future schema-construction bug regressions — D9 is a fixture catalog, not a single assertion.
- **D7 gets per-scenario token counts in `eval-diff` output.** Observability-only. Feedback loop for scenario authors via PR logs, not a structural cap. Per-scenario `max_request_tokens` guardrail explicitly deferred as a follow-up (file only if D7 observability shows authors actually blowing budget).
- **Edge direction sanity-checked:** `#742 blockedBy #338` confirmed via GraphQL query (`#338.blockedBy` returns `#340` only; `#742.blockedBy` returns `#338`). Mutation went the intended direction.
