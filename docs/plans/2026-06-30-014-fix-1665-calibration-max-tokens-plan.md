---
issue: 1665
type: fix
date: 2026-06-30
---

# Plan — fix(calibration): refusal_regression max_tokens too tight for reasoning-mode models (mika#1665)

## Problem

`crates/mika-agent/src/calibration/roles/mika_dev.rs::run_refusal_regression` sets `max_tokens: 1000` on the `LlmRequest` (line 106). For reasoning-mode models (Z.AI GLM-5.2, DeepSeek-R1, o1-style), the entire output budget can be consumed by internal reasoning tokens before any visible content is emitted — producing a false `EmptyResponse` classification.

The other 4 mika-dev calibration scenarios use `max_tokens: 2000` (line 179 of the same file) and pass cleanly. The refusal_regression scenario alone fails because its budget is half of theirs.

**Hard evidence (body-cited):** GLM-5.2 via Z.AI direct, scenario refusal_regression:
- `input_tokens: 353`, `output_tokens: 1000` (cap), `reasoning_tokens: 1000` (entire budget burned on reasoning), `content: empty`, `finish_reason: "length"`.
- Reasoning preview shows legitimate task analysis ("Let me analyze... 1. Remove stale build artifacts..."), NOT refusal.
- The classifier surfaces this as `EmptyResponse` → 80% pass rate → blocks the mika#1190 calibration gate.

## Architectural lineage

- mika#1190 — calibration discipline (the gate this blocks).
- mika#1633 — glm-5.2 swap (introduced reasoning-mode behavior).
- mika#1657 — Z.AI native provider (surfaced the constraint with `reasoning_tokens` field).
- mika#1659 / mika#1663 — sibling zai/glm-5.2 hygiene fixes (independent code paths).

## Fix shape (two units, independent)

### Unit A (primary, required for the gate) — bump max_tokens

Change `crates/mika-agent/src/calibration/roles/mika_dev.rs:106` from `max_tokens: 1000` to `max_tokens: 2000`. Parity with the other 4 scenarios in the same file. Minimal-risk one-line change.

### Unit B (classifier sharpening, defense-in-depth) — `ReasoningBudgetExhausted` class

Extend `crates/mika-agent/src/calibration/failure.rs::FailureClass` enum with a new variant `ReasoningBudgetExhausted`. Extend `classify_failure()` to detect: `output_tokens > 0` AND visible content empty AND finish_reason == "length" → new class. Emit operator-actionable message: "Output budget consumed by reasoning tokens before visible content. Consider raising `max_tokens`."

Unit B requires `reasoning_tokens` / `finish_reason` to be available to the classifier. The body notes "depends on mika#<reasoning-content-ticket> for the reasoning_tokens field to be surfaced." If the field surfacing hasn't shipped, Unit B is gated; defer Unit B to a follow-up. Unit A is sufficient to unblock the gate today.

### Unit C (architect-bearing, optional) — per-scenario max_tokens override

Add optional `max_tokens` field to scenario YAML manifest frontmatter (currently a `markdown` body — would extend the manifest schema). Scenario-level override would let per-scenario budgets be tuned without code changes. This is bigger scope than the body's "scenario-configurable" suggestion implies. Defer if architect agrees; ship Unit A alone for now.

## Implementation outline

1. **Edit `crates/mika-agent/src/calibration/roles/mika_dev.rs:106`** — bump `max_tokens: 1000` to `max_tokens: 2000`. Single one-line change.

2. **Re-run calibration to confirm AC2:** `make calibrate-mika-dev MODEL=zai/glm-5.2` after the bump. Capture the resulting markdown report + JSON artifact for PR evidence. Expect 100% pass.

3. **Unit B (optional, ship if reasoning_tokens field is surfaced):** extend `FailureClass` + `classify_failure()` per §Fix shape Unit B. If the underlying `reasoning_tokens` field isn't yet populated by the provider adapter, skip Unit B and file a follow-up ticket tying it to the field-surfacing work.

4. **Unit C explicitly OUT OF SCOPE for this PR.** File as follow-up if per-scenario tuning becomes recurrent need.

## Acceptance criteria

- **AC1** — `crates/mika-agent/src/calibration/roles/mika_dev.rs::run_refusal_regression` uses `max_tokens: 2000` (parity with peer scenarios). Verified by reading PR diff.

- **AC2** — `make calibrate-mika-dev MODEL=zai/glm-5.2` returns 100% pass after the bump. PR body includes the markdown report excerpt.

- **AC3 (Unit B, optional)** — `FailureClass::ReasoningBudgetExhausted` variant added. `classify_failure()` returns it when `output_tokens > 0` AND visible content empty AND `finish_reason == "length"`. Unit test covers the classification logic. **Gated on `reasoning_tokens` field being surfaced to the classifier** — if not yet plumbed, this AC is deferred and AC1 alone closes the ticket; file follow-up.

## Out of scope

- **`reasoning_content` surfacing** — separate ticket (referenced from body but not co-shipped).
- **Per-scenario max_tokens override** (Unit C from body) — yet another follow-up if recurrent.
- **Re-running calibration on other agents/models** — Unit A's bump is specific to refusal_regression on mika-dev. Other suites aren't blocked by this.

## Files involved

- `crates/mika-agent/src/calibration/roles/mika_dev.rs:106` — single-line max_tokens bump
- `crates/mika-agent/src/calibration/failure.rs` — Unit B variant + classifier (if shipped this PR)
- No schema migration; no test file changes (calibration test surface already exercises classify_failure)

## Verification

- **Static:** PR diff shows single-line change at line 106 (Unit A). Unit B (if shipped) is additive to `FailureClass` enum + `classify_failure()` + a new unit test.
- **Empirical (AC2):** PR body includes `make calibrate-mika-dev MODEL=zai/glm-5.2` markdown report showing 100% pass post-fix.
- **Regression:** other 4 mika-dev calibration scenarios still pass (they already use `max_tokens: 2000`, no regression risk).

## References

- mika#1190 — calibration discipline
- mika#1633 — glm-5.2 swap
- mika#1657 — Z.AI native provider
- `crates/mika-agent/src/calibration/roles/mika_dev.rs:106` — the line this fix changes
- `crates/mika-agent/src/calibration/failure.rs:22,101,105` — EmptyResponse classification logic (Unit B's extension target)
- Body-cited calibration artifact: `target/eval-calibration/mika-dev-20260630-062000.{md,json}`
