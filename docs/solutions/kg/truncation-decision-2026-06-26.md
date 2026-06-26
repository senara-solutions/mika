---
title: KG semantic-truncation empirical decision (mika#766)
date: 2026-06-26
issue: mika#766
pr: mika#1556
status: pre-registered
problem_type: empirical_decision
category: kg
tags:
  - kg
  - truncation
  - empirical-first
  - pre-registration
---

# KG semantic-truncation empirical decision (mika#766)

This doc is the auditable record for the empirical-first plan in mika#766. The
plan structure is fixed by mika-platform doctrine: §1 (pre-registration) is
written before measurement and **must not be edited** after Commit A; §2
(measurement) and §3 (disposition) are written after the eval runs.

The discipline exists because retroactively touching §1 after the numbers exist
collapses pre-registration into post-hoc rationalization. Git history is what
makes "willing to be told no" auditable.

## §1 — Pre-registration (immutable after Commit A)

### Pre-flight gate

**Harness measures outcome, not mechanism — PASSED.**

The harness at `crates/mika-agent/tests/eval/kg_provider_eval/truncation_eval.rs`
runs each fixture case through the production KG resolution disambiguation
prompt twice — once with `safe_truncate` (byte-boundary) and once with
`truncate_at_semantic_boundary` (the impl under review) — and scores the LLM's
returned `matched` entity against ground truth.

The measured quantity is **downstream disambiguation accuracy** (the LLM picked
the right candidate), not boundary adherence (did truncation land on a sentence
edge). A mechanism-only gate would be tautological — semantic truncation always
respects sentence boundaries by definition. The outcome gate proves the
mechanism causes a *quality* lift on the task the production system actually
performs.

### Hypothesis

**H1:** Semantic-boundary truncation produces measurably better disambiguation
accuracy than byte-boundary truncation on the 10 fixture cases at the eval's
500-byte budget.

**H0 (null):** No measurable difference; the two truncation strategies produce
equivalent disambiguation outcomes.

### Fixture

`docs/solutions/kg/eval-fixtures-2026-04-24/truncation_eval_contexts.toml` —
10 cases drawn from realistic `docs/solutions/` prose:

- 7 `clear_match` cases (ground truth is one specific candidate)
- 3 `no_match` cases (ground truth is "none of the candidates fit")
- 9 non-disputable + 1 disputable (case where ground truth is itself defensibly
  ambiguous; flagged so a swing on this single case doesn't dominate the verdict)

Eval budget: 500 bytes (vs production's 2000) — intentionally lowered to
guarantee truncation triggers on every case.

Note: the mika#766 plan body referenced ~20 cases. The actual fixture has 10.
This is documented as a discrepancy here so §2 doesn't introduce it as
post-hoc.

### Metric Y (primary)

```
net_accuracy_delta = total_semantic_correct - total_byte_correct
```

Where `total_*_correct` is the count over the 10 cases of `is_correct_match`
returning `true` for that strategy (per the harness's existing scoring at
`truncation_eval.rs:338`).

### Metric Y' (supplementary — guards against single-direction wins)

```
net_flips = total_flipped_semantic_wins - total_flipped_byte_wins
```

Where `total_flipped_semantic_wins` counts cases where byte was wrong **and**
semantic was right, and `total_flipped_byte_wins` counts the inverse.

A positive `net_accuracy_delta` with a negative `net_flips` would indicate that
semantic truncation regressed some real cases while winning others — that's not
a clean lift; it's a redistribution of wrongness. `net_flips ≥ 0` is required
to ship.

### Metric Y'' (informational only)

```
mean_confidence_delta = (Σ over cases of (semantic_confidence - byte_confidence)) / n
```

Reported in §2 for context. **Not load-bearing for the disposition** — LLM
confidence calibration is not the same signal as resolution correctness, and
the existing in-harness gate's reliance on it (line 266: `> 0.05`) is the
loose-threshold gap this pre-registration tightens.

### Decision rule (immutable)

| `net_accuracy_delta` | `net_flips` | Disposition |
|---|---|---|
| `≥ 2` | `≥ 0` | **Stay** — impl ships; close mika#766 with PR #1556 |
| `= 1` | any | **Revert** — ambiguous defaults to revert (empirical-first preference) |
| `≤ 0` | any | **Revert** — null hypothesis not rejected |
| `≥ 2` | `< 0` | **Revert** — redistribution of wrongness, not clean lift |

**Why `≥ 2` on 10 cases:** A 20% relative accuracy lift is the smallest
threshold where a single disputable-case swing cannot flip the verdict. With
one fixture flagged disputable, a `= 1` lift could be entirely due to that
case — empirical-first preference treats that as "not measurably better."

**Why not lean on `mean_confidence_delta`:** The built-in gate at
`truncation_eval.rs:266` accepts the impl when `flipped_semantic_wins > 0 OR
mean_confidence_delta > 0.05`. That OR-shape lets confidence-only wins (no
accuracy improvement) ship. This pre-registration rejects that — confidence
without accuracy is calibration drift, not quality.

### Revert clause (immutable scope)

If §3 returns **Revert**, the revert is **impl-only**:

**Revert:**
- `crates/mika-common/src/text.rs::truncate_at_semantic_boundary` (the function
  added in PR #1556)
- All call sites that switched from `safe_truncate` to
  `truncate_at_semantic_boundary` in `crates/mika-agent/src/kg/entity_resolver.rs`
  and `crates/mika-agent/src/kg/subject_extractor.rs`

**Keep:**
- The eval harness (`crates/mika-agent/tests/eval/kg_provider_eval/truncation_eval.rs`)
- The fixtures (`docs/solutions/kg/eval-fixtures-2026-04-24/truncation_eval_contexts.toml`)
- This decision doc (Commit A + Commit B both stay in history)
- The companion raw-output doc (sibling file, see §2)

Rationale: the harness and fixtures are reusable infrastructure for any future
KG-prompt-quality decision. Reverting them would force re-implementation if the
question is revisited. The decision doc and its history are the durable record
of how the question was answered.

### Stay clause (immutable scope)

If §3 returns **Stay**, no further code changes. PR #1556 ships as-is and
mika#766 closes with a `Closes #766` clause in the PR body referencing this
doc as the empirical record.

### Re-test trigger (immutable)

This decision binds for the current production state (KG resolution prompt as
of mika#1158, fixture as of 2026-06-26). The decision MUST be re-run if any
of these change materially:

- KG resolution disambiguation prompt (`subject_extractor.rs::EXTRACTION_PROMPT`
  or its resolver counterpart)
- Production truncation budget (`crates/mika-common/src/text.rs` constants used
  by the resolver call site)
- Fixture set (additions, removals, or ground-truth revisions)

A re-run produces a new dated decision doc; this one stays in history as the
record of the original answer.

## §2 — Measurement (filled in Commit B; do not edit §1 in the same commit)

*Pending — will be filled after running:*

```bash
MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval truncation_eval -- --ignored --nocapture
```

Sibling raw-output doc: `docs/solutions/kg/truncation-quality-comparison-2026-06-26.md`

## §3 — Disposition (filled in Commit B; do not edit §1 in the same commit)

*Pending — will be derived from §2 against the decision rule in §1.*
