---
title: Dispatch gate strict-matches "second-pass (GROOMED)", rejects spec-tolerated paraphrased verdicts
date: 2026-05-14
category: logic-errors
module: skills/executor
problem_type: logic_error
component: tooling
symptoms:
  - "Autonomous dispatch hangs in blocked state after grooming produces spec-tolerated paraphrased verdict"
  - "mika-dev writes 'Engine gate requires exactly second-pass (GROOMED)' to session messages but does not surface to operator"
  - "Operator must manually edit issue body to substitute literal (GROOMED) for paraphrased form"
root_cause: logic_error
resolution_type: code_fix
severity: high
tags:
  - dispatch-gate
  - grooming-marker
  - autonomous-loop
  - validate-dispatch-readiness
  - spec-drift
---

# Dispatch gate strict-matches "second-pass (GROOMED)", rejects spec-tolerated paraphrased verdicts

## Problem

The autonomous-loop dispatch gate in `validate_dispatch_readiness()` performed a literal substring check for `second-pass (GROOMED)` in the issue body's Grooming-history callout. When the grooming spec's Phase 5 emitted the spec-tolerated paraphrase `second-pass (READY, paraphrased GROOMED per spec tolerance)`, the gate rejected it. Implementation never dispatched, and the operator had to manually edit the issue body to unblock.

## Symptoms

- Host task stuck in `blocked` state with no visible error in operator-facing surfaces (`gh issue view`, `mika tasks list`)
- mika-dev consumed the `ready` label, stripped it, created the host task, but no claude-pilot subprocess spawned
- Only DB-level inspection of mika-dev session messages revealed the "Engine gate requires exactly `second-pass (GROOMED)`" rejection reason

## What Didn't Work

- The first groomed plan (superseded `2026-05-14-001`) went in the wrong direction — it proposed hardening the spec to only emit canonical `(GROOMED)`, which contradicted the issue's acceptance criteria requiring the gate to accept the paraphrase form
- The wrong-direction plan was produced by an autonomous-loop dev-groom that fabricated success without calling mika-arch (surfaced by mika#1097 dogfooding)

## Solution

Broadened `check_grooming_markers()` to accept both shapes via an OR match:

```rust
// Before (#919 — strict literal match)
if !issue_body.contains("second-pass (GROOMED)") {
    missing.push("groomed_verdict");
}

// After (#1108 — accepts canonical + spec-tolerated paraphrase)
let has_groomed_marker = issue_body.contains("second-pass (GROOMED)")
    || issue_body.contains("second-pass (READY, paraphrased GROOMED");
if !has_groomed_marker {
    missing.push("groomed_verdict");
}
```

Also added `record_dispatch_rejection()` — a fire-and-forget helper that writes structured rejection reasons to `tasks.result` at all 7 dispatch-rejection sites, so the operator sees the failure reason without DB-level inspection.

## Why This Works

The root cause was a **spec-gate drift** — the grooming spec (dev-groom Phase 5) authorized two callout shapes, but the dispatch gate (engine-level `check_grooming_markers()`) only accepted one. The fix aligns the gate with the spec by accepting everything the spec authorizes.

The broadening is bounded: bare `second-pass (READY)` without the "paraphrased GROOMED" qualifier is still rejected. The negative test enforces this boundary.

## Prevention

- When adding engine-level gates that validate spec-produced output, enumerate all output shapes the spec authorizes — not just the canonical one
- The `check_grooming_markers()` function and the prompt-level check at `skills/bundled/self-dev/system_prompt.md:253` are a coupled pair — both must update if the callout shape changes
- The `record_dispatch_rejection()` pattern (write rejection reasons to `tasks.result`) ensures future dispatch-gate rejections are visible to operators without requiring DB-level inspection

## Related Issues

- mika#1108 — this fix
- mika#1097 — the ticket that surfaced this (dev-groom early-exit; separate bug)
- mika#919 — original grooming-marker gate implementation
- mika#907 — grooming pre-flight check
- `docs/solutions/workflow-issues/dev-groom-zero-artifact-exit-2026-05-13.md` — compound covering the family of dev-groom/dispatch failure shapes
