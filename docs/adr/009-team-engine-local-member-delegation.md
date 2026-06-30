---
title: "arch: team-engine local-member delegation — name-addressed decompose, not a2a_call"
type: arch
status: draft
date: 2026-06-30
---

# arch: team-engine local-member delegation — name-addressed decompose, not a2a_call

## Overview

Records the design verdict for mika#1653. In a team run, a team-mode orchestrator
must reach its local team members by **name**, via the engine's structural
decompose→spawn→resume cycle — never via `a2a_call`, which is URL-addressed,
cross-container, and structurally fails on local siblings.

## Context

In the Litha incident (team-`d68fdcaf`), a team-mode orchestrator tried to reach
four LOCAL team members (`odds-engine-{ceo,cto,quant}`, `chase-hughes` — agents
under `~/.mika/agents/`) via `a2a_call`. All four returned **503 Service
Unavailable**. The orchestrator diagnosed the mismatch in-band ("the team agents
aren't directly reachable via A2A — this is a team run, so I should assign tasks
through the team run infrastructure instead") but could not recover within the run.

Two coupled facts drove the failure:

1. **Team-mode delegation is already structural — not tool-driven.** The team
   engine does not expect a "reach a member" tool. `TeamEngine::decompose()` runs
   the orchestrator LLM turn; the orchestrator responds with a JSON array of task
   assignments (`[{"agent", "task", "output_file"}]`); `execute_tasks()` spawns one
   child session per assignment via `run_team_agent()`; the parent resumes when
   children complete; members exchange results through workspace files. Members are
   routed by **name** from the team manifest.

2. **The team tool array contained a contradicting affordance.** `TeamEngine::
   init_resources()` built the registry as `default_tools()` + `team_tools(...)`.
   `a2a_call` is registered unconditionally inside `default_tools()`, so it was
   present. `delegate_task` — the conversation-mode local-delegation tool — is a
   *management* tool added only by `management_tools_if_needed()`, which the team
   engine never calls, so it was absent. The only mis-reach affordance the model
   saw was `a2a_call`. A model that sees team members and an "agent-to-agent call"
   tool will reach for the tool.

`a2a_call` is structurally cross-container: its schema requires a `url` (not an
agent name); it has SSRF protection rejecting loopback/private targets; a public
gateway URL routes through `/a2a/{customer_id}/{agent_name}` to a per-customer
container that does not exist for a local sibling, yielding a gateway **503**.
`a2a_call` has no local fast-path, so it is structurally guaranteed to fail on a
local target — either at the SSRF guard or with a gateway 503.

## Decision

**Shape A — the team engine routes local members by name; the orchestrator never
calls a per-member reach tool. Make that path exclusive and structurally
enforced.**

The structural routing already exists (decompose→spawn→resume). The incident was
an unguarded affordance plus an under-specified prompt, not a missing mechanism.
The fix is three layers:

1. **Layer 1 — Prompt (`teams/prompt.rs`).** `build_orchestrator_context()` names
   the decompose-JSON assignment as the exclusive reach mechanism, states that the
   engine spawns assigned members automatically and returns results next turn,
   explicitly forbids using `a2a_call` for team members, and points members at
   workspace files for result exchange.

2. **Layer 2 — Structural suppression (`teams/engine.rs`).** `a2a_call` is removed
   from the team-mode tool registry (`build_team_tool_registry()`, applying the
   `TEAM_SUPPRESSED_TOOLS` list) so the orchestrator never sees it. The team engine
   owns a private `ToolRegistry`, so suppression is scoped to team runs and cannot
   affect conversation-mode agents. This is the enforcement that answers the
   brittleness objection — with the tool absent from the array, the orchestrator
   cannot mis-reach regardless of prompt drift. A unit test
   (`test_team_registry_suppresses_a2a_call`) asserts the invariant so a future
   `default_tools()` change cannot silently re-introduce the affordance.

3. **Layer 3 — `a2a_call` self-describes (`tools/a2a_call.rs`).** For conversation
   mode, where `a2a_call` legitimately remains registered alongside `delegate_task`:
   the description states the tool is external/remote-only and points local targets
   to `delegate_task` (conversation) or team task-assignments (team mode); the SSRF
   rejection error names the local-agent alternative, converting a dead-end refusal
   into an actionable redirect.

### What a team-mode orchestrator should use, and how it knows (AC2)

The reach mechanism is the **decompose-JSON task assignment** — name-addressed,
engine-spawned. The orchestrator knows because Layer 1 names it explicitly and
Layer 2 removes the only competing affordance.

### What `a2a_call` does when targeted at a local agent (AC2)

**Conversation mode:** error-with-hint (Layer 3) — the loopback/private SSRF
rejection redirects to `delegate_task`. **Team mode:** not presented at all (Layer
2) — the tool is absent from the array.

### Rejected: Shape B — `a2a_call` learns local routing

`a2a_call` is URL-addressed; the failed targets were **public gateway URLs**,
indistinguishable at the tool layer from legitimate remote endpoints. Teaching
`a2a_call` to recognize "this gateway URL loops back to a local sibling" would
require it to parse the gateway's `mika-{customer_id}/{agent_name}` routing
convention and resolve it to a local agent name — a brittle layering violation
encoding gateway topology into the tool. The clean boundary is the existing one:
**name → local (`delegate_task`/decompose); URL → remote (`a2a_call`)**.

### Rejected: Shape C — manifest differentiates local vs remote

The team manifest (`TeamDefinition`/`TeamAgent`) holds **only** local agents,
addressed by name. Teams have no remote-member concept today. An `is_local` flag
would be universally `true`, encoding zero information, while forcing a
schema/manifest migration for no behavioral gain. Local-vs-remote is already
encoded structurally by which mechanism addresses the member (name vs URL). No
manifest change is warranted (AC2 — manifest schema change = none).

## Consequences

- Orchestrators **cannot** mis-reach local members — the affordance is gone from
  the team tool array, and the prompt names the real mechanism.
- Conversation-mode `a2a_call` users who target a local agent get an actionable
  redirect to `delegate_task` instead of an opaque refusal.
- **Composability with mika#1652 (AC3):** this design is *prevention*; mika#1652 is
  *containment* (a terminal-state writer for stuck `team_runs`). They share no code
  surface. This plan routes team-member work correctly so the orchestrator cannot
  enter the 503-loop that stranded `team_runs` in the founding incident; mika#1652's
  reaper still catches `team_runs` that wedge for other reasons (member crash, LLM
  timeout, deadline-exceeded). No ordering dependency — either can land first.
- **Migration (AC4):** none required. All three layers are additive and
  presentation-scoped. No team manifest format change, no DB migration, no breaking
  change to `a2a_call`'s contract for legitimate remote callers. Deployed teams keep
  working; their orchestrators lose a tool they should never have used and gain
  clearer instructions.

## References

- mika#1653 — this design ticket.
- mika#1652 — sibling containment (terminal-state writer for stuck `team_runs`).
- `crates/mika-agent/src/teams/engine.rs` — `build_team_tool_registry`,
  `TEAM_SUPPRESSED_TOOLS`.
- `crates/mika-agent/src/teams/prompt.rs` — `build_orchestrator_context`.
- `crates/mika-agent/src/tools/a2a_call.rs` — description + SSRF redirect.
- `docs/architecture/review-guide.md` § Orthogonality — clean name-vs-URL boundary.
