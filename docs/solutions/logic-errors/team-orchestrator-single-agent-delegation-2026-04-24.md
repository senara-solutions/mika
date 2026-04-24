---
title: "Team orchestrator delegates to only one agent instead of distributing work"
date: 2026-04-24
category: logic-errors
module: teams
problem_type: logic_error
component: assistant
symptoms:
  - "Team runs with 5 agents produce sessions for only 1-2 specialists"
  - "Orchestrator assigns all tasks to the most technical agent, ignoring specialists with relevant mandates"
  - "Creative/brainstorming goals that should leverage diverse perspectives use only one agent"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags:
  - team-engine
  - orchestrator
  - decompose
  - coverage-check
  - retry
  - prompt-reinforcement
---

# Team orchestrator delegates to only one agent instead of distributing work

## Problem

The team engine's `decompose()` function had no contract requiring the orchestrator to account for the full roster of team members. On multi-agent teams with diverse specialists, the orchestrator would assign all work to one or two agents (typically the most technically-framed one), silently ignoring members whose mandates matched the goal.

## Symptoms

- Team run `fd7ef7ef` (inner-circle, 5 agents): only orchestrator + mika-dev active; elon-musk, chase-hughes, mika-qa never used on a username-brainstorming goal
- `SELECT id, agent_id FROM sessions WHERE id LIKE 'team-<run_id>%'` shows sessions for only 1-2 of N specialists
- Creative or brainstorming goals that plainly match multiple mandates route to a single technical agent

## What Didn't Work

- The orchestrator prompt listed team members with their mandates but never instructed the orchestrator to consider all of them. The LLM's path of least resistance was to pick the first relevant-looking agent and assign everything there.

## Solution

Two-layer fix: prompt reinforcement as the primary lever, with a structural coverage check as a backstop.

**Layer 1 — Prompt reinforcement** (`crates/mika-agent/src/teams/prompt.rs`):

Added a roster-awareness instruction to `build_orchestrator_context` after the "decompose into tasks" line:

```
Consider every team member's mandate before responding. It's fine to
leave a member out if their expertise doesn't fit the goal, but make
that decision deliberately -- don't default to the first one or two
that come to mind.
```

**Layer 2 — Coverage check** (`crates/mika-agent/src/teams/engine.rs`):

Added `missing_members()` helper that computes the set difference between non-orchestrator team members and assigned task agents. Called in `decompose()` after `parse_task_assignments` returns `Tasks(...)`:

- If all members are covered, proceed as before
- If members are missing, issue one nudge re-prompt listing the omitted names
- If the retry still has gaps (or returns a conversational reply), emit `warn!` with the locked `team_coverage_gap` schema and fall through
- Second response always wins (it's strictly more informed than the first)

**Layer 3 — Observability** (`crates/mika-agent/src/teams/types.rs`):

Added `coverage_retry_fired: bool` to `TeamRun` with `#[serde(default)]`. Persisted via the existing `team_runs.checkpoint` JSON column — no schema migration needed. Queryable via `json_extract(checkpoint, '$.coverage_retry_fired')`.

## Why This Works

The root cause was a missing contract, not a model deficiency. The orchestrator prompt described the team and the task format but never said "account for everyone." LLMs optimize for the path of least resistance — assigning to one agent satisfies the format requirements with minimal effort.

The prompt instruction biases the LLM toward roster-aware responses on the first try. The structural retry catches cases where the prompt alone isn't enough, bounded to one extra LLM call per decomposition. The `warn!` log provides post-ship signal: if it's silent, the prompt is doing the work; if it fires often, escalation to structured schemas is warranted.

## Prevention

- **Pattern: prompt-first, structural backstop.** When adding new behavioral expectations to LLM-driven flows, add both a prompt instruction (primary lever) and a code-level check (safety net). This mirrors the existing post-condition guard pattern in `agent_loop.rs`.
- **Log grep for monitoring:** `grep team_coverage_gap server.log | jq` shows retry fire rate, missing members, and recovery status per run.
- **Existing precedent:** `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — "prompt-only is fragile, add a code guard." This fix extends the pattern to team orchestration.

## Related Issues

- [senara-solutions/mika#286](https://github.com/senara-solutions/mika/issues/286)
- Related pattern: `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md`
