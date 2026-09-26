---
title: Extend structural verdict handler to cover block[*]/hold[*] dispatch paths
date: 2026-04-29
category: architecture-patterns
module: mika-agent
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - Adding new verdict types or dispatch paths to the structural verdict handler
  - Extending any pre-LLM webhook handler with bounded retry semantics
  - The LLM dismisses structured event tokens as "comments" based on GH review.state
tags:
  - verdict-handler
  - structural-handler
  - dispatch-table
  - bounded-retry
  - state-machine
  - pre-digest
  - webhook-handler
related_issues:
  - 524
  - 889
  - 888
---

# Extend structural verdict handler to cover block[*]/hold[*] dispatch paths

## Context

The structural verdict handler from mika#524 (2026-04-13) deterministically handled `VERDICT: pass` (merge via `pr_merge_with_gate`) but explicitly left `block[*]` and `hold[*]` verdicts to LLM judgment. This carve-out failed in production: PR mika#888 received `state=COMMENTED` + `VERDICT: block[ac]` from mika-qa with two real unsatisfied ACs. mika-dev's LLM gated on GH `review.state` ("COMMENTED" != "CHANGES_REQUESTED"), classified the verdict as "a comment, not a formal verdict," and went idle. The PR sat with unfixed ACs until manual operator intervention.

This was the same antipattern family as the original mika#522 misclassification: deterministic state-machine transitions implemented as LLM judgment over webhook payload text.

## Guidance

### Make verdict authoritative regardless of GH review.state

The `VERDICT:` token in the review body is the authority, not `review.state`. The body-as-truth contract (per `docs/skills.md` and mika#487) means:

- `VERDICT: pass` still gates on `state=approved` for merge safety (merge requires GitHub approval)
- All other verdicts (`block[*]`, `hold[*]`, missing) dispatch regardless of state

```rust
match verdict {
    Verdict::Pass => {
        // Only gate on state for merge safety
        if event.state != "approved" {
            return VerdictAction::Passthrough { enrichment: None };
        }
        handle_pass_verdict(...)
    }
    Verdict::Block(reason) => match reason.to_lowercase().as_str() {
        "ac" => handle_block_ac(...),    // state-independent
        "ci" => handle_block_ci(...),    // state-independent
        "security" | "pipeline" => handle_escalate(...),
        _ => VerdictAction::Passthrough { enrichment: None }, // unknown subtypes
    },
    Verdict::Hold(reason) => ...,
    Verdict::Missing { truncated } => ...,
}
```

### Use bounded retry counters for auto-dispatchable verdicts

`block[ac]` and `block[ci]` trigger claude-pilot dispatch, which creates an unbounded loop risk: a fix that satisfies some ACs but surfaces new ones creates an infinite retry cycle. Bounded retry counters stored in `task.metadata` JSON prevent this:

```rust
const BLOCK_AC_MAX_RETRIES: u32 = 3;
const BLOCK_CI_MAX_RETRIES: u32 = 3;  // separate constants — future calibration may diverge

let count = read_verdict_retry_count(&task.metadata, "verdict_block_ac");
if count >= BLOCK_AC_MAX_RETRIES {
    // Escalate: mark task blocked, notify operator
} else {
    // Dispatch: increment counter, dispatch claude-pilot
}
```

Separate constants for AC vs CI (even though both are 3) because the optimal retry budget may diverge: CI failures are typically auto-fixable (allow more retries), while AC failures depend on plan quality (stricter cap may be appropriate).

### Classify verdict subtypes into dispatch vs escalation

Not all verdicts warrant autonomous action:

| Verdict | Action | Rationale |
|---------|--------|-----------|
| `block[ac]` | Auto-dispatch claude-pilot | ACs are concrete, actionable |
| `block[ci]` | Auto-dispatch claude-pilot | CI failures are typically auto-fixable |
| `block[security]` | Escalate to operator | Security requires human judgment |
| `block[pipeline]` | Escalate to operator | Pipeline config is operator-owned |
| `hold[review]` | Notify operator, leave in_progress | Operator decides next move |
| missing/unparseable | Safe-default hold[review] | Never silently dismiss |

### Pre-digest all verdict actions for the LLM

Every verdict path returns `VerdictAction::Handled { pre_digest }` with a `<verdict_handler>` XML block. The pre-digest:
- States the action taken as a fait accompli
- Includes explicit "Do NOT dispatch" or "Do NOT call" instructions
- Avoids completion-claim guard trigger words (merged, deployed, completed, shipped)
- Uses parallel structure across all verdict types

### AC extraction with explicit fallback contract

The handler extracts unsatisfied ACs from mika-qa's structured verdict body using regex on `[❌] unsatisfied:` lines. When extraction yields zero matches (malformed body, different format):
- Fall back to first 2000 chars of body as-is
- Log structured `verdict_ac_extraction_fallback` event
- Mark the pre-digest with `[ac-extraction-fallback: true]`

This ensures the dispatch always has content for the child session, even when the parser fails.

## Why This Matters

- **LLMs gate on metadata, not semantic content.** When a review has `state=COMMENTED`, the LLM infers "this is a comment, not a verdict" — regardless of what the body says. Only engine-level parsing of the `VERDICT:` token is reliable.
- **Unbounded retry loops are expensive.** Without a counter, block[ac] + auto-fix + new-findings creates infinite loops. The bounded counter (3 attempts) gives room for legitimate iteration while preventing runaway spend.
- **Silent dismissal is worse than false escalation.** The safe-default for missing/unparseable verdicts is hold[review] (operator notification), not passthrough (LLM decides). False escalation costs one notification; silent dismissal leaves PRs blocked indefinitely.

## When to Apply

- Adding a new verdict subtype to the dispatch table (e.g., `block[test]`)
- Adding bounded retry semantics to any pre-LLM webhook handler
- Any case where the LLM is interpreting a structured event token instead of the engine handling it deterministically

## Examples

**Before (mika#524):** `block[*]` and `hold[*]` passed through to LLM — LLM dismissed `VERDICT: block[ac]` as "a comment, not a formal verdict" based on `state=COMMENTED`.

**After (mika#889):** Every parseable `VERDICT:` token maps to a deterministic engine action. The LLM receives a pre-digested fait accompli, never raw webhook text for verdict-class events.

## Related

- `docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md` — original #524 handler design (updated with #889 dispatch table)
- `docs/solutions/architecture-patterns/structural-ci-failure-handler-dispatch-2026-04-20.md` — CI failure handler with same bounded-retry pattern
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — general principle: engine guards > prompt rules
- mika#524 — original pass → merge handler
- mika#889 — this extension
- mika#888 — canonical reproduction (block[ac] dismissed)
- mika#864 — required_suffix_lines enforcement at emission
