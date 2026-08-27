---
title: "A boot-time consistency scan is only as good as its enumeration — lazy-construction paths reopen the hole"
date: 2026-08-27
category: architecture-patterns
module: agent-core
problem_type: bug_fix
component: server
severity: high
related_issues: [mika#1962, mika#1783, mika#1399]
tags:
  - boot-time-guard
  - lazy-construction
  - config-drift
  - enumeration-predicate
  - env-var
  - agent-tier
  - two-process-divergence
---

# A boot-time consistency scan is only as good as its enumeration

## Context (mika#1962)

`MIKA_AGENT_TIER` selects a being's persona, skill allowlist, and dispatch semantics. It lives in the process environment; the provisioning it selected is written to disk *once* at bootstrap and never rewritten. Two authorities, and they can drift. When they do, the drift is silent: a family-provisioned container that restarts without the var falls through to `AgentTier::Default`, nothing fails, and the leak surfaces only as a user complaint — the exact shape of mika#1783's founding incident.

The fix had two halves, and both halves had a hole worth naming.

## Half 1: caching closes mid-runtime drift. It does nothing for post-boot construction.

`AgentState.tier` reads `AgentTier::from_env()` once at `init_agent` and every `ToolContext` reads the cache. That closes "the value changes under a running agent."

It does **not** close "a new agent is built after the guard ran." `AppState::resolve_agent`'s lazy-construction slow path (mika#1399) calls `init_agent` at *request* time for any agent home that appeared on disk since startup — and `init_agent` does its own `from_env()`. The boot guard, which scanned the runtime home's agent directory (`~/.mika/agents/`, not a repo path) once inside `run_server`, never sees those agents.

**The generalizable rule:** when you add a boot-time consistency scan, enumerate every path that can construct the scanned entity *afterwards*, and give each one the per-entity form of the same check. A one-shot scan plus a lazy constructor is a guard with a documented coverage window and an undocumented one.

The two forms differ in failure disposition, and should:

| | Boot scan | Per-entity check on the lazy path |
|---|---|---|
| Scope | every entity present at startup | the one entity being constructed |
| On mismatch | abort the process | decline **that** entity, log, keep serving |

One drifted agent is not a reason to stop serving the healthy ones. Aborting from a request path would convert a provisioning mistake into an outage.

## The vector that actually fires is the one nobody documented

The module doc listed the drift vectors as "K8s ConfigMap edit, Helm value change, systemd drop-in, `docker exec`." Under scrutiny most of those **cannot fire**: none of them mutate an already-running process's `environ`. They only matter across a restart, which the boot guard already covers.

The vector that trivially fires is a **two-process divergence** that was not on the list:

1. `mika-spirit` runs with `MIKA_AGENT_TIER` unset. Boot guard scans, finds only operator agents, passes.
2. An operator runs `mika agents create nadia` from a shell that **does** export `MIKA_AGENT_TIER=family`. `home::bootstrap` reads *that shell's* env and writes `FAMILY_IDENTITY` + `FAMILY_SOUL` to disk.
3. First `/send` for `nadia` → `resolve_agent` slow path → `init_agent` → tier resolved from the **server's** env = `Default`.
4. A family persona is now served under operator semantics, silently, until someone restarts.

**Rule:** when a config value is read by a long-running process but *written to disk* by a short-lived CLI, the two processes are the drift vector. Enumerate writer processes, not just mutation events on the reader.

## Half 2: the scan's coverage predicate must match the *serving* predicate

The guard enumerated via `agent::list_agents`, which filters on `config.toml` existing. But the server's own definition of a servable agent is looser:

- `AppState::resolve_agent` gates only on `identity.toml`.
- `Settings::load_for_agent` adds the per-agent `config.toml` with `.required(false)`.

So an agent home with `identity.toml` + `soul.md` but no `config.toml` — a partial restore, a hand-assembled directory, an interrupted bootstrap — was **fully servable and invisible to the guard**. Two predicates inside one feature disagreed about what an agent is, and the guard held the looser-consequence side.

The fix takes the union. The rule is simpler than the fix: **a guard's enumeration predicate must be a superset of the predicate that decides what gets served.** When they are written in different modules by different tickets, they drift; assert the relationship rather than assuming it. (Sibling case, same shape: `asymmetric-perimeter-predicate-drift.md`.)

## Prevention

Before shipping a boot-time scan, answer three questions in the plan, not in review:

1. **Who else constructs this?** Grep for every callsite of the constructor the scan protects. A lazy/on-demand path is the usual miss.
2. **Who writes the state I am scanning?** If any writer is a *different process* from the reader, that is your live drift vector — the reader-side mutation stories are usually the inert ones.
3. **Does my enumeration match what gets served?** Diff the guard's predicate against the serving path's predicate explicitly.

## Applicability

This generalizes past agent tiers to any "on-disk provisioning vs process config" pair: feature flags baked at install time vs read from env, a license/plan tier written at provisioning vs enforced at runtime, schema/migration markers written by a CLI and read by a server. The three questions above are the reusable part; the tier specifics are not.
