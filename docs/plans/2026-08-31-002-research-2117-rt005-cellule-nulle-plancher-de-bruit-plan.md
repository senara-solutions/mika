---
issue: mika#2117
title: RT-005 Null Cell — Measuring the Apparatus Noise Floor - Plan
type: research
scope_repo: mika
priority: p1-important
date: 2026-08-31
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# RT-005 Null Cell — Measuring the Apparatus Noise Floor - Plan

## Goal Capsule

**Objective.** Anyone reading an RT-005 contrast can tell whether it is larger than
what the apparatus produces on identical inputs. Today that number does not exist,
so no contrast can be called an effect rather than an artefact.

**Means.** A null cell that holds the question constant and varies only the factors
suspected of producing the −7,4: sampling, agent identity, and position in batch.
Decomposed, not aggregated (KTD1).

**Authority hierarchy.** `research/rt005-physics-pilot/orchestration/PREREGISTRATION.md`
> issue ACs > this plan. The pre-registration is sealed; this work does not amend it.

**Stop conditions.**
- Stop if the design would change any RT-005 estimand, arm, or item. This measures
  the apparatus, never the hypothesis.
- Stop if the null cell would run on the Claude quota. The pilot runs on z.ai; a
  measurement that competes with product work is not affordable at this size.
- Stop before any scale-up of RT-005 itself. This ticket is that gate.
- If the measured floor reaches |−7,4|, stop and report the floor as the result
  (R6b). Do not propose more runs: replicates cannot resolve an effect smaller
  than the instrument's own noise.

**Execution profile.** Single repo. Reuses `run-batch.sh`; adds a mode, not a script.

## Product Contract

### Summary

Measure how much the RT-005 apparatus varies when the input does not. Report a
noise floor per source — sampling, agent, cache position — and write it into the
pre-registration's reading rules as the bound below which no contrast may be
called an effect.

### Problem Frame

The pre-registered analyser reports its own within-design control at **−7,4 over
56 runs**, and names in its own words what that means: those runs have
byte-identical inputs across arms, so their contrast *should* be noise.

The 7 unperturbed items carry only the item and the peer's answer. Between the
`fiable` and `degradee` arms those two are identical; the only difference is a
label the model never sees. A non-null contrast there comes from neither the item,
nor the peer, nor the injected confidence.

**Why it blocks scale-up.** RT-005's estimand is the *existence* of an
interaction — a sign flip. The batch returned "no flip" on both pre-registered
contrasts, which is a clean result. But a noise floor of this order changes how
that result reads: the question "above what gap is a contrast distinguishable
from an artefact?" has no measured answer. Going R=2→3 and 10→15 items would buy
precision on a quantity whose floor is unknown — 40 then 200 runs to tighten an
interval around a zero of unknown width.

### Key Decisions

- **Measure, do not fix.** This ticket produces a number and a reading rule. It
  changes no arm, item, or estimand. Governs R1, R5.
- **Decompose the floor by source.** A single aggregate floor would not let anyone
  act on it. Governs R2, R3.

### Requirements

**The measurement**

- R1. A null cell runs the 7 unperturbed items with the question held constant,
  varying only the suspected factors, and reports the dispersion of `out_tokens`.
- R2. The floor is reported **per source**, not as one aggregate: sampling,
  agent identity, position in batch.
- R3. The decomposition is identifying — each source's contribution is separable
  from the others by design, not by assumption.

**The consequence**

- R4. The measured floor is written into `PREREGISTRATION.md` as a dated amendment
  carrying a reading rule: below this bound, no RT-005 contrast may be reported as
  anything but noise.
- R5. The RT-005 scale-up gate is discharged or held explicitly, with the number
  as its reason.
- R6b. **The "floor is large" branch is stated, not left to judgement.** If the
  measured total floor is at or above |−7,4|, then the observed control contrast
  lies inside the apparatus's own noise. In that case the floor — not the RT-005
  contrast — is the ticket's primary result, the scale-up gate stays **held**, and
  the recommendation is an apparatus change (RT-005 v2), never more runs at the
  same design. More replicates cannot resolve an estimand smaller than the floor
  of the instrument measuring it.

**Bounds**

- R6. No RT-005 estimand, arm, item, or agent prior is modified.
- R7. The cell runs off the Claude quota, on the same z.ai engine as the pilot.

### Acceptance Examples

- AE1. The report gives three numbers with their spread, one per source, plus the
  total. A reader can say which source dominates. Covers R2, R3.
- AE2. A contrast of −7,4 is placed against the measured floor and the report says
  plainly whether it is inside it. Covers R1, R4.
- AE3. `PREREGISTRATION.md` carries a dated amendment with the bound and the rule.
  Its sha256 before and after is recorded. Covers R4.

### Sources

- Issue: `senara-solutions/mika#2117`
- `research/rt005-physics-pilot/orchestration/run-batch.sh` — `--seed`,
  `--replicates`, `--limit`, `--out-dir`, `agent_state_fingerprint()` at `:280`
- `crates/mika-agent/src/research/mechanism_analyzer.rs:70-73`, `:733` — the
  within-design control and the sentence that names it
- `research/rt005-physics-pilot/orchestration/PREREGISTRATION.md` — sealed;
  amendment 1 (2026-08-31) is the precedent for how an amendment is written
- Batch under discussion: `rt005-20260831`, 80/80, both contrasts `SAME DIRECTION`

## Planning Contract

### Key Technical Decisions

- KTD1. **Three sources, separated by design.** The issue lists three hypotheses
  and privileges none. A single "repeat everything N times" cell would confound
  them. The cell is therefore a 2×2 over `{same agent, different agent}` ×
  `{same position class, different position class}`, with replicates inside each:
  - within-agent, same position class → **sampling** floor alone;
  - across-agent, same position class → sampling **+ agent state**;
  - within-agent, across position class → sampling **+ cache warmth**.
  Each source is read as a difference between adjacent cells. Governs R2, R3.
- KTD2. **`run-batch.sh` gains a `--null-cell` mode; no second script.** The batch
  runner already owns the agent fingerprinting, the retry policy, the manifest,
  and the exclusion accounting. A parallel script would drift from it, and a
  measurement whose harness differs from the thing measured measures the harness.
- KTD3. **The metric is `out_tokens`, matching the estimand.** Amendment 1
  (2026-08-31) established that the pre-registered contrast reads on `out_tokens`,
  not on string equality. The floor must be measured on the same quantity it will
  bound, or it bounds nothing.
- KTD4. **Replicates are chosen from the measurement, not guessed.** Run a small
  pilot of the null cell first, read its spread, and derive N from it. A plan that
  fixes N up front picks a number with no relation to the dispersion it must
  resolve.

### High-Level Technical Design

```
7 unperturbed items, question HELD CONSTANT
                │
      ┌─────────┴─────────┐
      │                   │
  same agent          different agent
      │                   │
 ┌────┴────┐         ┌────┴────┐
same pos  diff pos  same pos  diff pos
  (A)       (C)       (B)       (D)

sampling floor      = spread(A)
agent contribution  = spread(B) − spread(A)
cache contribution  = spread(C) − spread(A)
interaction check   = spread(D) vs A+B+C   (if D departs, the sources are not additive
                                            and the report says so instead of summing)
```

### Assumptions

- The z.ai engine's sampling is stationary over the batch window. If cell A's
  spread drifts between its first and last replicate, that is itself a finding and
  is reported, not smoothed.
- `agent_state_fingerprint()` captures the state that matters. If two agents share
  a fingerprint but differ in dispersion, the fingerprint is incomplete — report it.

### Sequencing

U1 (mode) before U2 (pilot), U2 before U3 (full cell) because U2 sets N. U4
(analysis) after U3. U5 (amendment) last: it writes a number that must exist.

## Implementation Units

### U1. `--null-cell` mode in `run-batch.sh`

**Goal.** The batch runner can hold the question constant and vary only agent and
position.

**Requirements.** R1, R7.

**Files.** `research/rt005-physics-pilot/orchestration/run-batch.sh`;
`research/rt005-physics-pilot/orchestration/tests/test_run_batch.sh`.

**Approach.** Add `--null-cell` selecting the 7 unperturbed items and emitting the
2×2 of KTD1. Reuse the existing seed handling for position, and the existing
agent selection for identity. The manifest records `mode: null-cell`, the seed,
the agent fingerprints, and the item set, so a reader can tell a null-cell batch
from an RT-005 batch without opening a run.

**Test scenarios.** The mode selects exactly the 7 unperturbed items. The prompt is
byte-identical across all four cells for a given item — asserted by hashing the
prompt, not by reading it. The manifest carries `mode: null-cell`.

**Verification.** `research/rt005-physics-pilot/orchestration/tests/test_run_batch.sh`.

### U2. Pilot the null cell and derive N

**Goal.** Choose the replicate count from data.

**Requirements.** R1, KTD4.

**Files.** none — an execution producing a batch directory.

**Approach.** Run a small null cell (a few replicates per cell). Read the spread of
`out_tokens` in cell A. Derive N so the floor is resolved to a stated precision.
Write the derivation — the observed spread, the target precision, the resulting N —
into the batch's `PROVENANCE.md` **before** the full run, so N is not chosen after
seeing the answer.

**Test scenarios.** None; it is a measurement. Its guard is that the derivation is
written before U3 runs.

**Verification.** `PROVENANCE.md` exists, is dated, and predates the U3 batch.

### U3. Run the full null cell

**Goal.** The measurement itself.

**Requirements.** R1, R2, R7.

**Files.** none — a batch directory under `~/.mika/rt005/`.

**Approach.** Run the four cells at N. Record exclusions the way the RT-005 batch
does. On the z.ai engine, never the Claude quota.

**Test scenarios.** None. **Guard:** the run aborts if any prompt hash differs
across cells for the same item — that would mean the question did not stay
constant, and the whole cell would be void.

**Verification.** Run count matches the design; exclusions accounted; prompt-hash
guard passed.

### U4. Analyse and report the floor

**Goal.** Three numbers a reader can act on.

**Requirements.** R2, R3.

**Files.** `crates/mika-agent/src/research/mechanism_analyzer.rs`, or a sibling
reader beside it.

**Decision criterion, stated so it does not need judgement at implementation
time:** extend `mechanism_analyzer.rs` **if and only if** `load_batch` parses a
null-cell manifest without modification. If it needs any change to accept
`mode: null-cell`, write a sibling reader instead — because changing `load_batch`
would alter the loader that the sealed RT-005 analysis path depends on, and this
ticket must not touch that path (R6). Run `load_batch` against a null-cell
manifest fixture as the first step of this unit; its result decides the file, and
the result is recorded in the PR body.

**Approach.** Report spread per cell, then the three differences of KTD1, then the
additivity check. Place the observed −7,4 against the total and state plainly
whether it falls inside.

**Test scenarios.** A synthetic batch with a known injected dispersion is recovered
by the reader. **Negative control:** a synthetic batch with zero dispersion must
report a floor of zero — a reader that finds structure in constant data is
measuring itself.

**Verification.** `cargo test -p mika-agent research`.

### U5. Amend the pre-registration with the bound

**Goal.** The number becomes a rule.

**Requirements.** R4, R5.

**Files.** `research/rt005-physics-pilot/orchestration/PREREGISTRATION.md`.

**Approach.** Append a dated amendment, in the form of amendment 1 (2026-08-31):
the measured floor per source, the total, and the reading rule — below this bound,
no RT-005 contrast is reported as anything but noise. Record the file's sha256
before and after. Then state whether the scale-up gate is discharged or held, and
why, in the ticket.

**Test scenarios.** None.

**Verification.** sha256 recorded both sides; the amendment is dated and additive
(no earlier text altered).

## Verification Contract

- `research/rt005-physics-pilot/orchestration/tests/test_run_batch.sh` passes.
- `cargo test -p mika-agent research` passes, including U4's negative control.
- The prompt-hash guard (U3) is demonstrated to fire: run it once against a
  deliberately altered prompt and show it aborts. A guard never seen to refuse is
  not known to work.
- `shellcheck` clean on `run-batch.sh`.

## Definition of Done

**Global.**
- R1–R7 satisfied, each traced to a landed unit.
- The floor is reported per source, with the additivity check stated.
- `PREREGISTRATION.md` carries the dated amendment; sha256 recorded before/after.
- The RT-005 scale-up gate is explicitly discharged or held, with the number as
  its stated reason.
- No RT-005 estimand, arm, item, or prior changed — verified by an empty diff on
  the pre-registration's estimand section and on the agent prior files.
- Nothing ran on the Claude quota.

**Per unit.** Each unit's Verification passes, and U3's prompt-hash guard and U4's
negative control are both demonstrated, not merely present.
