# Plan: team-engine a2a_call vs delegate_task — orchestrator reaches for the wrong tool for LOCAL team members

**Ticket:** mika#1653
**Type:** design (architectural verdict + scoped implementation)
**Repo:** senara-solutions/mika
**Sibling:** mika#1652 (load-bearing containment — terminal-state writer for stuck `team_runs`)

## Problem

In the Litha incident (team-d68fdcaf), a team-mode orchestrator tried to reach four LOCAL team members (`odds-engine-{ceo,cto,quant}`, `chase-hughes` — agents at `~/.mika/agents/`) via `a2a_call`. All four returned **503 Service Unavailable**. The orchestrator identified the mismatch in-band ("the team agents aren't directly reachable via A2A — this is a team run, so I should assign tasks through the team run infrastructure instead") but could not recover.

The ticket poses two coupled architect-bearing questions:

1. **Why did the orchestrator reach for `a2a_call` instead of the team-run delegation mechanism?**
2. **Why does `a2a_call` return 503 for local team-member targets?**

This plan answers both from code evidence, records the verdict (the ticket's AC1), and ships the scoped fix the verdict implies.

## Current state (code-grounded)

### Team-mode delegation is ALREADY structural — not tool-driven

The team engine does not expect the orchestrator to call any "reach a member" tool. Delegation is a decompose→spawn→resume cycle:

1. **Decompose** — `TeamEngine::decompose()` (`crates/mika-agent/src/teams/engine.rs:686`) runs the orchestrator LLM turn with the context built by `build_orchestrator_context()` (`crates/mika-agent/src/teams/prompt.rs:14`). The orchestrator is instructed to **"Respond with a JSON array of task assignments"** — `[{"agent": ..., "task": ..., "output_file": ...}]` (`prompt.rs:116-125`).
2. **Spawn** — `parse_task_assignments()` (`engine.rs:1573`) parses that JSON; `execute_tasks()` spawns one child session per assignment via `run_team_agent()` (`engine.rs:1034` `join_set.spawn`). Each member runs in `team-{run_id}-{agent_name}` with workspace R/W tools.
3. **Resume** — the parent `invoke_orchestrator` task fires when children complete; members exchange results via workspace files, not direct calls.

The orchestrator never needs `a2a_call` or `delegate_task` to reach a member. The engine routes them by **agent name** from the manifest.

### What tools the team orchestrator actually has

`TeamEngine::init_resources()` builds the team tool registry as `tools::default_tools()` + `tools::team_tools(...)` (`engine.rs:144-147`). Critically:

- **`a2a_call` IS present** — registered unconditionally inside `default_tools()` (`crates/mika-agent/src/tools/mod.rs`, `default_tools`).
- **`delegate_task` is NOT present** — it is a *management* tool, registered only by `management_tools_if_needed()` (gated on `agents.len() > 1 || !teams.is_empty()`), which the team engine **does not call**. The shared registry is base + workspace tools only.

So the ticket's framing ("two tools available: `delegate_task` and `a2a_call`") is accurate for **conversation mode** but not for **team mode**: inside a team run the only mis-reach affordance the model sees is **`a2a_call`**. `delegate_task` is the conversation-mode local-delegation tool; the team-mode equivalent is the decompose-JSON assignment.

### Why the model picked `a2a_call` (answer to Q1)

`build_orchestrator_context()` lists the members and says "respond with a JSON array of task assignments," but it does **not** tell the orchestrator (a) that the engine spawns those members automatically and returns their results on the next turn, nor (b) that the `a2a_call` tool in its array must NOT be used to reach a member. The prompt under-specifies the decompose mechanism as the *exclusive* path while leaving a contradicting affordance (`a2a_call`, literally an "agent-to-agent call" tool) in the LLM tool array. A model that sees team members and an agent-to-agent call tool will reach for the tool. This is the same "unsound proxy" class flagged in the ticket — a tool that *looks* like the right lever but routes nowhere valid for this target.

### Why `a2a_call` 503s on a local target (answer to Q2)

`a2a_call` is structurally **cross-container, URL-addressed, HTTP**:

- Its schema requires a `url` (not an agent name) (`crates/mika-agent/src/tools/a2a_call.rs:27-44`).
- It has SSRF protection that rejects `localhost`/`127.0.0.1`/`::1`/private IPs (`a2a_call.rs:74-90`) — so a direct loopback URL is refused outright.
- A public gateway URL passes SSRF, then routes through the gateway A2A proxy `/a2a/{customer_id}/{agent_name}` (`crates/mika-gateway/src/a2a_routes.rs:31`), which forwards to a per-customer **container** at `mika-{customer_id}...svc.cluster.local:8080`. When no such container/route exists for a local sibling agent, the gateway returns **503 BAD_GATEWAY** (`a2a_routes.rs:118-128`).

Local sibling agents under `~/.mika/agents/<name>` are not separate containers with gateway routes. `a2a_call` has **no** local fast-path (`agent_exists()` at `crates/mika-common/src/agent.rs:47` is used by `delegate_task`, never by `a2a_call`). So `a2a_call` against a local sibling is structurally guaranteed to fail — either at the SSRF guard (loopback URL) or with a gateway 503 (public URL).

## Design verdict (AC1)

**Chosen: Shape A — the team engine routes local members by name; the orchestrator never calls a per-member reach tool. Make that the exclusive, enforced path.**

Shape A is correct because the structural routing **already exists** (decompose→spawn→resume). The incident was not a missing mechanism — it was an unguarded affordance (`a2a_call` in the array) plus an under-specified prompt. The fix is to (1) name the real mechanism in the prompt, (2) remove the mis-reach affordance structurally, and (3) make `a2a_call` self-describe and fail-loud toward the right alternative. The ticket's tradeoff note ("A is cheapest but relies on model discipline; same brittleness as keyword-tightening") is answered by **Layer 2**, which removes the affordance from the tool array entirely — so enforcement does not depend on model discipline.

### Why not Shape B (`a2a_call` learns local routing)

Rejected. `a2a_call` is URL-addressed; the failed targets were **public gateway URLs**, indistinguishable at the tool layer from legitimate remote endpoints. Teaching `a2a_call` to recognize "this gateway URL loops back to a local sibling" would require it to parse the gateway's `mika-{customer_id}/{agent_name}` routing convention and resolve it back to a local agent name — a brittle layering violation (the tool would encode gateway topology). The clean boundary is the existing one: **name → local (`delegate_task`/decompose); URL → remote (`a2a_call`)**. Don't blur it.

### Why not Shape C (manifest differentiates local vs remote)

Rejected. The team manifest (`TeamDefinition`/`TeamAgent` in `crates/mika-common/src/team.rs:12-35`) holds **only** local agents, addressed by `name`. Teams have no remote-member concept today. An `is_local` flag would be universally `true`, encoding zero information, while forcing a schema/manifest migration (AC4 cost) for no behavioral gain. Local-vs-remote is already encoded structurally by *which mechanism addresses the member* (name vs URL). No manifest change is warranted.

## Design (the three layers)

### Layer 1 — Prompt: name the real mechanism, exclude the wrong one (`teams/prompt.rs`)

In `build_orchestrator_context()` (`crates/mika-agent/src/teams/prompt.rs:106-125`), extend the `## Instructions` block with an explicit reach-mechanism statement. Required content:

- Team members are reached **only** by including them in the JSON task-assignment array. The team engine spawns each assigned member's session automatically and returns their results to the orchestrator on the next turn.
- The orchestrator must **not** use `a2a_call` to reach a team member. `a2a_call` is for *external, cross-container* agents addressed by URL; team members are local siblings addressed by name.
- Members communicate results through workspace files (reinforces the existing `read_workspace`/`write_workspace` guidance at `prompt.rs:244-252`), not through direct calls to each other.

This is prompt-layer disambiguation. It is necessary (the model needs to know the mechanism) but **not sufficient** on its own — Layer 2 is the enforcement.

### Layer 2 — Structural: remove `a2a_call` from the team-mode tool array (`teams/engine.rs`)

No agent inside a team run — orchestrator or member — has a valid use for `a2a_call` to reach another team member, and local siblings are never A2A-reachable. Suppress `a2a_call` from the team tool registry so the model never sees it.

**Where:** `TeamEngine::init_resources()` (`crates/mika-agent/src/teams/engine.rs:143-147`), immediately after the registry is assembled from `default_tools()` + `team_tools()`. Remove the `a2a_call` definition/handler from this team-local registry. The team engine builds its **own** registry (not the shared server `ToolRegistry`), so suppression here is scoped to team runs and cannot affect conversation-mode agents.

**Implementation choice (implementer to pin):** Two viable mechanisms, both already in the codebase —
1. **Registry-level** — after building `tool_registry`, drop `a2a_call` from it (the cleanest: the tool is then neither presented nor executable in team runs). Requires a `ToolRegistry` removal/filter affordance; if none exists, add a minimal `remove(name)` or build the registry from a filtered tool list.
2. **Presentation-level** — reuse the existing `apply_agent_tool_visibility()` hook (`crates/mika-agent/src/agent_loop/mod.rs:4897`) by threading a team-mode suppression list (`["a2a_call"]`) into `run_team_agent`'s `disabled_tools` so it is unioned with each agent's identity `[tools].disabled`. This hides the tool from the LLM array while leaving the shared registry intact.

**Recommendation:** prefer (1) (registry-level) because the team engine owns a private registry — removing the tool there is the most localized and leaves no "present in registry but hidden" gap. Fall back to (2) if `ToolRegistry` lacks a removal affordance and adding one is judged out of scope; (2) is functionally equivalent for the LLM-visibility outcome. The implementer pins the choice in the PR with the concrete diff.

This is the layer that answers the ticket's brittleness objection: with `a2a_call` absent from the array, the orchestrator **cannot** mis-reach regardless of prompt drift.

### Layer 3 — `a2a_call` self-describes and fails toward the right alternative (`tools/a2a_call.rs`)

Defense-in-depth for **conversation mode**, where `a2a_call` legitimately remains registered alongside `delegate_task`. Two changes:

1. **Description hardening** (`a2a_call.rs:23-26`): state that `a2a_call` is for **external, remote (cross-container) agents only**, and that to reach a local agent on this host the caller should use `delegate_task` (conversation) or team task-assignments (team mode).
2. **Error-message hardening** (`a2a_call.rs:75-89`): when the SSRF guard rejects a `localhost`/loopback/private-IP target, the returned error should name the alternative ("this looks like a local agent; use `delegate_task` to reach a local agent on this host"). This converts a dead-end refusal into an actionable redirect — the same "error-with-hint" shape the ticket lists as an acceptable `a2a_call`-on-local outcome.

Note the deliberate scope boundary: Layer 3 does **not** attempt to detect a local agent behind a *public gateway* URL (that is the rejected Shape B). It only improves the message on the cases `a2a_call` can already detect (loopback/private) plus the static description.

## Composability with mika#1652 (AC3)

This design is **prevention**; mika#1652 is **containment**. They share no code surface and compose cleanly:

- This plan routes team-member work correctly so the orchestrator cannot enter the 503-loop that stranded `team_runs` in the founding incident.
- mika#1652's terminal-state writer still catches any `team_runs` that wedge for *other* reasons (member crash, LLM timeout, deadline-exceeded mid-run). Prevention narrows the inflow; the reaper remains the backstop for the residual.

No ordering dependency: this PR can land before or after mika#1652. Neither blocks the other.

## Migration (AC4)

**None required.** All three layers are additive and presentation-scoped:

- Layer 1 — prompt text only; no schema, no manifest, no API.
- Layer 2 — filters the team-mode LLM tool array; the shared `ToolRegistry` and all other modes are unchanged. Existing team manifests (`team.toml`) hold only local agent names and work unchanged.
- Layer 3 — tool description + error-message text; no behavioral change to successful remote calls.

No team manifest format change, no DB migration, no breaking change to `a2a_call`'s contract for legitimate remote callers. Deployed teams keep working; their orchestrators simply lose a tool they should never have used and gain clearer instructions.

## Implementation steps

### Step 1 — ADR: record the verdict
**File:** `docs/adr/009-team-engine-local-member-delegation.md` (next sequential after `008-github-identity-separation.md`; verify no collision with mika#1410's pending ADR-009 at implementation time and bump to 010 if taken).

- **Context:** team-mode delegation is structural (decompose→spawn→resume); the orchestrator's array contained `a2a_call`, which is structurally cross-container and 503s on local siblings.
- **Decision:** Shape A — local members are routed by name via decompose JSON; `a2a_call` is suppressed from the team tool array; prompt names the mechanism explicitly; `a2a_call` self-describes as remote-only with a local→`delegate_task` redirect.
- **Rejected:** Shape B (URL-based tool can't see through gateway URLs), Shape C (no remote-member concept in teams; flag would be universally true).
- **Consequences:** orchestrators cannot mis-reach local members; conversation-mode `a2a_call` users get an actionable redirect; composes with mika#1652.

### Step 2 — Layer 1 prompt (`crates/mika-agent/src/teams/prompt.rs`)
Extend the `## Instructions` block in `build_orchestrator_context()` with the reach-mechanism statement (Layer 1 above). Update the existing `prompt.rs` tests (`test_orchestrator_context_includes_team_members` and siblings) to assert the new guidance is present (e.g., the context mentions that the engine spawns assigned members and that `a2a_call` must not be used for team members).

### Step 3 — Layer 2 structural suppression (`crates/mika-agent/src/teams/engine.rs`)
In `init_resources()`, remove `a2a_call` from the team tool registry (pin mechanism per Layer 2 recommendation — prefer registry-level removal; fall back to `apply_agent_tool_visibility` suppression list threaded through `TeamAgentParams`/`run_team_agent`). Add a unit test asserting the team registry's tool definitions do **not** include `a2a_call` (and DO still include the workspace tools + core tools), so a future `default_tools()` change can't silently re-introduce the affordance.

### Step 4 — Layer 3 `a2a_call` hardening (`crates/mika-agent/src/tools/a2a_call.rs`)
Harden the tool `description` (remote-only + local→`delegate_task` pointer) and the SSRF-rejection error messages (loopback/private-IP → name the local-agent alternative). Add unit tests: (a) localhost target returns an error whose text points to `delegate_task`; (b) the description string contains "remote"/"external" and "delegate_task". (`a2a_call.rs` currently has no embedded tests — add a `#[cfg(test)] mod tests`.)

### Step 5 — Regression coverage
Add a team-engine-level test (alongside the existing `teams` unit tests) asserting that a goal with multiple members produces a decompose JSON path (member sessions spawned by name) and that `a2a_call` is not in the orchestrator's available tools for that run. Where a full team run is too heavy for a unit test, assert the invariant at the registry/prompt seams (Steps 2-3 tests) — the structural seams are the regression-gating surface.

### Step 6 — Docs
Update `crates/mika-agent/CLAUDE.md` (Team Engine / Management Tools sections) to note: team-mode delegation is by decompose-JSON assignment (name-addressed); `a2a_call` is suppressed in team runs and is remote-only; `delegate_task` is the conversation-mode local-delegation tool. Keep `docs/` as the single source of truth (CI `docs-sync` job).

## Verification contract

- `cargo build` and `cargo clippy` clean.
- `cargo test -p mika-agent` passes, including the new `prompt.rs`, `engine.rs`, and `a2a_call.rs` tests.
- Manual/structural assertion: the team orchestrator's LLM tool array (as assembled in `run_team_agent` via `inject_skills_and_resolve_tools` → `apply_agent_tool_visibility`, or via the filtered team registry) does not contain `a2a_call`; the orchestrator context prompt contains the decompose-only reach guidance.
- ADR-009 (or 010) committed and cross-referenced from the PR body.

## Definition of Done

- ADR records the Shape-A verdict with Shape-B/C rejection rationale (AC1, AC2).
- Team-mode orchestrator can no longer call `a2a_call` to reach a member (Layer 2 structural suppression), and the prompt names the decompose mechanism as the exclusive path (Layer 1) (AC2).
- **Unit test asserts `a2a_call` is not present in the tool definitions returned to the team orchestrator LLM** (registry-level assertion against the team tool registry built in `init_resources()`, or the presentation-level array per the Layer 2 mechanism pinned by the implementer). This hard assertion defines the structural boundary so a future change to `default_tools()` or `init_resources()` cannot silently re-introduce the affordance (F1; Step 3 / Step 5 coverage).
- `a2a_call` self-describes as remote-only and redirects local targets to `delegate_task` (AC2 — "what `a2a_call` does when targeted at a local agent": error-with-hint).
- No team manifest schema change; existing manifests work unchanged (AC4).
- Design note in ADR + PR body documents composability with mika#1652 (AC3).
- All new behavior covered by tests; `cargo test -p mika-agent` green.

## Acceptance criteria

Transcribed verbatim from mika#1653 (`## Acceptance criteria (for architect's design pass)`):

- **AC1** — mika-arch produces a design verdict: pick one of A/B/C/other, with scope defined.
- **AC2** — Design includes: what tool a team-mode orchestrator should use to reach a local team member (and how it knows); what `a2a_call` does when targeted at a local agent (succeed-via-routing, error-with-hint, refuse); whether the team manifest needs schema changes.
- **AC3** — Composability with mika#1652 (the reaper-gap fix): the chosen design should compose with terminal-state writing — the reaper catches stuck rows, this design prevents them by routing correctly in the first place.
- **AC4** — Migration: if the design changes existing tool semantics, document the migration path for already-deployed team manifests.

How this plan satisfies them:
- **AC1** → Design verdict section: **Shape A** chosen, scope = three layers (prompt, structural suppression, `a2a_call` hardening) + ADR; Shapes B and C rejected with rationale.
- **AC2** → Reach tool = decompose-JSON task assignment, name-addressed, engine-spawned (Layer 1 + the "Current state" mechanism). `a2a_call`-on-local = **error-with-hint** in conversation mode (Layer 3) and **not presented at all** in team mode (Layer 2). Manifest schema change = **none** (Shape C rejected).
- **AC3** → "Composability with mika#1652" section: prevention (this) + containment (#1652), disjoint surfaces, no ordering dependency.
- **AC4** → "Migration" section: none required; all layers additive/presentation-scoped; deployed manifests unchanged.

## Out of scope

- **The reaper / terminal-state writer** — sibling mika#1652.
- **Team-run completing without delegation** (run #1 solo behavior) — ticket observation D; resurfaces only if n=2.
- **Why `add_team_member` shifted the dispatch** — resolves from Shape A: with the structural decompose path enforced and `a2a_call` removed, roster size no longer steers the orchestrator toward a URL-based reach tool.
- **Cross-container team members** — teams hold only local agents today; a future remote-member feature would revisit Shape C on its own merits (and is not blocked by this design).

## Revision history

- rev 2 (2026-06-30): addressed F1 by adding an explicit Definition-of-Done line requiring a unit test that asserts `a2a_call` is absent from the tool definitions returned to the team orchestrator LLM (registry- or presentation-level per the pinned Layer 2 mechanism), making the structural boundary a hard, regression-gated assertion rather than only narrative coverage in Step 3/Step 5; citation preserved to review-guide.md § Orthogonality. F2 (missing `## Acceptance criteria` header) was already satisfied in the rev-1 content — the plan carries a standalone top-level `## Acceptance criteria` section transcribing AC1–AC4 verbatim from mika#1653 with the "How this plan satisfies them" mapping beneath it; no structural change required.
