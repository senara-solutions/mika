---
module: eval
date: 2026-06-30
problem_type: bug_fix
component: testing_framework
severity: medium
tags:
  - model-calibration
  - reasoning-mode-models
  - max-tokens
  - failure-classification
  - false-positive
  - eval-harness
applies_when: "Authoring or tuning calibration scenarios for reasoning-mode models (Z.AI GLM-5.2, DeepSeek-R1, o1-style) that emit internal reasoning tokens before visible content"
---

# Calibration: reasoning-budget exhaustion masquerades as EmptyResponse (mika#1665)

## Context

A calibration scenario sets `max_tokens` on its `LlmRequest` to bound cost. For
non-reasoning models a 1000-token budget is ample for the short structured
responses the calibration scenarios expect. For **reasoning-mode models**
(Z.AI GLM-5.2, DeepSeek-R1, o1-style), the model spends output-token budget on
*internal reasoning* before emitting any visible content — and the cap is
charged against the **combined** reasoning + visible budget.

`crates/mika-agent/src/calibration/roles/mika_dev.rs::run_refusal_regression`
was the only mika-dev scenario at `max_tokens: 1000` (the other four use 2000).

## Symptom

GLM-5.2 via Z.AI direct, scenario `refusal_regression`:

- `input_tokens: 353`, `output_tokens: 1000` (the cap), `reasoning_tokens: 1000`
  (entire budget burned on reasoning), visible `content: empty`,
  `finish_reason: "length"`.
- Reasoning preview was *legitimate task analysis* ("Let me analyze... 1. Remove
  stale build artifacts...") — the model was working the task, **not refusing**.
- The scenario's empty-content branch produced `FailureClass::EmptyResponse`
  → 80% pass rate → blocked the mika#1190 pre-swap calibration gate → the
  zai/glm-5.2 swap looked like a regression and would be reverted unnecessarily.

The model was healthy. The harness was starving it.

## Fix (two parts)

### 1. Budget parity (the unblock)

Raise `refusal_regression`'s `max_tokens` from 1000 to 2000, matching the peer
scenarios. One reasoning pass + a short visible plan now fits. This alone
unblocks the gate.

### 2. Failure-class sharpening (defense-in-depth)

A budget bump is not a guarantee — a sufficiently reasoning-heavy model can
exhaust *any* finite budget. So the classifier now distinguishes the two cases
instead of collapsing both into `EmptyResponse`:

- New `FailureClass::ReasoningBudgetExhausted` variant.
- `classify_failure()` gained two signals — `output_tokens: Option<u64>` and
  `finish_reason_is_length: bool`. When visible content is empty **and**
  `output_tokens > 0` **and** the finish reason is length-capped, it returns
  `ReasoningBudgetExhausted` (remediation: raise `max_tokens`) rather than
  `EmptyResponse` (remediation: treat as model regression).
- Shared `roles::empty_response_result()` helper builds the failure result from
  an `LlmResponse`, reading `usage.output_tokens` and
  `stop_reason == LlmStopReason::MaxTokens`. All five mika-dev scenarios route
  their empty-content branch through it, so the sharper label is uniform.

The three detection signals are all available from `LlmResponse` today — no
dependency on a separately-surfaced `reasoning_tokens` field. `finish_reason ==
"length"` maps to `LlmStopReason::MaxTokens` in the OpenAI-compatible adapter
(`crates/mika-common/src/llm/openai.rs:750`), which is the path Z.AI direct
takes.

## Why this matters beyond one scenario

- `expected_failure_classes_absent` is **not** wired into pass/fail scoring
  (`RoleScoreReport::from_results` keys only on `result.passed`). Adding a new
  variant is therefore gate-safe — it changes the *label* in the failure
  breakdown, never the pass/fail count. The value of the new class is operator
  legibility: the breakdown table now names the actual problem.
- A misclassified false-fail in a *pre-swap gate* is uniquely costly: it doesn't
  just lose a test, it reverts a correct model swap. Classification precision in
  gate harnesses pays for itself.

## Guidance for new calibration scenarios

- Budget reasoning-mode headroom: 2000 is the floor for short structured
  responses; reasoning models routinely spend 500–1500 tokens thinking first.
- Never classify empty visible content as `EmptyResponse` without checking
  `output_tokens` and the stop reason first — route through
  `empty_response_result()`.
- If a *non-empty* response is still truncated mid-sentence (`MaxTokens` with
  content present), that is a different signal (genuine length overrun), not
  reasoning-budget exhaustion — the guard only fires on empty content.

## Out of scope (follow-ups)

- Re-running the full `make calibrate-mika-dev MODEL=zai/glm-5.2` matrix to
  capture a 100% artifact requires the `MIKA_ZAI_API_KEY` and is tracked as a
  separate operator-run follow-up (runtime already verified empirically via
  canaries per mika#1665).
- Per-scenario `max_tokens` override via fixture frontmatter (manifest schema
  extension) — deferred unless per-scenario tuning becomes a recurring need.
- Surfacing `reasoning_tokens` into `LlmUsage` for richer evidence — separate
  ticket; not required for the classification above.

## References

- mika#1665 — this fix
- mika#1190 — calibration discipline (the gate this unblocks)
- mika#1633 — glm-5.2 swap that surfaced reasoning-mode patterns
- mika#1657 — Z.AI native provider
- `docs/solutions/best-practices/model-calibration-framework-structural-assertions-2026-05-18.md`
  — the framework this scenario lives in
