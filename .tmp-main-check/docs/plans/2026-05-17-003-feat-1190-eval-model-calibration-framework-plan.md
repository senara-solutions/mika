---
ticket: senara-solutions/mika#1190
title: Model calibration framework — fixed scenario suite + pass-rate gate for every model swap
type: feat
branch: feat/1190/eval-model-calibration-framework-fixed
labels: enhancement, p1-important, agent-core
created: 2026-05-17
---

# Plan — mika#1190: Model calibration framework

## 1. Context

`project_model_calibration` (memory, 2026-04-23) established the principle but the framework was never built. The 2026-05-07 kimi → sonnet swap for mika-dev landed without a calibration gate and produced three downstream incidents in the past 10 days:

- **mika#1168** — sonnet's "Prompt injection. Rejected." pattern silently dropped autonomous dispatches (two confirmed lost dispatches on 2026-05-17 alone).
- **mika#1166** — dev-groom skill exits in 3ms without invoking `/ce:plan`; autonomous grooming dispatch broken.
- **mika#1173** — dev-groom prompt-only design reverted after 5+ regressions since #934.

The framework's value proposition is concrete: had these scenarios been encoded in a pre-swap calibration suite, the 2026-05-07 swap would have been blocked. The same suite catches future swaps (grok-4, gpt-5, sonnet-4.7, opus-4.7) by construction.

## 2. Existing infrastructure inventory (load-bearing for this plan)

A non-trivial amount of the "framework" already exists. Reading these files is a prerequisite to implementation; the plan layers ON TOP of them, it does not replace them.

| File / module | What it does | Reuse posture |
|---|---|---|
| `crates/mika-agent/tests/eval/calibration.rs` | `CalibrationArtifact` JSON schema, `ScenarioCalibration` per-scenario record, diff tool for drift detection (#742) | **Reuse verbatim** — the artifact schema is the report schema |
| `crates/mika-agent/tests/eval/scenarios.rs` | `ScenarioOutcome`, `Scenario`, `ScenarioRegistry::default_scenarios()` (2 basic provider-level scenarios) | **Extend** — the abstraction shape is right; new role-scoped scenarios register here |
| `crates/mika-agent/tests/eval/providers.rs` | `parse_real_providers()` env gate, `create_real_provider(kind)`, supports `MIKA_EVAL_REAL_PROVIDERS=all` plus comma lists | **Reuse verbatim** |
| `crates/mika-agent/tests/eval/harness.rs` | `EvalHarness` — full agent loop with `MockLlmProvider` OR real provider via `with_real_provider()` | **Reuse verbatim** — this is THE harness for role-scoped scenarios |
| `crates/mika-agent/tests/eval/test_real_provider_matrix.rs` | Matrix runner: providers × scenarios, formatted table, `#[ignore]` gated | **Extend** — adds a role dimension |
| `crates/mika-agent/tests/eval/golden/` | 25 curated scenarios + hard-assertions + LLM-judge for quality (#339); judge pinned to `claude-sonnet-4-6` | **Reuse pattern** — golden's hard-assertion shape is the scoring contract |
| `crates/mika-agent/tests/eval/grounding_regressions/` | 31 fabrication-detection scenarios + frozen pre-fix fixtures (#741, #862, #863, #864, #890, #894, #901, #1059) | **Reuse pattern** — failure-class taxonomy already lives here |
| `crates/mika-agent/tests/eval/kg_provider_eval/` | Per-task provider comparison (#762), per-provider quality/cost/latency tables | **Mirror structure** — closest analog for a role-scoped suite |
| `crates/mika-agent/tests/eval/skills/mika_arch_groom_milestone.rs` | Per-skill output-contract eval through the agent loop with synthetic skills (#879) | **Reuse pattern** — this IS a role-scoped scenario in current form |
| `crates/mika-agent/CLAUDE.md § Evaluation` | Three-tier execution model (D6): Unit / Integration / Calibration via `MIKA_EVAL_CALIBRATE=1` | **Reuse contract** — Calibration tier is exactly the gate this ticket asks for |

The genuine gaps this ticket fills:

1. **Role-scoped scenarios** — existing scenarios are provider-level (`basic_conversation`, `multi_turn_greeting`) or task-level (KG extraction). There is no `mika-dev` role-scoped suite.
2. **CLI shape** — the existing path is `MIKA_EVAL_REAL_PROVIDERS=... cargo test -p mika-agent --test eval ... -- --ignored --nocapture`. Operator-friendly `make calibrate-<role> MODEL=<id>` does not exist.
3. **Committed baseline** — `calibration.rs` writes EPHEMERAL artifacts to `target/eval-calibration/`. There is no committed baseline at `docs/eval/calibration/baselines/<date>.md` that diffs anchor against.
4. **Pre-swap discipline encoded** — CLAUDE.md does not block model swaps on a passing calibration run.
5. **Failure-mode aggregation** — `grounding_regressions/` detects fabrications class-by-class; refusals, scope creep, hallucinated tools are not aggregated into a single report taxonomy.

## Phase 0 Pin — base SHA and verbatim slices (architect F1+F2)

**Base SHA**: `72021b78` (`origin/main` HEAD at plan time, 2026-05-17). Title: `chore(dev-groom): revert prompt-only design — restore deterministic tool+handler (mika#1173) (#1187)`.

The four load-bearing module-promotion / pattern-lift targets are pinned verbatim below. If any of these slices drift between plan time and implementation time, halt and re-pin before the first commit on `feat/1190/eval-model-calibration-framework-fixed`.

### Slice 1 — `CalibrationArtifact` schema (DR-3 promotion target)

`crates/mika-agent/tests/eval/calibration.rs` § lines 13–24, 27–34, 37–50:

```rust
/// Top-level calibration artifact, written as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationArtifact {
    /// Schema version for forward compatibility.
    pub version: u32,
    /// ISO 8601 timestamp of when the calibration was run.
    pub timestamp: String,
    /// Per-provider results.
    pub providers: BTreeMap<String, ProviderCalibration>,
}

/// Per-provider calibration data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCalibration {
    /// Model used for this provider's run.
    pub model: String,
    /// Per-scenario results.
    pub scenarios: BTreeMap<String, ScenarioCalibration>,
}

/// Per-scenario calibration data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioCalibration {
    /// Whether the scenario succeeded.
    pub outcome: String, // "pass", "fail", "error"
    /// Error classification if applicable.
    pub error_class: Option<String>,
    /// Input token count.
    pub input_tokens: Option<u64>,
    /// Output token count.
    pub output_tokens: Option<u64>,
    /// Wall-clock latency in milliseconds.
    pub latency_ms: u64,
}
```

DR-3 commits to "no schema change" — the promoted `src/calibration/artifact.rs` MUST export these three structs with identical field names, types, and `#[derive]` set. The `pub struct CalibrationDiff` and `pub struct DiffResult` types (lines 51–66) also move unchanged.

### Slice 2 — `Scenario` struct + `ScenarioRegistry` (DR-3 + DR-4 promotion target)

`crates/mika-agent/tests/eval/scenarios.rs` § lines 38–47:

```rust
/// Registry of all available scenarios.
pub struct ScenarioRegistry {
    pub scenarios: Vec<Scenario>,
}

/// A boxed async future returning a `ScenarioOutcome`.
type ScenarioFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ScenarioOutcome> + Send>>;

pub struct Scenario {
    pub name: &'static str,
    pub description: &'static str,
    pub run: fn(Arc<dyn LlmProvider>) -> ScenarioFuture,
}
```

DR-4's `RoleScenario` adopts this shape: `fn run` field over a `pin<Box<dyn Future>>`. The provider-level `Scenario` stays in `tests/eval/scenarios.rs` (used by existing matrix); `RoleScenario` is the role-scoped sibling that takes a `&EvalHarness` builder closure in addition to a provider.

### Slice 3 — `mika_arch_groom_milestone.rs` test entry (DR-4 pattern lift)

`crates/mika-agent/tests/eval/skills/mika_arch_groom_milestone.rs` § lines 118–145:

```rust
#[tokio::test]
async fn test_three_sub_issue_brief_emits_ready_with_scope_milestone() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_milestone_skill()]);

    let mock_response = "\
Per-sub-issue disposition summary:
…
Scope: milestone
Disposition: READY";

    let harness = EvalHarness::builder()
        .responses(vec![text_response(mock_response)])
        .skills(skills)
```

DR-4's `Role::mika_arch()` lifts this pattern (synthetic skill via `make_milestone_skill()`-style constructors + `EvalHarness::builder().skills(...)`), generalized to swap `.responses(...)` for `.with_real_provider(provider)` when invoked from the calibration runner.

### Slice 4 — `grounding_assertions` API (DR-5 scoring dependency)

`crates/mika-agent/tests/eval/grounding_assertions/mod.rs` § function signatures:

```rust
pub fn assert_response_forbids(trace: &AgentTrace, forbidden: &[&str])
pub fn assert_tool_called_before_response(trace: &AgentTrace, tool_name: &str)
pub fn assert_any_tool_called_from(trace: &AgentTrace, tool_names: &[&str])
```

DR-5's structural scoring composes these — for mika-dev's no-refusal check, `assert_response_forbids(&trace, &["Prompt injection. Rejected.", "I can't help with that"])`; for mika-arch's contract scoring, `assert_any_tool_called_from(&trace, &["read_file", "fetch"])` to enforce citation-or-silence.

### F2 — Compile-outside-test verification (DR-3 path-only commit verdict)

Verified 2026-05-17 against base SHA `72021b78`:

| Module | `#[cfg(test)]` blocks | Test-only deps in main body | Decision |
|---|---|---|---|
| `tests/eval/calibration.rs` | One inline `mod tests` at L227 (internal unit tests for `from_outcomes`, `diff_calibrations`) | None — main body imports `std::collections::BTreeMap`, `std::path::{Path, PathBuf}`, `serde::{Deserialize, Serialize}`, `super::scenarios::ScenarioOutcome` | **Path-only safe** |
| `tests/eval/scenarios.rs` | None | None — main body imports `std::sync::Arc`, `std::time::Instant`, `mika_common::llm::*`, `serde` | **Path-only safe** |
| `tests/eval/providers.rs` | One inline `mod tests` at L92 (internal unit tests for `parse_provider_list`) | None — main body imports `std::str::FromStr`, `std::sync::Arc`, `mika_common::llm::{LlmProvider, ModelSpec, ProviderKind, create_provider}` | **Path-only safe** |

The `#[cfg(test)]` blocks contain *internal unit tests* of the modules themselves, which continue to compile and run identically after promotion to `src/calibration/`. There are NO test-only crate imports (`mockall`, `tempfile`, `proptest`, etc.) in any main body. `super::scenarios::ScenarioOutcome` in calibration.rs becomes `super::scenario::ScenarioOutcome` after promotion — both files move together as siblings in `src/calibration/`, so the relative path resolves identically.

**Verdict on DR-3 path choice**: **path-only same-PR isolated commit** (Unit 1 in §6 below). No precursor PR needed.

## 3. Goals (v1 — what this ticket ships)

1. **Two role-scoped scenario suites** with 5-7 hand-coded scenarios each:
   - `mika-dev` — anchored on mika#1168 (refusal regression), mika#1166 (dev-groom skill route), mika#1173 (contract regression) plus 2 golden-path scenarios
   - `mika-arch` — groom-ticket + groom-milestone contract scenarios anchored on existing `mika_arch_groom_milestone.rs` pattern + citation discipline + Disposition-keyword discipline
2. **`calibrate` CLI** at `crates/mika-agent/src/bin/calibrate.rs` accepting `--role <name>` `--model <provider/model>` `--baseline <path>` flags; emits JSON artifact + markdown report
3. **`make calibrate-mika-dev MODEL=...` and `make calibrate-mika-arch MODEL=...` targets** that invoke the binary with appropriate env propagation
4. **Initial baseline** at `docs/eval/calibration/baselines/2026-05-17.md` capturing current active models (`anthropic/claude-sonnet-4-6` for mika-dev; `anthropic/claude-opus-4-6` for mika-arch) — pass-rate, cost, latency, failure modes
5. **Pre-swap discipline in CLAUDE.md** — explicit prohibition: no `mika_dev` / `mika_arch` model/provider swap merged without an attached `make calibrate-<role>` report ≥ baseline
6. **At least one A/B committed** — `mika-arch` on `claude-sonnet-4-6` vs `claude-opus-4-6` (current), report committed at `docs/solutions/agent-tuning/mika-arch-sonnet-vs-opus-2026-05-17.md`

## 4. Non-goals (deliberate scope cuts → follow-up tickets)

| Item | Why deferred | Follow-up |
|---|---|---|
| `mika-qa` scenario suite | Needs separate PR-context fixtures (AC1 + AC2 reviews, build callbacks); orthogonal to the model-swap-gate v1 | New ticket: "feat(eval): mika-qa calibration scenarios" |
| `permission-classifier` scenarios | Tied to `mika-relay` deprecation (sibling ticket); shape unknown until relay sunset lands | New ticket: "feat(eval): permission-classifier calibration scenarios (post-relay-deprecation)" |
| Continuous online calibration | Ticket explicitly out-of-scope; CI-gate variant of pre-swap discipline | New ticket if/when needed |
| Cost-tolerance CI auto-gate | v1 surfaces cost-vs-baseline in the report; humans decide. Automated gate is a config-policy decision | New ticket: "feat(eval): calibration CI auto-gate with cost tolerance" |
| Replacing existing eval harnesses (`golden/`, `grounding_regressions/`, `kg_provider_eval/`) | Those are correctness tests, not model-swap gates. Orthogonal — leave alone | n/a |
| Automated A/B "recommend a model" | Framework reports; humans decide (matches ticket's Out-of-scope) | n/a |

## 5. Architecture decisions

### DR-1: Role-scoped scenarios live in Rust, not YAML/JSON

**Ticket AC1 says** "Suite definitions exist as YAML/JSON files in `mika/docs/eval/calibration/<role>/`". **The plan deliberately deviates** to Rust-coded scenarios with **plaintext fixtures** (ticket bodies, brief texts) co-located under `crates/mika-agent/tests/eval/calibration_fixtures/<role>/*.md`.

**Why deviate:**

- All existing scenario suites (`golden/`, `grounding_regressions/`, `kg_provider_eval/`, `skills/`) are Rust. The data-driven YAML path requires building an assertion DSL — a hidden re-implementation of `grounding_assertions/mod.rs`.
- Scoring is non-trivial: response-text regex, tool-call shape inspection, trace-event analysis. None of those compress cleanly into declarative YAML.
- Rust-coded scenarios get type-checked at compile time; YAML scenarios fail at runtime when fixtures drift from skill manifests.
- The pattern that works at this scale already exists: `mika_arch_groom_milestone.rs` — synthetic skill + EvalHarness + assertions, ~150 lines per scenario.

**What we keep from AC1's spirit:**

- Scenario INPUTS (the ticket body / brief content / system prompt fragments) live as markdown files under `crates/mika-agent/tests/eval/calibration_fixtures/<role>/<scenario>.md`. Operators can edit fixtures without touching Rust.
- A YAML manifest per role at `crates/mika-agent/tests/eval/calibration_fixtures/<role>/manifest.yaml` declares which fixtures are part of the suite, their pass-rate weights, and their expected failure modes. The Rust scenarios load this manifest.

**Open question for architect**: is markdown fixtures + YAML manifest + Rust scoring an acceptable interpretation of AC1, or do we need a stricter declarative form?

### DR-2: `calibrate` is a binary, not a test target

**Decision**: New binary `crates/mika-agent/src/bin/calibrate.rs`. Reuses the calibration framework via promoted modules (see DR-3).

**Why a binary and not `cargo test ...`:**

- Pre-swap gate is operator-driven (the operator runs it manually, or CI runs it via `make`). `cargo test --ignored` syntax leaks test-runner detail that doesn't belong in a "swap gate" mental model.
- The binary produces a markdown report writable to `docs/eval/calibration/baselines/<date>.md` and `--output <path>` for ad-hoc runs. Tests do not produce committable artifacts cleanly.
- Exit code from a binary IS the gate. Exit 0 = pass (≥ baseline); exit 1 = fail. CI workflows can consume this directly.

**Counter-argument considered**: doesn't this duplicate the existing `test_real_provider_matrix.rs` runner? **Answer**: Yes, partially. The mitigation is DR-3 — promote the framework to a non-test module so the binary AND the test runner consume it identically. The duplication is the matrix-iteration loop, ~30 lines, acceptable.

### DR-3: Promote calibration modules from `tests/eval/` to `src/calibration/`

**Decision**: Move (not duplicate):

- `tests/eval/calibration.rs` → `src/calibration/artifact.rs`
- `tests/eval/scenarios.rs::Scenario` + `ScenarioOutcome` → `src/calibration/scenario.rs` (the provider-level scenarios stay in `tests/eval/scenarios.rs`; only the trait definitions move)
- `tests/eval/providers.rs::parse_provider_list` + `create_real_provider` → `src/calibration/providers.rs`

**Why**: A binary cannot depend on a `[[test]]` target. Today these modules live under `tests/eval/` and are only reachable from test code. Promoting to `src/calibration/` makes them available to the binary AND keeps the test integrations intact via the existing `mod` declarations in `tests/eval.rs` (re-export instead of own-module).

**Risk**: Refactor scope creep. Mitigation: this is a **mechanical move**, not a redesign. The schemas don't change. The test imports change paths only. A single commit, reviewed for path-only deltas.

**Architect challenge expected**: "Is this refactor necessary, or can the binary be a thin shell around `cargo test`?" — argued in DR-2; binary path is cleaner for pre-swap discipline. Open to alternatives.

**Path verification (F2 — see Phase 0 Pin above)**: the three promoted modules have no test-only crate dependencies (`mockall`, `tempfile`, `proptest`, etc.) in their main bodies, and their inline `#[cfg(test)]` blocks contain only internal unit tests that continue to compile and run after promotion. Decision: **path-only same-PR isolated commit** (Unit 1). No precursor PR needed.

### DR-4: Role definition is "skill + tool set + scenario suite"

A `Role` is a 3-tuple:

```rust
pub struct Role {
    pub name: &'static str,                  // "mika-dev", "mika-arch"
    pub skill_loader: fn() -> SkillRegistry, // synthetic skills matching production manifest
    pub scenarios: Vec<RoleScenario>,        // hand-coded
}
```

This MIRRORS how mika-dev and mika-arch actually operate in production: a base agent + a primary skill that defines its behavior. The framework doesn't need to know about mika's task system or message routing — `EvalHarness::builder()` already provides the agent loop.

**Why this shape**: it lets the framework be agent-agnostic. Adding `mika-qa` later is "add `Role::mika_qa()` constructor + N scenarios"; no framework change.

**D6 env-gate contract (F4 — three-tier execution model)**: per `crates/mika-agent/CLAUDE.md § Evaluation` (D6), the existing calibration tier is gated by `MIKA_EVAL_CALIBRATE=1`. DR-4's `Role::mika_arch()` does NOT supersede this gate. The pattern lifted from `mika_arch_groom_milestone.rs` continues to live at `tests/eval/skills/mika_arch_groom_milestone.rs` and now `pub use`s the promoted scenario implementation from `src/calibration/roles/mika_arch.rs`. The test continues to run under `cargo test -p mika-agent --test eval` (Unit tier with MockLlmProvider); the new `calibrate` binary (Unit 6) reaches the same scenario implementation via the role module and runs it against a real provider (Calibration tier). Both tiers share scenario code; neither tier is silently replaced. `crates/mika-agent/CLAUDE.md § Evaluation` requires no update.

### DR-5: Pass criteria — structural, not semantic

For each scenario, the pass condition is a structural check on the agent trace, not "did the LLM produce the right answer." Concretely, for mika-dev scenarios:

- **Trace-shape assertions**: tool call N is `tool_X`, message text matches/excludes regex set
- **No-refusal**: `response_text` does not match the refusal regex set (anchored on mika#1168's actual refusal strings)
- **No-fabrication**: tool calls reference tools in the registry; cited files exist in the synthetic worktree
- **Required-tools-gate (#890) satisfied**: if the skill declares required tools, final turn is self-contained

For mika-arch scenarios:

- **Disposition keyword as literal final line** (mirrors `mika_arch_groom_milestone.rs`)
- **Required suffix lines** (#864) per skill manifest
- **Required finding list** (#901) when verdict is ITERATE/ESCALATE
- **Citation-or-silence**: if response cites a path, that path was fetched via tool call (existing #863 guard logic, reused)

Semantic "did the answer help" gating is OUT OF SCOPE for v1 — it requires LLM-as-judge, which `golden/` already does for general quality and which is orthogonal to swap-gating.

### DR-6: Failure-mode taxonomy (new enum in `src/calibration/`)

```rust
pub enum FailureClass {
    Refusal,            // refusal regex match
    Fabrication,        // tool call to non-existent tool, or cite to non-existent file
    EmptyResponse,      // no text output and no tool calls
    Timeout,            // exceeded per-scenario latency budget (default 60s)
    TransportError,     // provider returned non-200 / network error
    ContractViolation,  // skill output contract failed (suffix line, finding list, etc.)
    Other(String),
}
```

This is the failure axis the report breaks out. It maps directly to known regression classes (mika#1168 = Refusal; mika#1166 = ContractViolation; mika#1173 = ContractViolation). The taxonomy is intentionally small and additive — new classes are added when a new regression appears that doesn't fit.

**Note on scope-creep (architect F3 / NF7)**: a `ScopeCreep` variant was considered and rejected for the enum. "Did the agent take too many tool calls" is not detectable from trace shape alone without semantic judgment, and DR-5 rules out LLM-as-judge for v1. When a specific scenario wants to enforce "at most N tool calls of type T," it does so via an explicit per-scenario assertion in the scenario's Rust code (e.g., `assert_tool_call_count(&trace, "run_gh", ..3)`) and maps the failure to `FailureClass::ContractViolation` for the aggregate report. This keeps the enum's detection paths uniform (all variants are mechanically detectable from the trace or transport result).

### DR-7: Cost tolerance — report-only in v1

The report emits `cost_delta_pct` per scenario relative to baseline. v1 **does NOT** auto-fail on cost delta; humans inspect the report. AC4 says "≥ baseline pass-rate AND cost-per-task within configured tolerance" — interpret "configured tolerance" as **policy in CLAUDE.md (humans-decide)** for v1, with the framework providing the number. Auto-gating is a v2 ticket once we have real cost data from N≥3 swaps.

### DR-8: Run count and flakiness — N=1 with explicit acceptance

v1 runs each scenario **once** per calibration. Real LLM non-determinism is acknowledged. The report includes a "confidence: single-shot" stamp.

Multi-run averaging (N=3, median) is a v2 ticket. The reason for N=1 in v1: cost. A 5-scenario × 2-role suite at N=3 is 30 LLM calls per calibration; at N=1 it is 10. For pre-swap gating we want it cheap enough that operators actually run it.

When a baseline scenario is N=1-flaky (e.g., the agent fails 1-in-3 times legitimately), file an `.flaky` annotation in the YAML manifest — the framework tolerates flaky scenarios by treating them as "soft" weighted at 0.5.

## 6. Implementation units

Each unit ships as a separate commit on `feat/1190/eval-model-calibration-framework-fixed` so the diff is reviewable.

### Unit 1 — Module promotion (mechanical refactor)

- Move `crates/mika-agent/tests/eval/calibration.rs` → `crates/mika-agent/src/calibration/artifact.rs`
- Move `parse_provider_list` + `create_real_provider` from `tests/eval/providers.rs` → `crates/mika-agent/src/calibration/providers.rs`. Leave `parse_real_providers()` (env-var reader) in tests/ — it's test-runner glue.
- Extract `ScenarioOutcome` + `Scenario` trait stubs to `crates/mika-agent/src/calibration/scenario.rs`. Provider-level scenarios stay in `tests/eval/scenarios.rs` and import from the new location.
- Update `tests/eval.rs` to re-declare via `pub use mika_agent::calibration::*;`
- **Validation**: `cargo build -p mika-agent` and `cargo test -p mika-agent --test eval -- --include-ignored --list` both succeed without diff to test count

**Files touched** (estimate): 6 moved, 4 modified, ~120 LOC net.

### Unit 2 — Role abstraction

- New `crates/mika-agent/src/calibration/role.rs` — `Role`, `RoleScenario`, `RoleScenarioRun`, `RoleScoreReport`.
- New `crates/mika-agent/src/calibration/failure.rs` — `FailureClass` enum and detection helpers (refusal regex set, tool-existence check).
- `Role::scenarios_under(provider)` is the iterator the runner consumes.

**Files touched**: 2 new, ~250 LOC.

### Unit 3 — Synthetic skill fixtures

- New `crates/mika-agent/tests/eval/calibration_fixtures/mika-dev/` with 5 markdown files (one per scenario: refusal_regression, contract_dev_groom, golden_path_dispatch, required_tools_gate, plan_callout_recognition) + `manifest.yaml`.
- New `crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/` with 5 markdown files (groom_ticket_basic, groom_milestone, citation_discipline, disposition_keyword_discipline, required_finding_list) + `manifest.yaml`.
- `manifest.yaml` schema:
  ```yaml
  version: 1
  role: mika-dev
  scenarios:
    - id: refusal_regression
      fixture: refusal_regression.md
      tags: [grounding, refusal]
      flaky: false
      weight: 1.0
      expected_failure_classes_absent: [Refusal, EmptyResponse]
  ```

**Files touched**: 12 new fixture files, 2 manifests, ~300 LOC of markdown.

### Unit 4 — mika-dev role implementation

- New `crates/mika-agent/src/calibration/roles/mika_dev.rs` — `Role::mika_dev()` constructor: synthetic dev-pilot skill (mirrors `skills/bundled/dev-pilot/skill.toml`), tool registration (mirrors what self-dev expects), 5 scenarios.
- Each scenario: load fixture markdown, call `EvalHarness::builder().with_real_provider(provider).with_skill(dev_pilot_skill).build()`, run, score.

**Files touched**: 1 new module + 5 scenario files, ~600 LOC.

### Unit 5 — mika-arch role implementation

- New `crates/mika-agent/src/calibration/roles/mika_arch.rs`. Reuses `tests/eval/skills/mika_arch_groom_milestone.rs` patterns directly — that file IS already a role-scoped scenario in current form. Lift its skill construction, generalize for groom-ticket too.

**Files touched**: 1 new module + 5 scenario files, ~500 LOC.

### Unit 6 — `calibrate` binary

- New `crates/mika-agent/src/bin/calibrate.rs` — clap-based CLI:
  ```
  calibrate --role <mika-dev|mika-arch> \
            --model <provider/model> \
            [--baseline docs/eval/calibration/baselines/<date>.md] \
            [--output <path>] \
            [--max-cost-usd 5.0]
  ```
- Loads role, parses provider/model, iterates scenarios, writes JSON artifact + markdown report.
- Exit codes: 0 = pass (≥ baseline OR no baseline supplied + run committed), 1 = fail (pass-rate below baseline), 2 = transport error / config error.

**Files touched**: 1 new, ~250 LOC.

### Unit 7 — Makefile + dev script integration

- Add `mika/Makefile` targets:
  ```make
  calibrate-mika-dev: ## Pre-swap calibration gate for mika-dev (MODEL=provider/model required)
  calibrate-mika-arch: ## Pre-swap calibration gate for mika-arch (MODEL=provider/model required)
  ```
- Both invoke `cargo run --bin calibrate --release -- --role <role> --model "$(MODEL)" --baseline docs/eval/calibration/baselines/latest.md`
- `latest.md` is a symlink to the most-recent dated baseline (UNIX convention).

**Files touched**: Makefile +20 LOC, 1 symlink.

### Unit 8 — Initial baseline + A/B + docs

- Run the binary against current production models:
  - `make calibrate-mika-dev MODEL=anthropic/claude-sonnet-4-6` → commit JSON + markdown report
  - `make calibrate-mika-arch MODEL=anthropic/claude-opus-4-6` → commit JSON + markdown report
- A/B run: `make calibrate-mika-arch MODEL=anthropic/claude-sonnet-4-6` → comparison doc at `docs/solutions/agent-tuning/mika-arch-sonnet-vs-opus-2026-05-17.md`
- Update `mika/CLAUDE.md`:
  - Add pre-swap discipline paragraph (AC5)
  - Add link to baseline + framework docs
- Update `crates/mika-agent/CLAUDE.md § Evaluation`:
  - Add fourth subsection "Evaluation — Model Calibration (#1190)" with run instructions, scenario lists, baseline location

**Files touched**: 5 new committed reports, 2 CLAUDE.md edits, ~600 LOC of generated content.

## 7. AC mapping

| AC | Unit | Notes |
|---|---|---|
| AC1 — Suite definitions exist as YAML/JSON in `docs/eval/calibration/<role>/` (10-20 scenarios per role) | Unit 3 (partial) | **Interpretation deviation**: see DR-1. v1 ships 5 scenarios per role with markdown fixtures + YAML manifest at `crates/mika-agent/tests/eval/calibration_fixtures/<role>/`. Documented in plan; flagged for grooming. |
| AC2 — `cargo run --bin calibrate -- --role <role> --model <id>` runs suite, produces JSON+markdown report | Unit 6 | Direct match |
| AC3 — Baseline run against all current models committed to `docs/eval/calibration/baselines/<date>.md` | Unit 8 | v1 commits baseline for `mika-dev` (current model only — sonnet) and `mika-arch` (opus + sonnet for A/B). Other roles' baselines deferred to their follow-up tickets. |
| AC4 — `make calibrate-<role> MODEL=<id>` is canonical pre-swap gate | Unit 7 | Direct match; "tolerance" is report-only per DR-7 |
| AC5 — CLAUDE.md updated with pre-swap rule | Unit 8 | Direct match |
| AC6 — At least one A/B test committed | Unit 8 | Direct match — mika-arch sonnet vs opus |

## 8. Risks and mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| **Real-provider cost spike**: 5 scenarios × 2 roles × 11 providers × 4 baseline runs ~= 440 LLM calls if "baseline against all current models" interpreted literally | Med | v1 baselines only ACTIVE models (sonnet, opus), not all configured providers. ~20 LLM calls per baseline pass. `--max-cost-usd` flag stops mid-suite if budget exceeded. |
| **Flakiness from N=1**: a real model has bad luck on one scenario, baseline locks the bad luck in | Med | DR-8: `.flaky` annotation in manifest weights soft. Initial baseline runs each scenario 3 times by hand, picks median outcome. Subsequent CI runs are N=1. |
| **Refactor (Unit 1) breaks the existing eval matrix** | Low | Mechanical move, no schema change. Validation: test count and `--list` output unchanged pre/post. |
| **Skill-prompt drift**: scenarios encoded against today's `dev-pilot/skill.toml`; tomorrow's skill change breaks the synthetic skill | High over time | Synthetic skill mirrors production skill manifest. When production skill changes, calibration scenarios are updated in the same PR (manifest pin in `manifest.yaml`'s `skill_sha` field). Add to skill-change checklist in `mika-skills/CLAUDE.md`. |
| **Scope creep into v2 work** during review | High | This plan's §4 (Non-goals) is explicit. Follow-up tickets are filed BEFORE this PR opens; reviewers can challenge scope without re-litigating. |
| **AC1 interpretation rejected**: architect or operator demands literal YAML/JSON declarative scenarios | Med | DR-1 documents the deviation with reasoning. If rejected, scope balloons by ~1.5x (assertion DSL). File as escalation; do NOT silently ship the larger version. |
| **`calibrate` binary depends on `mika-agent` library compiling cleanly** | Low | The binary is in the same crate as `mika-agent`; if mika-agent doesn't compile, neither does the gate. This is fine. |
| **Baseline file rot**: future swaps don't refresh the baseline → "passing" becomes meaningless | Med | CLAUDE.md rule (AC5): every swap PR commits the new baseline. Lefthook check (follow-up ticket): block PR if `crates/*/Cargo.toml` model defaults change without `docs/eval/calibration/baselines/` change. |

## 9. Open questions (resolved by architect first-pass + retro)

All first-pass questions are resolved. Trail:

1. ~~DR-1 (YAML/JSON vs Rust scenarios)~~ — **RESOLVED 2026-05-17**: Vincent edited mika#1190 AC1 to match DR-1's shape (markdown fixtures + YAML manifest + Rust scoring); architect session `fd1e7375-7872-41d0-8629-809a4261bffb` retro acknowledged F3 resolution.
2. ~~DR-3 (module promotion scope)~~ — **RESOLVED**: Phase 0 Pin §F2 verification shows all three modules are path-only-safe. Same-PR isolated commit.
3. ~~DR-7 (cost tolerance report-only)~~ — **RESOLVED**: architect first-pass NF4 confirmed AC4 verbatim does not mention tolerance enforcement; the cost-tolerance clause appears only in ticket §4 prose, which is scope, not AC. Report-only in v1 is AC4-compliant.
4. ~~Run count (DR-8)~~ — **RESOLVED**: architect NF5 ratified N=1 with `.flaky` annotations for v1; the `calibrate` binary exposes `--runs-per-scenario` (default 1) from day one so v2 N=3 is a flag change.
5. ~~Scenario count (5 vs 10-20)~~ — **RESOLVED**: architect NF2 ratified; AC1 text updated to "v1 ships 5+ per active role; follow-ups grow toward 10-20."
6. ~~Scope cut of mika-qa and permission-classifier~~ — **RESOLVED**: architect NF6 ratified deferral; mika-qa needs synthetic PR context fixtures and gets its own ticket post-framework.

## 10. Compounding & follow-ups

To file at PR-open time (separate tickets, all under `agent-core` label):

- "feat(eval): mika-qa calibration scenarios" — 5-7 scenarios for PR review role (AC1/AC2 review, build callbacks)
- "feat(eval): permission-classifier calibration scenarios (post-relay-deprecation)"
- "feat(eval): calibration CI auto-gate with cost tolerance" — v2 cost-gate
- "feat(eval): N≥3 multi-run averaging for calibration scenarios" — flakiness mitigation
- "chore(eval): lefthook guard — model change in `Cargo.toml` requires baseline refresh"
- "feat(eval): scale role suites to 10-20 scenarios each" — grow toward AC1's full target

References for the compounding doc (post-merge):

- `feedback_read_skill_prompt_before_model_downshift.md` — what calibration would have caught
- `project_mika_dev_model_switch.md` — the 2026-05-07 swap that motivated this framework
- mika#1168, mika#1166, mika#1173 — the incident chain encoded as first-class scenarios

## 11. Validation checklist (for /ce:work)

- [ ] Unit 1 refactor: `cargo build -p mika-agent` clean, `cargo test --test eval -- --list` count unchanged
- [ ] Unit 6 binary: `cargo run --bin calibrate -- --help` shows clap usage
- [ ] Unit 8 baseline: `make calibrate-mika-dev MODEL=anthropic/claude-sonnet-4-6` produces committed JSON + markdown
- [ ] Unit 8 A/B: `docs/solutions/agent-tuning/mika-arch-sonnet-vs-opus-2026-05-17.md` exists with side-by-side pass-rate + cost table
- [ ] CLAUDE.md rule: `mika/CLAUDE.md` contains pre-swap prohibition language; `crates/mika-agent/CLAUDE.md § Evaluation` updated
- [ ] All 6 follow-up tickets filed and linked in PR body
- [ ] Cost budget: total API spend for baseline + A/B run logged in PR body (target: <$5)
- [ ] Lefthook + clippy + tests pass on the branch
