---
title: "INTENT_GUARD trigger predicates must share the tool-boundary allowlist"
date: 2026-05-14
category: architecture-patterns
module: agent-loop
problem_type: bug_fix
component: tooling
severity: medium
applies_when:
  - Adding or modifying an INTENT_GUARD trigger predicate
  - The same domain surface (e.g., [GitHub] webhook prefix) has both a pre-hoc tool-boundary gate and a post-hoc EndTurn guard
  - A guard fires false-positive corrections on turns handled by a different skill
tags:
  - intent-precondition
  - guard-registry
  - endturn-guard
  - webhook-guard
  - predicate-sharing
  - false-positive
---

# INTENT_GUARD trigger predicates must share the tool-boundary allowlist

## Context

mika#910 introduced a `webhook_no_unauthorized_dispatch` INTENT_GUARD that rejects EndTurn when `run_claude_pilot` was successfully called on a `[GitHub]` webhook turn that is NOT a ready-label dispatch. The trigger predicate was:

```rust
msg.starts_with("[GitHub]") && !msg.starts_with(READY_LABEL_DISPATCH_MARKER)
```

This matched ALL `[GitHub]` events except ready-label — including PR review and check-suite events that are legitimate qa/ci skill territory. When the guard fired on those turns, the agent received confusing "intent-precondition guard fired — re-prompting" corrections that didn't apply.

Meanwhile, the tool-boundary guard in `executor.rs` (check 0 of `validate_dispatch_readiness()`) already used a tighter predicate via `is_unauthorized_webhook_dispatch()` that correctly excluded PR and check-suite events.

## Root Cause

The EndTurn guard and the tool-boundary guard were written at different times. The EndTurn guard's doc comment explicitly acknowledged the over-broadness as a known follow-up. The two predicates drifted apart.

## Fix

Delegate the trigger function to the same `is_unauthorized_webhook_dispatch()` from `crate::webhook_dispatch`:

```rust
fn webhook_no_unauthorized_dispatch_trigger(msg: &str) -> bool {
    is_unauthorized_webhook_dispatch(msg)
}
```

## Lesson

When a domain surface has both a pre-hoc gate (tool-boundary) and a post-hoc guard (EndTurn), both must use the same predicate — ideally by delegating to a shared function. If the EndTurn guard is more permissive, it fires on turns the tool-boundary already handled correctly, generating noise. If it's more restrictive, it blocks legitimate flows. Either drift direction is a bug.

The shared predicate should live in a module both sites import from (here: `crate::webhook_dispatch`), not be copy-pasted.
