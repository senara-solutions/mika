---
module: self-dev
tags: [autonomous-loop, grooming, dispatch, dev-groom, mika-arch, plan-on-branch]
problem_type: structural-gap
category: best-practices
date: 2026-05-06
ticket: mika#996
---

# Grooming as a phase of dispatch, not as orchestrator overhead

## Problem

The orchestrator's manual `/mika-groom-ticket` workflow cannot keep up with cascade-mode dispatch speed. On 2026-05-06, milestone#13 cascade dispatched mika#671 to claude-pilot before the orchestrator could pre-groom it. claude-pilot ran `/ce:plan` from scratch in its worktree, missing the architect's two-pass roundtrip. Retroactive grooming would have collided with the in-flight implementation on the same branch — actively harmful.

This is the third structural autonomous-loop fix surfaced that session (alongside mika#988 closed-issue auto-skip and mika#991 post-callback conversational turn). All three address loop correctness/legibility, not speed.

## Root cause

The architect roundtrip (two-pass READY/ITERATE/ESCALATE → GROOMED/ESCALATE cycle) was treated as orchestrator-owned overhead rather than a phase of the dispatch pipeline itself. Manual gates do not scale with pipeline cadence — when the pipeline accelerates (cascade compression), the gate becomes the bottleneck and tickets ship without the design quality review they are supposed to receive.

## Solution

Make grooming a phase of the dispatch pipeline. Two integration points:

1. **Webhook path (Ready-Label Dispatch Step 3):** When a `ready`-labelled ticket lacks the `Plan: docs/plans/` bypass predicate in its issue body, mika-dev dispatches `dev-groom` (two-pass architect review) before `dev-pilot`. On `Verdict: GROOMED`, the handler re-enters the dispatch flow. On `Verdict: ESCALATE`, dispatch halts and surfaces to operator.

2. **Milestone path (M4 Step 1.5):** Before launching `dev-pilot` for a milestone child, check the child's issue body for the Plan callout. If absent, launch `dev-groom` first; on its callback, then launch `dev-pilot`.

Both paths use serial grooming-then-dispatch (no concurrency — deferred to mika#1001). The consent gate relocated from the slash-command path to the `ready` label transition + the existing positive-consent dispatcher (mika#807/#810).

Terminal-semantics rule (NF2): on HANDLER CRASH, retry once reusing the same task_id; on second consecutive crash, treat as ESCALATE (surface to operator, do NOT retry again). Closes the latent infinite-retry class.

## Key decisions

- **A+B serial, not C (hybrid + concurrency):** Covers both webhook and milestone paths without requiring engine changes for concurrent claude-pilot dispatches. Concurrency deferred to mika#1001.
- **Re-entry mechanism, not inline duplication:** The post-groom webhook handler re-enters the full Ready-Label Dispatch handler from the top, not by duplicating Steps 4-5 inline. Keeps dispatch logic in one place.
- **Bypass predicate is `Plan: docs/plans/`**, not just `Plan:`. Avoids false positives on the word "Plan:" appearing in issue body prose.
- **`[output] required_suffix_lines`** added to dev-groom's `skill.toml` for mechanical verdict parsing (not LLM-dependent). Same pattern as `mika-arch-second-review` and `mika-arch-groom-ticket`.
- **No engine-level Rust changes.** The `webhook_ready_label_dispatch` engine guard is skill-agnostic — accepts `run_claude_pilot` with any `skill` parameter. The `disabled_skills` entry for dev-groom in `well_known_agents.rs` prevents keyword-triggered activation, not `run_claude_pilot` dispatch.

## Contrapositive

mika#988 (closed-issue auto-skip) is the contrapositive: the gate IS the right shape there because the validation is non-LLM-driven and instant. Auto-groom is the right shape when the validation requires an LLM-driven prerequisite (architect review) that takes minutes.

## Principle

When a ticket-handling pipeline has a "validate-before-execute" step that depends on an LLM-driven prerequisite (architect review, design check, security audit, etc.), the prerequisite should run as a phase of the pipeline itself, not as a manual gate the operator runs separately. Manual gates do not scale with pipeline cadence — when the pipeline accelerates, the gate becomes the bottleneck.
