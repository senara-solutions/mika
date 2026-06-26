---
title: KG semantic-truncation eval raw output (mika#766)
date: 2026-06-26
issue: mika#766
pr: mika#1556
problem_type: eval_output
category: kg
tags:
  - kg
  - truncation
  - eval-output
---

# KG semantic-truncation eval raw output (mika#766)

Companion doc to
[`truncation-decision-2026-06-26.md`](truncation-decision-2026-06-26.md).
Captures the raw output of the eval run that produced §2's numbers, so
the per-case detail is preserved even if the harness output format
changes later.

The decision doc is the auditable record (§1 pre-registration, §2
measurement, §3 disposition); this doc is the supporting evidence.

## Eval invocation

```bash
git rev-parse HEAD   # 967f165d05a76ce65b5b4f17c204ce8bea8463ed (Commit A-prime)
date -Iseconds       # 2026-06-26T13:47:49+02:00 (Run 1 start)
MIKA_EVAL_KG_PROVIDERS=openrouter/google/gemini-2.5-flash-lite \
  cargo test -p mika-agent --test eval truncation_eval -- --ignored --nocapture
```

## Run 1 — per-case outcomes (10 cases)

| Case | Byte correct | Semantic correct | Flipped | Conf Δ |
|---|---|---|---|---|
| `problem_type:state_drift` | ✓ (0.95) | ✓ (0.95) | — | +0.000 |
| `skill:self_dev` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |
| `tool:pr_merge_with_gate` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |
| `pattern:two_layer_deploy_guard` | ✗ (0.90) | ✗ (0.95) | — | +0.050 |
| `agent:mika_arch` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |
| `problem_type:fabrication` | ✓ (1.00) | ✓ (0.95) | — | -0.050 |
| `concept:dispatch_slot` | ✓ (0.00) | ✓ (0.00) | — | +0.000 |
| `skill:qa_review` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |
| `failure_mode:context_leak` | ✓ (0.00) | ✓ (0.00) | — | +0.000 |
| `tool:resolve_issue_order` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |

Totals: byte 9/10, semantic 9/10, 0 flips, mean Δ confidence ≈ 0.
**d (run 1) = 0.**

## Run 2 — per-case outcomes (10 cases)

| Case | Byte correct | Semantic correct | Flipped | Conf Δ |
|---|---|---|---|---|
| `problem_type:state_drift` | ✓ (1.00) | ✓ (0.95) | — | -0.050 |
| `skill:self_dev` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |
| `tool:pr_merge_with_gate` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |
| `pattern:two_layer_deploy_guard` | ✗ (0.95) | ✗ (0.95) | — | +0.000 |
| `agent:mika_arch` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |
| `problem_type:fabrication` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |
| `concept:dispatch_slot` | ✓ (0.00) | ✓ (0.00) | — | +0.000 |
| `skill:qa_review` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |
| `failure_mode:context_leak` | ✓ (0.00) | ✓ (0.00) | — | +0.000 |
| `tool:resolve_issue_order` | ✓ (1.00) | ✓ (1.00) | — | +0.000 |

Totals: byte 9/10, semantic 9/10, 0 flips, mean Δ confidence ≈ -0.005.
**d (run 2) = 0.**

## Per-case agreement (Run 1 vs Run 2)

| Case | Run 1 (B,S) | Run 2 (B,S) | Agreed |
|---|---|---|---|
| `problem_type:state_drift` | (T,T) | (T,T) | yes |
| `skill:self_dev` | (T,T) | (T,T) | yes |
| `tool:pr_merge_with_gate` | (T,T) | (T,T) | yes |
| `pattern:two_layer_deploy_guard` | (F,F) | (F,F) | yes |
| `agent:mika_arch` | (T,T) | (T,T) | yes |
| `problem_type:fabrication` | (T,T) | (T,T) | yes |
| `concept:dispatch_slot` | (T,T) | (T,T) | yes |
| `skill:qa_review` | (T,T) | (T,T) | yes |
| `failure_mode:context_leak` | (T,T) | (T,T) | yes |
| `tool:resolve_issue_order` | (T,T) | (T,T) | yes |

**Agreement: 10/10.**

## Mechanical disposition (verbatim from harness output)

```
d_run1=0, d_run2=0, agreement=10/10 → agreement≥8 ✓ AND d≥3 on both runs ✗ → Revert
```

## Observations (informational, not load-bearing for §3)

- **Both runs were fully reproducible** — all 10 per-case outcomes
  matched between runs. The unmeasured-nondeterminism-assumption that
  motivated the two-run protocol was validated (zero divergence
  observed; the protocol's threshold of ≤2 case divergence was not
  approached).
- **Semantic and byte truncation produced identical outcomes on every
  case in both runs.** Zero flips in either direction. The model's
  answer was insensitive to which truncation strategy was used.
- **The single failing case** (`pattern:two_layer_deploy_guard`) failed
  under both byte and semantic truncation in both runs. The failure is
  an LLM-side limit at this model+budget, not a truncation-side
  failure mode. Increasing the byte budget back toward production's
  2000 would likely change this — but that is a separate experiment
  not in §1's scope.
- **Confidence deltas were negligible** (mean |Δ| < 0.01 in run 1,
  -0.005 in run 2). Even if confidence-delta had been a load-bearing
  signal in §1, it would not have crossed the prior 0.05 OR-gate
  threshold here.
- **Provider parity with production:**
  `openrouter/google/gemini-2.5-flash-lite` matches
  `MIKA_KG_RESOLUTION_MODEL` from the operator's `~/.mika/.env`. The
  eval is load-bearing on the production model the resolution call
  actually runs against.
