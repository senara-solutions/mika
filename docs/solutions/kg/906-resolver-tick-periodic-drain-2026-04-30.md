---
module: kg/entity_resolver
tags: [kg, resolver, periodic-task, drain-rate, budget, tokio]
problem_type: throughput-bottleneck
category: kg
date: 2026-04-30
issue: 906
---

# KG Resolver: Periodic tick decouples drain rate from restart cadence

## Problem

`MIKA_KG_BATCH_BUDGET=500` per agent caps Stage-2 LLM disambiguation at 500 calls per restart. With infrequent production restarts and continuous new extraction via compound hooks, drain rate fell below new-extraction rate. mika-arch primary corpus had ~17K pending subjects; at 500 LLM calls per restart, draining would take ~56 restarts.

The bottleneck was **execution cadence**, not budget size. The resolver only ran in two contexts: (1) startup background spawn, (2) compound-hook synchronous spawn. Neither was sufficient for steady-state drain.

## Root Cause

`resolve_pending(budget)` had no periodic execution path. It only ran reactively — at startup and after compound extraction. mika#757's AC explicitly anticipated this: *"leave remaining work for subsequent deferred runs"* — but no deferred-run mechanism existed between restarts.

## Solution

Added a third execution context: `kg::resolver_tick::spawn_resolver_tick_task()` — a periodic tokio task that runs `resolve_pending(budget)` every 30 minutes per KG-enabled agent.

### Key design decisions

1. **Budget unchanged at 500.** Throughput improvement via cadence, not burst size. Preserves #757's "no silent multi-thousand-call bursts" invariant.
2. **First fire skipped.** `interval.tick().await` at the top consumes the immediate fire; the startup spawn handles post-restart drain.
3. **Fail-open (log-and-skip).** Same pattern as `checkpoint_task`. A failing tick does NOT abort the interval.
4. **Lifecycle tied to tokio runtime drop.** No explicit shutdown hook. Body fully bounded per iteration.
5. **30-min interval hard-coded.** Future tunable env var deferred per #757's lazy-introduction precedent.

### Race safety

Three execution contexts (startup, compound-hook, tick) can overlap. `kg_resolutions_log UNIQUE(agent_id, subject_entity_id)` ensures concurrent resolution of the same entity is a fast no-op — second writer's INSERT is rejected by the constraint.

### Observability

- `kg_resolver_tick.start` — fires at each tick with `pending_before` count
- `kg_resolver_tick.complete` — fires after resolution with `resolved_in_tick`, `pending_after`, `aborted_budget`, `llm_calls`
- `kg_resolver_tick.error` — fires on resolution failure (fail-open)
- Signal E in `CLAUDE.md` § Post-restart safety check: `grep kg_resolver_tick.complete server.log | jq 'select(.pending_after == 0)'`

### Cost model

- Active drain: 4 agents × 500 calls × 48 ticks/day × ~$0.0001/call = ~$10/day at full burn
- Drained state: ~$0/day (`resolve_pending` short-circuits on empty pending — 1 SQLite query per 30 min per agent)

## Files Changed

- `crates/mika-agent/src/kg/resolver_tick.rs` — new module with `spawn_resolver_tick_task()` and `tick_body()`
- `crates/mika-agent/src/kg/entity_resolver.rs` — added `count_pending()` public method (mirrors `get_pending_entities` SQL with `SELECT COUNT(*)`)
- `crates/mika-agent/src/kg/mod.rs` — registered `resolver_tick` module
- `crates/mika-agent/src/server/mod.rs` — wired tick into per-agent init alongside startup spawn

## Pattern Reference

Mirrors `server::checkpoint::spawn_dashboard_checkpoint_task()` — interval-based tokio task with fail-open error handling and runtime-drop lifecycle. This is the canonical pattern for periodic background tasks in mika-server.

## Lessons

1. **Cadence vs burst** — When a budget-bounded operation needs higher throughput, prefer increasing execution cadence over raising the per-execution budget. Cadence is self-limiting (bounded by interval); budget increases compound across agents.
2. **Deferred runs need a mechanism** — mika#757's AC said "leave remaining work for subsequent deferred runs" but didn't build the deferred-run mechanism. Design documents that reference future deferred execution should either build the mechanism or explicitly log a follow-up ticket.
3. **Pattern precedent** — The `checkpoint_task` pattern (interval + fail-open + runtime-drop lifecycle) is reusable for any periodic background work in mika-server. New periodic tasks should follow this pattern.

## Counter contract (added 2026-05-06)

`pending_before` (this tick's `count_pending()` result) is scoped to the 5 domain-resolvable subject types — see `crates/mika-agent/src/kg/entity_resolver.rs:891-906`. It will not match `mika kg status` "pending", which includes the 3 subject-graph-only types (`pattern`, `failure_mode`, `solution_path`) that the resolver intentionally never touches. Steady-state `pending_before: 0` means the resolver has drained everything it's supposed to drain — not that the subject graph is empty. Full type-allowlist contract: `docs/solutions/best-practices/kg-resolver-tick-visibility-audit-2026-05-06.md`.
