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

This doc is the auditable record for the empirical-first plan in mika#766.
The plan structure is fixed by mika-platform doctrine: §1 (pre-registration)
is written before measurement and **must not be edited** after the commit
that locks it; §2 (measurement) and §3 (disposition) are written after the
eval runs.

The discipline exists because retroactively touching §1 after the numbers
exist collapses pre-registration into post-hoc rationalization. Git history
is what makes "willing to be told no" auditable.

## §1 — Pre-registration (immutable after this commit)

§1 was reviewed by Mika Prime and stress-tested by an external peer review
(via mika-ask-a-friend). Two structural errors in earlier drafts were
caught before lock:

- An algebraic identity: `net_delta == net_flips` under boolean per-case
  correctness. The original two-axis decision rule was a single-axis rule
  wearing two hats; the "redistribution of wrongness" guard was the empty
  set and could never fire.
- A noise-model rationale (LLM nondeterminism + selection bias as
  asserted-but-unmeasured) was reseated to a **complexity-asymmetry**
  rationale: the disputable case is the only measured noise source in the
  current fixture; the threshold sits where it sits because adding a
  permanent maintenance surface needs a higher bar than keeping the
  working incumbent.

The §1 below is the post-review shape.

### Pre-flight gate

**Harness measures outcome, not mechanism — PASSED.**

The harness at `crates/mika-agent/tests/eval/kg_provider_eval/truncation_eval.rs`
runs each fixture case through the production KG resolution disambiguation
prompt twice — once with byte-boundary `safe_truncate`, once with
`truncate_at_semantic_boundary` (the impl under review) — and scores the
LLM's returned `matched` entity against ground truth via boolean
`is_correct_match` (truncation_eval.rs:338).

The measured quantity is **downstream disambiguation accuracy**, not
boundary adherence. A mechanism-only gate would be tautological — semantic
truncation always respects sentence boundaries by definition. The outcome
gate proves the mechanism causes a *quality* lift on the task the
production system actually performs.

### Hypothesis

**H1:** Semantic-boundary truncation produces measurably better
disambiguation accuracy than byte-boundary truncation on the 10 fixture
cases at the eval's 500-byte budget.

**H0 (null):** No measurable difference; the two truncation strategies
produce equivalent disambiguation outcomes.

### Fixture

`docs/solutions/kg/eval-fixtures-2026-04-24/truncation_eval_contexts.toml`
— 10 cases drawn from realistic `docs/solutions/` prose:

- 7 `clear_match` cases (ground truth is one specific candidate)
- 3 `no_match` cases (ground truth is "none of the candidates fit")
- 9 non-disputable + 1 disputable

Eval truncation budget: **500 bytes** (vs production's 2000) —
intentionally lowered to guarantee truncation triggers on every case.

Note: the mika#766 plan body referenced ~20 cases. The actual fixture has
10. The discrepancy is documented here so §2 cannot introduce it
post-hoc.

### Locked metric

**The decision rule is single-axis under the algebraic identity.**

Per-case correctness is boolean (truncation_eval.rs:148-149); `flipped` is
exactly the diff between the two booleans (truncation_eval.rs:151-155).
Therefore:

```
d = total_semantic_correct - total_byte_correct
  = semantic_wins - byte_wins
```

The two definitions are arithmetically identical (truth table walked in
the peer review; truthfully verified against the harness code at the
SHA-pinned commit). The "redistribution of wrongness" guard from earlier
drafts (`net_delta ≥ 2 AND net_flips < 0`) is the empty set and is
dropped.

**Single metric: `d` over the 10 fixture cases.**

**Informational only — not load-bearing:** `mean_confidence_delta`. LLM
self-reported confidence is calibration, not correctness; allowing it to
break a tie would let the model rule on its own change. The decision rule
is monotone in `d`, no confidence-delta tie-breaker.

### Provider lock

The `d` statistic and the per-case agreement count below are computed for
**one provider**, matching production:

```
MIKA_EVAL_KG_PROVIDERS=openrouter/google/gemini-2.5-flash-lite
```

This is the production `MIKA_KG_RESOLUTION_MODEL` value (`~/.mika/.env`).
The eval is load-bearing on production only when it tests the production
model. Other providers (haiku/sonnet/deepseek/kimi from the default set)
are NOT included in §1's locked run — they would muddy `d` by mixing
provider variance into a single statistic.

Other configuration locked into §1:

- Temperature: provider default (deterministic for the locked model; not
  overridden by the harness)
- Seed: not exposed by the LLM API for this model; this is the
  irreducibly unfrozen input (see honesty clause below)

**Honesty clause:** the provider is the irreducibly unfrozen input. A
server-side model update under the same name (Google releasing a new
`gemini-2.5-flash-lite` revision) invalidates the run, and no SHA pin
catches it. §2 records the run date so this drift is at least visible to
future readers; it cannot be prevented.

### Two-run confirmatory protocol

A single run is exposed to LLM nondeterminism on borderline cases. The
unmeasured assumption "nondeterminism affects at most 1-2 cases" is
converted into a **measurement**: run the eval twice and check per-case
agreement.

Agreement axis is **per-case outcome tuples** `(byte_correct,
semantic_correct)`, NOT aggregate `d` value. Two runs can produce the same
`d` with different winning cases — `d`-stability is a derived quantity;
per-case stability is the underlying claim.

`compute_per_case_agreement()` in truncation_eval.rs aligns outcomes by
`(provider, entity_key)` and counts cases where the tuples match.

### Decision rule (immutable)

Three gates, all visible in the disposition line in §2 (per
`print_two_run_comparison`):

| Gate | Test | If fails → |
|---|---|---|
| Agreement | `agreement_count ≥ 8` (of 10) | Revert (inability-to-measure) |
| Lift (run 1) | `d_run1 ≥ 3` | Revert |
| Lift (run 2) | `d_run2 ≥ 3` | Revert |

**Stay iff all three gates pass. Otherwise Revert.**

Short-circuit ordering in the harness: agreement first, then `d` on both
runs. Disposition line MUST show every gate's `✓`/`✗` evaluation, not just
the firing path (mechanical-disposition property is verifiable only when
the full evaluation is visible).

**Why d ≥ 3 on 10 cases — complexity-asymmetry rationale:**

Semantic truncation adds a permanent maintenance surface
(`truncate_at_semantic_boundary` + every call site that switched to it)
on top of a working incumbent (`safe_truncate`). The bar to **add**
complexity sits above the bar to **keep** simple. At n=10, nothing
reaches statistical significance — binomial p<0.10 would require ≈ 7 net
wins, which the fixture cannot produce. The threshold is a
**judgment-floor**, not a significance test.

`d = 2` with a disputable case in the pool is `d = 1` on the
non-disputable subset; on a 10-case fixture that's couldn't-tell. The
empirical-first response to couldn't-tell is **don't add complexity**.
`d ≥ 3` absorbs the disputable case and leaves a 2-case margin of
non-disputable signal as the floor of "this is real."

The other noise sources that earlier drafts named (LLM nondeterminism,
fixture selection bias) are NOT in this rationale: nondeterminism is
operationally measured by the two-run protocol above; selection bias is
acknowledged as unmeasured exposure but does not load-bear on the
threshold.

### Asymmetry the decision rule encodes

`d ≥ 3` makes false-revert (rejecting a real lift) more likely than a
weaker threshold would. Under empirical-first this is a feature, not a
bug. The asymmetry favors the higher floor because:

- **False-stay = sticky complexity debt.** The function ships, nobody
  later notices it's not pulling weight, the permanent maintenance
  surface stays. Silent.
- **False-revert = reopenable backlog.** Harness + fixtures + this doc
  are retained (per the revert clause); mika#766 reopens against a bigger
  fixture or a different model. Visible.

Sticky-and-silent vs visible-and-reopenable favors the higher floor.

### Revert clause (immutable scope)

If §3 returns **Revert**, the revert is **impl-only**:

**Revert:**
- `crates/mika-common/src/text.rs::truncate_at_semantic_boundary` (the
  function added in PR #1556)
- All call sites that switched from `safe_truncate` to
  `truncate_at_semantic_boundary` in
  `crates/mika-agent/src/kg/entity_resolver.rs` and
  `crates/mika-agent/src/kg/subject_extractor.rs`

**Keep:**
- The eval harness
  (`crates/mika-agent/tests/eval/kg_provider_eval/truncation_eval.rs`),
  including the two-run protocol and `print_two_run_comparison`
- The fixture
  (`docs/solutions/kg/eval-fixtures-2026-04-24/truncation_eval_contexts.toml`)
- This decision doc (this commit + the Commit B that fills §2/§3 both
  stay in history)
- The companion raw-output doc (sibling file, see §2)

The harness and fixtures are reusable infrastructure for any future
KG-prompt-quality decision. Reverting them would force re-implementation
if the question is revisited.

### Stay clause (immutable scope)

If §3 returns **Stay**, no further code changes. PR #1556 ships as-is
and mika#766 closes with a `Closes #766` clause referencing this doc as
the empirical record.

### Tree-pin (provenance lock)

§2 runs against the tree at the commit containing this §1. Any change
to anything in that tree — fixture content, harness scoring at
truncation_eval.rs:148-155, `print_two_run_comparison`, the truncation
functions themselves, this doc above the §2 marker — voids §1 and
requires re-registration with a new dated decision doc.

The pin is on the tree, not on enumerated files, because enumerating
invites litigation about whether a change is in-scope. Git records which
commit a file lives in; readers of §1 in §2's commit can recover the
locked SHA via `git log -- docs/solutions/kg/truncation-decision-2026-06-26.md | head -1`.

### §2 commit shape (pre-specified to keep §2 mechanical)

§2 (the Commit B that fills measurement and disposition) MUST contain:

1. Both runs' per-case outcome tables (10 rows each, columns:
   `entity_key`, `byte_correct`, `semantic_correct`).
2. Per-run `d` value: `d_run1 = …`, `d_run2 = …`.
3. Per-case agreement count: `agreement = …/10`.
4. The mechanical disposition line emitted by `print_two_run_comparison`,
   **showing every gate's `✓`/`✗` evaluation**, not just the firing path.
   Examples:
   - `d_run1=3, d_run2=3, agreement=9/10 → d≥3 on both runs ✓ AND agreement≥8 ✓ → Stay`
   - `d_run1=2, d_run2=3, agreement=9/10 → agreement≥8 ✓ AND d≥3 on both runs ✗ → Revert`
   - `d_run1=3, d_run2=4, agreement=6/10 → agreement≥8 ✗ → Revert (inability-to-measure)`
5. Run date and time for both runs (model-revision drift visibility per
   the honesty clause).

§3 is the one-word disposition keyword (`Stay` or `Revert`) plus a
one-paragraph follow-through naming any artifacts created or removed by
the disposition (e.g., the revert commits referenced if Revert).

**No interpretation in §2.** Whoever writes §2 cannot re-derive the
disposition — they apply §1's rule. Pre-registration's protection only
holds if the application is mechanical.

### Re-test trigger (immutable)

This decision binds for the current production state (KG resolution
prompt as of mika#1158, fixture as of 2026-06-26, provider
`openrouter/google/gemini-2.5-flash-lite`). The decision MUST be re-run
if any of these change materially:

- KG resolution disambiguation prompt (`subject_extractor.rs::EXTRACTION_PROMPT`
  or its resolver counterpart)
- Production truncation budget (`crates/mika-common/src/text.rs`
  constants used by the resolver call site)
- Fixture set (additions, removals, or ground-truth revisions)
- Provider lock (`MIKA_KG_RESOLUTION_MODEL` changes in production)

A re-run produces a new dated decision doc; this one stays in history as
the record of the original answer.

## §2 — Measurement (filled in Commit B; do not edit §1 in the same commit)

*Pending — will be filled after running:*

```bash
MIKA_EVAL_KG_PROVIDERS=openrouter/google/gemini-2.5-flash-lite \
  cargo test -p mika-agent --test eval truncation_eval -- --ignored --nocapture
```

Sibling raw-output doc:
`docs/solutions/kg/truncation-quality-comparison-2026-06-26.md`

## §3 — Disposition (filled in Commit B; do not edit §1 in the same commit)

*Pending — will be derived from §2 against the decision rule in §1.*
