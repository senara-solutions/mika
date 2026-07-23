# Plan: widen calibration scenario artifact JSON to include LLM response text (verify-not-guess)

**Ticket:** mika#1716 — `feat(calibration): widen scenario artifact JSON to include LLM response text (verify-not-guess)`
**Type:** feat (enhancement)
**Author:** dev-groom (content-only pilot)
**Date:** 2026-07-23 (re-groom; content re-validated against current `role.rs`/`artifact.rs`/`eval-diff.rs`)

## Problem

The calibration artifact JSON (`target/eval-calibration/<role>-<ts>.json`, and the committed
baselines under `docs/eval/calibration/**`) records per-scenario `outcome` / `error_class` /
tokens / latency, but **discards the LLM response text and the raw human-readable failure
reason**. When a scenario fails (e.g. `deploy_gate_discipline` → *"Did not name `make deploy`
as the deploy path"*), the artifact only preserves the classified `error_class: "unknown"` — not
the reason string, and never the model's actual words. During the mika#1641 orchestrator
commissioning (PR#1715) both candidate models failed `deploy_gate_discipline` and there was no
way to tell a **real fail** (model proposed a hack) from **fixture-strictness** (model said
"run the deploy target", missing the literal `make deploy` substring) without re-running with
debug logging. This ticket closes that diagnostic gap.

## Root cause (grounded)

Data flow (verified in the worktree):

1. Role runners (`crates/mika-agent/src/calibration/roles/{mika_dev,mika_arch,mika_qa,mika_orchestrator}.rs`)
   call `response.send_message()`, bind `let text = response.text().to_string();`, run structural
   assertions, and return a `RoleScenarioResult` via `RoleScenarioResult::pass(...)` /
   `RoleScenarioResult::fail(...)`. **`text` is used for the assertions (`text.to_lowercase()`) then
   dropped** — it never reaches the result struct.
2. `calibrate.rs:229-233` maps each result → `ScenarioOutcome` via
   `RoleScenarioResult::to_scenario_outcome(role, model)`, which **hardcodes `response_text: None`**
   (`role.rs:95`).
3. `CalibrationArtifact::from_outcomes()` (`artifact.rs:82-114`) builds one `ScenarioCalibration`
   per scenario. **`ScenarioCalibration` has no `response_text` and no `failure_reason` field** — it
   keeps only `error_class = classify_error(error)`, discarding the raw `error` string.
4. `RoleScoreReport::to_markdown()` (`role.rs:190-251`) renders the `.md` companion from
   `Vec<RoleScenarioResult>`; it has no per-scenario response/reason section.

So the fix is three connected schema extensions (result struct → outcome → artifact JSON) plus the
markdown surface, with a backwards-compatible serde contract for the committed baselines.

## Design decisions

- **DR-1 — `failure_reason` reuses the existing `error` string; only `response_text` is genuinely
  new state.** `RoleScenarioResult` already carries `error: Option<String>` — the exact
  human-readable reason AC1 describes (*"Did not name `make deploy`…"*). Adding a *second*
  `failure_reason` field to the struct would duplicate load-bearing state (a DRY / single-source-of-truth
  violation per `docs/architecture/review-guide.md`). Instead: add **`response_text: Option<String>`**
  to `RoleScenarioResult` (the only field currently thrown away), and at the JSON-serialization layer
  (`ScenarioCalibration`) expose **both** `response_text` (new) and `failure_reason` (sourced from
  `ScenarioOutcome.error`, i.e. the raw reason — distinct from the classified `error_class`). This is
  faithful to AC1's explicit *"(or per-scenario JSON serialization)"* latitude and to AC2's *"at the
  same nesting as `outcome`"*.
- **DR-2 — cap = 8000 chars, UTF-8-safe.** Truncation must cut on a char boundary
  (`text.chars().take(N).collect()`), **not** a byte slice — the CI `byte-slice-lint` job
  (`scripts/check-byte-slices.sh`) rejects `&str` byte-slicing that panics on multi-byte UTF-8
  (mika#764). Append a marker when capped: `"… [truncated to 8000 chars]"`. Const
  `RESPONSE_TEXT_CAP: usize = 8000` in `artifact.rs`.
- **DR-3 — backwards-compat via `#[serde(default)]`.** New `ScenarioCalibration` fields are
  `Option<String>` with `#[serde(default)]` (same pattern the file already uses for `latency_ms`
  at `artifact.rs:51`). The committed baselines (`mika-qa-1632`, `mika-dev-1633`, `mika-dev-1221`)
  are v1 and lack the fields; the swap-gate loads them via `CalibrationArtifact::load()`
  (`calibrate.rs:270`), so they MUST keep deserializing. `#[serde(default)]` guarantees it.
- **DR-4 — schema version bump v1 → v2.** `CalibrationArtifact::from_outcomes()` currently hardcodes
  `version: 1`. Bump to `2`. Nothing gates on `version == 1` in production code (the gate compares
  `unweighted_pass_rate`; `diff_calibrations` compares `outcome`); only `test_calibration_round_trip`
  asserts `version == 1` — update it. Old v1 baselines still load (version is not validated on read).
- **DR-5 — wire all four role runners, not just orchestrator.** The schema is role-agnostic; a
  half-wired schema (populated only for `mika_orchestrator`) would silently emit `response_text: null`
  for dev/arch/qa and read as "captured" when it wasn't. Thread `response_text` through every
  scenario runner in all four role files via a builder method (below). Error-path helpers in
  `roles/mod.rs` (`llm_error_result`, `empty_response_result`) legitimately leave `response_text = None`
  — there is no visible response text on a transport error or an empty/refusal turn.
- **DR-6 — `eval-diff.rs` needs no change.** The weekly-drift binary has its *own* private
  `ScenarioCalibration` struct (`eval-diff.rs:31-38`) without `#[serde(deny_unknown_fields)]`; serde
  ignores unknown fields by default, so new fields in written artifacts do not break its parse. (Its
  pre-existing bare `latency_ms: u64` — which already cannot parse a `null` latency — is a separate
  concern, out of scope here.)
- **DR-7 — AC7 doc target.** There is no `docs/eval/calibration/README.md` yet (only a per-baseline
  README under `mika-orchestrator-1641/`). Create `docs/eval/calibration/README.md` with an **Artifact
  schema** section documenting the v1 → v2 evolution. Verified this path is NOT copied by
  `scripts/sync-agent-docs.sh` (that script syncs a fixed doc list + `docs/openapi/`), so it does not
  trip the `docs-sync` CI job.

## Implementation steps

### 1. `crates/mika-agent/src/calibration/role.rs` — `RoleScenarioResult`
- Add field `pub response_text: Option<String>` to the struct (after `error`).
- In `pass()` and `fail()` constructors, initialize `response_text: None` (keeps every existing
  callsite in all four role files compiling unchanged — no signature churn).
- Add a builder:
  ```rust
  /// Attach the LLM response text captured during the scenario run (mika#1716).
  pub fn with_response_text(mut self, text: impl Into<String>) -> Self {
      self.response_text = Some(text.into());
      self
  }
  ```
- In `to_scenario_outcome()`, replace `response_text: None` with
  `response_text: self.response_text.clone()`.
- In `RoleScoreReport::to_markdown()`, after the per-scenario results table, add a **Failure Details**
  section (AC4): for each `result` where `!result.passed`, emit the scenario id, the
  `failure_reason` (= `result.error`), and the response text truncated for display (reuse a shared
  truncation helper / the `RESPONSE_TEXT_CAP` const — display cap can be smaller, e.g. show
  "Response text (truncated to N chars)"). Guard the whole section behind "any failures exist" so
  passing reports are unchanged.

### 2. `crates/mika-agent/src/calibration/artifact.rs` — `ScenarioCalibration` + cap
- Add `const RESPONSE_TEXT_CAP: usize = 8000;`.
- Add a helper `fn cap_response_text(text: Option<&str>) -> Option<String>` that char-safe-truncates
  (DR-2) and appends the marker when capped.
- Extend `ScenarioCalibration`:
  ```rust
  #[serde(default)]
  pub response_text: Option<String>,
  #[serde(default)]
  pub failure_reason: Option<String>,
  ```
- In `from_outcomes()`, populate:
  - `response_text: cap_response_text(outcome.response_text.as_deref())`
  - `failure_reason: outcome.error.clone()` (raw reason; `error_class` stays the classified value)
- Bump `version: 1` → `version: 2` (DR-4).

### 3. Role runners — thread `response_text` (all four files)
`roles/mika_orchestrator.rs`, `roles/mika_dev.rs`, `roles/mika_arch.rs`, `roles/mika_qa.rs`.

For every scenario runner, in the `Ok(response)` branch attach the captured text to the returned
result. Recommended low-churn pattern per runner: bind `text` once (already present in most), and
attach at each `pass()`/`fail()` return via `.with_response_text(text.clone())`. Where a runner has
many early-return sites, an equivalent single-attach form is acceptable:
```rust
let base: RoleScenarioResult = { /* existing checks returning pass()/fail() */ };
base.with_response_text(text)
```
Leave the `roles/mod.rs` error helpers (`llm_error_result`, `empty_response_result`) returning
`response_text = None` (DR-5). Any `Err(_)` arm that routes through `llm_error_result` needs no change.

### 4. `docs/eval/calibration/README.md` (new — AC7)
- Create with an **Artifact schema** section documenting: v1 (outcome/error_class/tokens/latency)
  → v2 (adds per-scenario `response_text` capped at 8000 chars + `failure_reason` raw reason;
  both `Option`, both `#[serde(default)]` for backward-compat). Note AC5: response text is treated
  as ordinary opaque completion text — no PII/secret scrubbing is applied (a fixture that quotes a
  secret would be a fixture bug, not an artifact-schema one).

### 5. Tests (`artifact.rs` `#[cfg(test)]` + `role.rs`)
- Update `test_calibration_round_trip`: assert `version == 2`.
- New: `from_outcomes` populates `response_text` + `failure_reason` for both a PASS (response_text
  Some, failure_reason None) and a FAIL (both Some).
- New: `cap_response_text` truncates a >8000-char string to ≤ 8000 + marker, on a char boundary
  (include a multi-byte UTF-8 char at the boundary to prove no panic), and passes short strings
  through unchanged.
- New: backwards-compat — deserialize a v1-shaped JSON blob (no `response_text` / `failure_reason`
  keys) and assert it loads with both fields defaulting to `None` (AC6). Reuse the existing
  `test_load_tolerates_null_latency` fixture shape.
- New: markdown report contains a Failure Details entry (reason + response snippet) when a result
  failed, and omits the section when all pass.

## Verification contract

- `cargo build -p mika-agent` — clean (both `calibrate` and `eval-diff` bins compile).
- `cargo test -p mika-agent calibration::` — all calibration unit tests pass, including the new ones.
- `cargo test -p mika-agent calibration::artifact` — round-trip (v2), cap, backwards-compat pass.
- `cargo clippy -p mika-agent` — no new warnings.
- `cargo fmt` — clean.
- `scripts/check-byte-slices.sh` — clean (proves DR-2 char-safe truncation).
- Manual/inspection: an existing committed baseline (e.g.
  `docs/eval/calibration/mika-qa-1632/mika-qa-glm-5.2-post-1632.json`) still loads via
  `CalibrationArtifact::load()` under the v2 schema (covered by the backwards-compat test).
- Non-regression: the swap-gate exit-code logic (`gate::evaluate_gate`, `calibrate.rs`) is untouched;
  `tests/calibrate_integration.rs` still passes.

## Out of scope (from ticket + discovered)

- Fixing the specific mika#1641 `deploy_gate_discipline` fixture-strictness question (PR#1715 follow-up).
- LLM-as-judge grading (mika#1641 article §3 design evolution).
- `eval-diff.rs`'s pre-existing bare `latency_ms: u64` (cannot parse `null` latency) — separate defect.
- Re-generating or committing any live calibration baseline (requires real provider keys; forbidden
  as fabricated evidence per the `mika-orchestrator-1641/README.md` discipline).

## Definition of Done

- `RoleScenarioResult` carries `response_text`; `to_scenario_outcome` forwards it (no longer hardcoded
  `None`).
- Artifact JSON (`ScenarioCalibration`) serializes `response_text` (capped 8000, UTF-8-safe) and
  `failure_reason` (raw reason) at the same nesting as `outcome`, for both PASS and FAIL scenarios.
- New fields are `#[serde(default)] Option<String>`; all committed v1 baselines still load.
- Schema `version` is `2`; the bump is documented in `docs/eval/calibration/README.md`.
- The `.md` companion report shows a per-FAIL response-text + reason section.
- All four role runners populate `response_text`; error-path helpers leave it `None`.
- Full verification contract green (build, tests, clippy, fmt, byte-slice lint).

## Acceptance criteria

Transcribed verbatim from mika#1716:

- **AC1** — extend `RoleScenarioResult` (or per-scenario JSON serialization) with two optional fields:
  `response_text: Option<String>` (full LLM output) and `failure_reason: Option<String>` (the
  human-readable reason currently printed to stdout, e.g. *"Did not name `make deploy` as the deploy
  path"*).
- **AC2** — serialize both fields into the artifact JSON at the same nesting as `outcome`. Include for
  both PASS and FAIL scenarios (for PASS, response_text supports scenario-tuning; for FAIL it enables
  the verify-not-guess diagnostic).
- **AC3** — trim to a reasonable cap — cap `response_text` at 8000 chars (or similar) to avoid multi-MB
  artifacts on verbose models. Truncate with a suffix marker if capped.
- **AC4** — matching change in the markdown report — the `.md` companion should show `Response text
  (truncated to N chars)` per FAIL scenario at minimum, so a human reading the report can spot
  fixture-strictness at-a-glance.
- **AC5** — no PII/token-secret guard needed — LLM responses are opaque completion text; treat as
  ordinary logging. (If a scenario's LLM output ever quotes a secret from the fixture, that's a
  fixture bug, not an artifact-schema one.)
- **AC6** — backwards-compat — old artifacts without these fields still parse (fields are `Option<>`).
- **AC7** — schema-version bump documented — `docs/eval/calibration/README.md` or equivalent notes the
  schema evolution (v1 → v2).
