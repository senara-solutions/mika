---
title: "Post-hoc EndTurn guards are insufficient for stateful side-effect tools"
date: 2026-05-13
category: architecture-patterns
module: agent-core
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - Adding a new guard that must prevent a tool's side effects from executing
  - Choosing between EndTurn intent-precondition guards and tool-boundary checks
  - A post-hoc guard fires correctly but the damage is already done
tags:
  - guard-design
  - tool-boundary
  - endturn
  - intent-precondition
  - side-effects
  - dispatch
  - webhook
  - defense-in-depth
related_components:
  - tooling
---

# Post-hoc EndTurn guards are insufficient for stateful side-effect tools

## Context

mika#910 (2026-04-30) added `webhook_no_unauthorized_dispatch` as an INTENT_GUARD — a post-hoc EndTurn check that fires after the LLM's response is complete. When a `[GitHub]` webhook turn (not a ready-label event) successfully called `run_claude_pilot`, the guard rejected the response and re-prompted the agent.

The guard worked as designed: it detected the unauthorized dispatch and corrected the agent's behavior on the next turn. But by the time `EndTurn` evaluates, the `run_claude_pilot` tool has already executed — the callback task exists in the DB, the claude-pilot subprocess is running, and real work is happening. The correction teaches the model for the next turn but cannot undo the side effects.

mika#932 (2026-05-02) reproduced this gap live: an `issue_comment.created` webhook with dispatch-class keywords ("Ready to dispatch") in the body triggered the dispatch heuristic. The #910 guard fired in the tool-call traces, but claude-pilot was already running task `30e8bf22`.

## Guidance

**Identify the tool's reversibility before choosing the guard layer.**

| Tool category | Side effects | Correct guard layer |
|---|---|---|
| **Read-only** (`search_memory`, `gh_read`, `list_tasks`) | None — no state change | EndTurn intent-precondition guard is sufficient |
| **Idempotent writes** (`store_fact`, `update_core_memory`) | Reversible via audit log | EndTurn guard sufficient; rewind can undo |
| **Stateful side-effect** (`run_claude_pilot`, `pr_merge_with_gate`) | Subprocess spawned, external state changed | **Tool-boundary check required** — reject before execution |

For stateful side-effect tools, the guard must run inside the executor's dispatch-readiness chain (`validate_dispatch_readiness` in `skills/executor.rs`), before the subprocess spawns or the external API call fires. EndTurn guards remain as defense-in-depth (belt-and-suspenders) but are not load-bearing for the side-effect prevention.

## Why This Matters

The failure mode is subtle: the EndTurn guard works *correctly* — it detects the violation and re-prompts. Observability shows the guard firing. But the side effect has already shipped. This creates a false sense of safety where the guard's presence in the codebase suggests the invariant is enforced when it is only *detected*.

The distinction matters most for tools that spawn subprocesses (like `run_claude_pilot`) or mutate external state (like `pr_merge_with_gate`), because those side effects are not reversible by the agent's retry on the next turn.

## When to Apply

- When adding a new behavioral invariant that must prevent a tool from executing
- When an existing EndTurn guard is observed "working" but the side effect still lands
- When designing the check ordering in `validate_dispatch_readiness` — cheapest checks first (string prefix < DB read < API call)

## Examples

**mika#910 (post-hoc, insufficient alone):**

```rust
// EndTurn INTENT_GUARD — fires AFTER the tool call chain completes
fn webhook_no_unauthorized_dispatch_trigger(msg: &str) -> bool {
    msg.starts_with("[GitHub]") && !msg.starts_with(READY_LABEL_DISPATCH_MARKER)
}
// By the time this evaluates, run_claude_pilot has already spawned
```

**mika#933 (tool-boundary, prevents execution):**

```rust
// validate_dispatch_readiness check (0) — fires BEFORE subprocess spawn
if let Some(msg) = originating_message
    && crate::webhook_dispatch::is_unauthorized_webhook_dispatch(msg)
{
    return Err(json!({"error": "unauthorized_webhook_dispatch", ...}).to_string());
}
// run_claude_pilot never executes — the tool returns an error
```

Both guards coexist: #933 is load-bearing (prevents the side effect), #910 is defense-in-depth (catches any bypass path the tool-boundary check might miss).

## Related

- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — the broader "engine guards vs prompt rules" pattern (complementary: that doc argues for engine guards over prompt rules; this doc argues for tool-boundary guards over EndTurn guards within the engine layer)
- `docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md` — the INTENT_GUARDS registry design
- mika#910 — the post-hoc EndTurn guard (defense-in-depth)
- mika#933 — the tool-boundary guard (load-bearing fix)
- mika#932 — the live incident that demonstrated the post-hoc gap
