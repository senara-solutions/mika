---
title: Intent-precondition registry — generalizing the webhook zero-tools guard
date: 2026-04-21
category: architecture-patterns
module: agent-loop
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding a new EndTurn guard that follows the trigger + tool-signature pattern
  - Diagnosing why a resume/continue instruction was ignored by self-dev
  - Extending the intent guard registry with additional entries
tags:
  - intent-precondition
  - guard-registry
  - endturn-guard
  - resume-intent
  - webhook-guard
  - self-dev
  - structural-enforcement
---

# Intent-precondition registry — generalizing the webhook zero-tools guard

## Context

The webhook zero-tools guard (#696) rejects EndTurn when the user message starts with `[GitHub]` and zero successful tool calls were made. A structurally identical failure occurred when mika-dev was instructed to `resume mika milestone#8` — the agent made zero qualifying reconciliation calls (no `check_task` or `list_tasks`) and ended the turn without dispatching.

Adding a second inline guard next to #696 would fragment a near-identical post-condition check. Instead, the guard was generalized into a registry-driven pattern (`IntentPrecondition` struct + `INTENT_GUARDS` const array in `agent.rs`).

## Guidance

### Registry structure

Each `IntentPrecondition` entry has four fields:
- `label: &'static str` — unique key for retry tracking and logging
- `trigger: fn(&str) -> bool` — matches against `user_input_text`
- `satisfied: fn(&[ToolCallSummary]) -> bool` — checks tool call outcomes
- `correction_message: &'static str` — injected on first rejection

### Adding a new entry

1. Define `detect_<intent>()` as a function with fast-path substring check + `LazyLock<Regex>` (follow existing patterns: `detect_resume_intent`, `detect_completion_claim`)
2. Define `<intent>_satisfied()` checking `ToolCallSummary` fields
3. Add an entry to `INTENT_GUARDS` const array
4. Add eval tests in `tests/eval/test_intent_precondition_guard.rs`

No new retry flag variable needed — the `HashSet<&'static str>` tracks all entries by label.

### Current entries

| Label | Trigger | Satisfied when | Issue |
|-------|---------|---------------|-------|
| `webhook_zero_tools` | `msg.starts_with("[GitHub]")` | Any tool succeeded | #696 |
| `resume_reconcile` | Resume/continue verb + milestone/project ref | `check_task` or `list_tasks` succeeded | #702 |

### What does NOT belong in the registry

Guards with heterogeneous logic that don't fit the trigger + tool-signature pattern:
- Persistence nudge (#648) — nudge, not rejection
- Completion claim (#483) — checks tool registry for `update_task_status`
- Fabricated action (#308) — checks `tools_called.is_empty()` + URL regex
- Text/prose tool call detection — no tool-signature check

## Why This Matters

The codebase has measured evidence (#693, #695, #696, #702) that LLMs ignore prompt-level rules under cognitive load. Structural guards bind deterministically. The registry pattern makes adding new guards a data-declaration task (one entry in `INTENT_GUARDS`) rather than duplicating the guard boilerplate each time.

## When to Apply

- When a new class of "user intent implies required action" failures is observed
- When the agent ends a turn without calling expected tools on a specific message pattern
- When the failure fits the trigger + tool-signature shape (as opposed to content-analysis or nudge shapes)

## Examples

**Before (inline guard for each intent):**
```rust
let mut webhook_zero_tools_retry_done = false;
let mut resume_reconcile_retry_done = false;
// ... each with 20+ lines of identical guard boilerplate
```

**After (registry-driven):**
```rust
let mut intent_guard_retries: HashSet<&'static str> = HashSet::new();

for guard in INTENT_GUARDS {
    if intent_guard_retries.contains(guard.label) { continue; }
    if (guard.trigger)(&user_input_text) && !(guard.satisfied)(&all_tool_summaries) {
        intent_guard_retries.insert(guard.label);
        // inject correction, continue
    }
}
```

## Related

- `docs/solutions/architecture-patterns/webhook-zero-tools-guard-fabrication-prevention-2026-04-20.md` — original #696 guard (now the first entry in the registry)
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — structural guards over prose rules philosophy
- #702 — this PR
- #696 — webhook zero-tools guard (generalization source)
- #265 — match-reason conditioning (AlwaysOn vs Keyword)
