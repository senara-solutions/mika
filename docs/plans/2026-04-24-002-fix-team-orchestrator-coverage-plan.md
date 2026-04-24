---
title: "Fix: Team orchestrator doesn't distribute work across all agents"
type: fix
status: active
date: 2026-04-24
---

# Fix: Team orchestrator doesn't distribute work across all agents

## Overview

Team orchestrators consistently delegate to only one or two team members, leaving the rest idle. This defeats the purpose of assembling a team with distinct roles. The fix adds a **structural roster-awareness guard** in the orchestrator's assignment parse path plus complementary prompt reinforcement — the structural layer is the primary defense because prompt-only constraints have been shown to be fragile here (see `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md`).

## Problem Frame

Orchestrators receive a team roster + goal and are told to "decompose into tasks for your team members" (see `crates/mika-agent/src/teams/prompt.rs:110-120`). The prompt never asks the LLM to account for the whole roster. When given a creative/brainstorming goal, the path-of-least-resistance is to pick the one or two most technically competent-sounding members and skip the rest. This happened on team run `fd7ef7ef` (inner-circle, 5 agents): orchestrator (`steve-jobs`) + 1 specialist (`mika-dev`) were active; `elon-musk`, `chase-hughes`, and `mika-qa` were never invoked, even though the goal ("brainstorm GitHub usernames") directly matched their mandates.

Downstream: `parse_task_assignments()` in `crates/mika-agent/src/teams/engine.rs:1437` already validates agent names, path safety, and task length. It has no coverage check — an assignment list that names one agent passes through unchanged.

## Requirements Trace

- **R1.** Orchestrator must make an explicit decision about every non-orchestrator team member on every actionable turn — either assign a task or record why the member is being skipped.
- **R2.** A team run must fail the parse and retry (one nudge) if the orchestrator's response silently omits members without recording skip reasons.
- **R3.** Coverage signal must be observable — team run metadata should expose how many members were assigned vs skipped, with skip reasons available for debugging.
- **R4.** Fix must not regress legitimate single-member assignments when the team has only one non-orchestrator specialist, or when the goal is a focused single-domain task that truly needs one agent.

## Scope Boundaries

### In scope

- `parse_task_assignments()` in `crates/mika-agent/src/teams/engine.rs`
- Orchestrator system prompt in `crates/mika-agent/src/teams/prompt.rs`
- Assignment schema change: add `skipped: Vec<SkipEntry>` alongside the existing `tasks` array
- One-retry nudge when coverage check fails (mirrors existing post-condition retry pattern in `src/agent_loop.rs`)
- Unit tests for the coverage check; integration test via team engine with a mocked LLM

### Deferred to Separate Tasks

- **Callback consolidation (#287):** different bug class (delivery shape, not assignment) — handled in its own plan on `feat/287/...`
- **Orchestrator rework for parallel vs sequential execution:** out of scope; orchestrator's execution ordering is unchanged
- **Per-member role-fit scoring / mandate-matching LLM judge:** would be a larger evaluation feature; this plan uses the simpler structural contract (explicit skip reasons) which catches the observed failure mode

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/teams/prompt.rs:14-124` — `build_orchestrator_context`, where the roster and assignment instructions are assembled
- `crates/mika-agent/src/teams/engine.rs:1427-1525` — `DecomposeResult` enum + `parse_task_assignments`, the structural choke point
- `crates/mika-agent/src/teams/engine.rs:674` — `decompose()` method that calls `parse_task_assignments` and consumes the result
- `crates/mika-agent/src/teams/types.rs` — `TaskAssignment` struct; add `SkipEntry` alongside
- `crates/mika-agent/src/agent_loop.rs` — post-condition retry pattern (guards that reject once and re-prompt) used by required-tools gate, completion-claim guard, etc. Team engine does not have an identical retry harness; we'll implement one pass of re-prompt inline in `decompose()`

### Institutional Learnings

- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — prior precedent: when prompt-only enforcement of "always track delegated work" failed, the fix was a code-level guard on `delegate_task` that rejects the call if a work item isn't referenced. Same playbook applies here: structural guard beats prompt reminder.
- `docs/solutions/architecture-patterns/conditional-required-tools-enforcement-via-match-reason.md` — another structural-over-prompt pattern, this one for tool-use constraints keyed on skill match reason.

### External References

Not needed. The bug and fix are entirely internal to the team engine; no external best-practice research materially sharpens the plan.

## Key Technical Decisions

- **Primary mechanism is structural, not prompt-only.** Coverage is enforced in `parse_task_assignments`, not only requested in `build_orchestrator_context`. **Rationale:** prompt-level coverage requests have a known failure mode (see `feedback_prompt_enforcement_fragile` — LLMs rationalize skipping members that look less useful). A structural contract forces acknowledgement of every member.
- **The contract is "record a skip, don't silently omit".** The orchestrator response schema is extended to `{tasks: [...], skipped: [{agent, reason}, ...]}`. The guard verifies `tasks.agents ∪ skipped.agents == team_members`. **Rationale:** we do NOT force 100% team coverage. A focused goal legitimately skips some members; we just require the skip to be explicit. This preserves legitimate single-member assignments (R4).
- **Failure mode is one-shot retry with feedback, then fall through.** If the response misses members and has no skip entries, the guard re-prompts once with a nudge listing the unaccounted members. On second failure the run proceeds with whatever was parsed (logging a warning) rather than hard-erroring. **Rationale:** matches the existing post-condition guard pattern in the agent loop; hard-erroring would block legitimate runs during a provider hiccup.
- **Backward-compatible parse.** Responses that emit only `tasks` (array of assignments, no wrapper) still parse as today — the guard triggers only when members are missing from both the assigned and skipped sets. **Rationale:** existing teams, tests, and old call sites keep working; only the failure mode changes.
- **Skip entries are persisted for observability.** `TeamRun` gains a `skipped_members: Vec<SkipEntry>` field that is surfaced on the dashboard's team-run detail view. **Rationale:** R3 — skip reasons are how we debug "why wasn't agent X used?" in the future.
- **Prompt reinforcement is additive, not load-bearing.** The orchestrator prompt is updated with roster-matching guidance and a required `skipped` field in the response schema. This makes the schema discoverable but the structural guard is what actually enforces it.

## Open Questions

### Resolved During Planning

- **Q: Should the coverage check require a minimum fraction (e.g. ≥50%) of members to be assigned?**
  Resolution: No. Require explicit accounting (assign or skip-with-reason) instead. Percentage thresholds don't distinguish legitimate focus from lazy under-coverage. The explicit-skip contract catches the observed bug (silent omission) without blocking legitimate focused goals.
- **Q: Should the guard apply to the orchestrator itself?**
  Resolution: No. The orchestrator is never a target of its own assignments (current behavior preserved — `build_orchestrator_context` filters the orchestrator from the roster).
- **Q: Should we retry more than once?**
  Resolution: No. One re-prompt matches the post-condition guard pattern and bounds cost. Persistent failure falls through with a warning.

### Deferred to Implementation

- **Exact wording of the orchestrator prompt addition (roster-awareness hint).** Will be settled when writing prompt.rs — test the two obvious phrasings against the failing run's setup before committing to one.
- **Whether skip reasons should be capped in length.** Likely yes (same 5000-char cap as task descriptions in `engine.rs:1495`), but will finalize when implementing.
- **Exact field name on `TeamRun` for surfaced skip metadata.** `skipped_members` is likely fine; will check if `teams/types.rs` already has a naming convention that should be followed.

## Implementation Units

- [ ] **Unit 1: Extend response schema with `SkipEntry` and wrapper object**

**Goal:** Add the data types that represent the new orchestrator response shape, without changing parse behavior yet.

**Requirements:** R1, R3

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/teams/types.rs` — add `SkipEntry { agent: String, reason: String }` and optional `OrchestratorResponse { tasks: Vec<TaskAssignment>, skipped: Vec<SkipEntry> }` wrapper types
- Test: `crates/mika-agent/src/teams/types.rs` — inline `#[cfg(test)] mod tests` for serde round-trips

**Approach:**
- Add `SkipEntry` as a simple struct with `#[derive(Debug, Clone, Serialize, Deserialize)]`
- Keep `TaskAssignment` unchanged — the skip list is a sibling, not a replacement

**Patterns to follow:**
- Mirror the existing `TaskAssignment` struct's derive set and naming

**Test scenarios:**
- Happy path: `SkipEntry { agent: "alice", reason: "goal is backend-only" }` round-trips through JSON
- Happy path: deserializing a `{"tasks": [...], "skipped": [...]}` wrapper preserves both arrays
- Edge case: deserializing a bare `[...]` array without the wrapper still yields `Vec<TaskAssignment>` (backward compatibility)

**Verification:**
- `cargo test -p mika-agent teams::types` passes.

---

- [ ] **Unit 2: Coverage check in `parse_task_assignments`**

**Goal:** Detect silent omissions — members who appear in the team roster but neither got a task nor were recorded as skipped.

**Requirements:** R1, R2, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/teams/engine.rs` — teach `parse_task_assignments` to parse the wrapper form and return a new `DecomposeResult::CoverageGap { missing: Vec<String>, partial_tasks: Vec<TaskAssignment>, partial_skips: Vec<SkipEntry> }` variant when members are unaccounted
- Test: `crates/mika-agent/src/teams/engine.rs` — inline test module `parse_coverage_tests`

**Approach:**
- After the existing assignment-collection loop, compute `accounted = tasks.agents ∪ skips.agents`, `expected = team_members_excluding_orchestrator`, `missing = expected - accounted`
- If `missing` is non-empty AND the response *did* attempt assignments (i.e. we have at least one assignment or skip), return `CoverageGap` with what was parsed; the caller (`decompose`) decides whether to retry
- If `missing` is empty, return `Tasks(tasks)` as today (no behavior change)
- Pure conversational-reply path is unchanged — the guard only fires on assignment-shaped responses

**Patterns to follow:**
- Existing `DecomposeResult::Conversational` short-circuit for non-actionable replies
- Existing `agent_names` collection at `engine.rs:1463`

**Test scenarios:**
- Happy path: full-coverage response (every member assigned or skipped) → `Tasks` result, no gap
- Happy path: single-member team assigned one task → `Tasks` result, no gap (no missing members)
- Edge case: bare array response covering all members → `Tasks` (backward compat)
- Edge case: bare array response covering 1 of 3 members → `CoverageGap { missing: [2 members], partial_tasks: [1], partial_skips: [] }`
- Edge case: wrapper with `tasks=[1]`, `skipped=[1]`, team has 3 members → `CoverageGap { missing: [1] }`
- Error path: empty assignments AND empty skips (existing "no valid assignments" branch) → still returns `Conversational` (unchanged)

**Verification:**
- New unit tests pass; existing `parse_task_assignments` tests still pass unchanged.

---

- [ ] **Unit 3: One-retry nudge in `decompose`**

**Goal:** On `CoverageGap`, re-prompt the orchestrator once with an explicit list of unaccounted members; on second failure, fall through with a warning and proceed with whatever was parsed.

**Requirements:** R2

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/src/teams/engine.rs` — update `decompose()` (around line 674) to handle `DecomposeResult::CoverageGap` by running one extra LLM call with a nudge prompt, then re-parsing
- Test: `crates/mika-agent/tests/eval/team_coverage.rs` (new) — integration test using `MockLlmProvider` sequence to simulate gap-then-recover and gap-then-gap-falls-through

**Approach:**
- Add a small `retry_once_with_coverage_nudge` helper in `engine.rs` that:
  - Takes the orchestrator context, the `missing` list, and a formatted partial response from the first turn
  - Builds a short follow-up user message: "Your previous response did not account for these team members: {missing}. For each, either assign a task or add an entry to the `skipped` array with a clear reason."
  - Calls the LLM once with the extended history
  - Re-runs `parse_task_assignments` on the new response
  - On success → `Tasks`
  - On repeat `CoverageGap` → fold partial results together, log `warn!` with member list and reasons (for observability), and return `Tasks(partial_tasks)` — the run proceeds with what the orchestrator actually produced
- Counter incremented in audit events for visibility (Unit 5)

**Execution note:** Write the integration test first so the retry shape is anchored to observable behavior, not implementation detail.

**Patterns to follow:**
- Existing single-retry pattern in `src/agent_loop.rs` post-conditions (e.g. required-tools gate)
- Existing `execute_inner` control flow in `teams/engine.rs`

**Test scenarios:**
- Integration (gap → recover): mock LLM returns `{tasks: [a], skipped: []}` with 3-member team, nudge turn returns `{tasks: [a,b], skipped: [{c, reason}]}`. Expect: 2 tasks created, 1 skip recorded, no warning.
- Integration (gap → gap): mock LLM returns the same under-covered response both times. Expect: 1 task created, partial skips recorded, `warn!` with member list emitted, run proceeds.
- Integration (pure conversational): mock LLM returns `{reply: "..."}`. Expect: no coverage check, no retry, conversational path unchanged.
- Edge case (empty team after orchestrator filter): 1-agent team = orchestrator only. No non-orchestrator members to assign. Expect: no coverage check triggered (missing set is empty).

**Verification:**
- New `team_coverage.rs` integration tests pass
- Existing team-engine tests in `tests/eval/` still pass

---

- [ ] **Unit 4: Update orchestrator prompt with roster-awareness guidance and schema**

**Goal:** Make the new response schema discoverable and bias the LLM toward explicit roster accounting on the first try, so the retry path is a backstop rather than the common case.

**Requirements:** R1 (complementary), R4

**Dependencies:** None (prompt change is independent of parse change)

**Files:**
- Modify: `crates/mika-agent/src/teams/prompt.rs` — extend `build_orchestrator_context` with roster-matching instructions and the new response schema (both `tasks` and `skipped`)
- Test: existing prompt tests (`teams/prompt.rs` lines 372+) — update assertions to check the new guidance is present, add a test for the `skipped` schema being mentioned

**Approach:**
- Add a "Roster awareness" section to the instructions block: "For each team member, decide: assign a task (if their mandate helps with the goal), or skip them (if their mandate is unrelated). Every member must be accounted for in either `tasks` or `skipped`."
- Update the response schema block: `{"tasks": [{assignment, ...}], "skipped": [{"agent": "<name>", "reason": "<why>"}]}`
- Update the "Examples" block with one example showing a partial skip
- Preserve the conversational-reply envelope and existing "list_workspace" instruction verbatim

**Patterns to follow:**
- Existing prompt scaffolding in `build_orchestrator_context`
- Existing test harness in `teams/prompt.rs#tests`

**Test scenarios:**
- Happy path: prompt contains "skipped" schema keyword, team member list is present
- Happy path: existing `test_orchestrator_context_includes_team_members` still passes
- Happy path: new `test_orchestrator_context_mentions_skipped_schema` test

**Verification:**
- `cargo test -p mika-agent teams::prompt::tests` passes, including the new assertion.

---

- [ ] **Unit 5: Observability — record coverage metadata on `TeamRun`**

**Goal:** Make coverage gaps visible after the fact so we can tell whether the retry fired, which members were skipped, and why.

**Requirements:** R3

**Dependencies:** Unit 3

**Files:**
- Modify: `crates/mika-agent/src/teams/types.rs` — add `skipped_members: Vec<SkipEntry>` and `coverage_retry_fired: bool` to `TeamRun`
- Modify: `crates/mika-agent/src/teams/engine.rs` — populate these fields when coverage retry runs
- Modify: `crates/mika-agent/src/db.rs` — if `TeamRun` is persisted through this layer, serialize the new fields to the `team_runs.checkpoint` JSON column (no schema migration needed; JSON column is free-form)
- Test: `crates/mika-agent/src/teams/engine.rs` — assert that after a `CoverageGap` run, `team_run.skipped_members` contains the recorded skips

**Approach:**
- Inspect `db.rs` to confirm `team_runs.checkpoint` is the JSON blob that round-trips `TeamRun`. If so, serialization is transparent — no schema migration.
- Dashboard consumption is a future concern; this unit only guarantees the data is captured and visible via `gh` + sqlite query.

**Test scenarios:**
- Integration: after a team run that fired the retry, the final `TeamRun` has `coverage_retry_fired = true` and `skipped_members.len() > 0`
- Integration: a clean run has `coverage_retry_fired = false` and empty `skipped_members`

**Verification:**
- Integration tests pass. Spot-check via `sqlite3 ~/.mika/data/mika.db 'SELECT checkpoint FROM team_runs ORDER BY started_at DESC LIMIT 1'` during manual testing.

## System-Wide Impact

- **Interaction graph:** Orchestrator LLM call path in `decompose()` gains a one-turn-retry branch. Nothing else changes.
- **Error propagation:** `CoverageGap` is absorbed internally; external callers (`execute_inner`, `run_team` tool, dashboard) only see the existing `Tasks(...)` or `Conversational(...)` outcomes, now potentially supplemented by `team_run.skipped_members`.
- **State lifecycle risks:** None — the retry is bounded to one extra LLM call; persistence happens only once after the final parse succeeds.
- **API surface parity:** No external API changes. `run_team` tool schema and A2A task shape are unchanged.
- **Integration coverage:** New integration tests live next to the existing eval tests and use the same `MockLlmProvider` harness.
- **Unchanged invariants:** Existing single-assignment team runs still work. Existing tests continue to pass. Prompt still emits conversational replies via `{reply: "..."}` envelope. `TaskAssignment` fields and validation rules (name, path, length) are unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| LLM ignores the new `skipped` schema entirely and keeps emitting bare arrays. | Guard explicitly supports bare-array responses; retries only when the parse actually finds a gap. Prompt reinforcement biases toward the new schema but does not depend on it. |
| Retry loop becomes expensive for large teams where the LLM struggles to produce full coverage. | Hard cap: one retry. On second failure, run proceeds with whatever was parsed. Cost bound is one extra LLM call per decompose phase. |
| Legitimate single-agent goals get false-flagged as gaps. | Contract is "account for each member", not "assign to each member". Skipping with a reason is always valid. Prompt explicitly spells this out. |
| Schema serialization drift on `team_runs.checkpoint`. | The checkpoint column is free-form JSON; adding fields is backward compatible. Old rows deserialize with `#[serde(default)]` yielding empty vec / false. |
| `MockLlmProvider` sequence-based tests become brittle if the retry path is tweaked later. | Integration tests assert on observable state (`team_run.skipped_members`, log output) rather than exact LLM call counts where possible. |

## Documentation / Operational Notes

- Update `crates/mika-agent/CLAUDE.md` under the team-engine section to describe the coverage-check contract (one-sentence mention of the `skipped` field and the retry).
- No deployment or migration concerns — purely an internal engine change.
- Dashboard surfacing of `skipped_members` is NOT part of this plan; it's a follow-up tied to milestone #13 team runs work (#652).

## Sources & References

- **Origin issue:** [senara-solutions/mika#286](https://github.com/senara-solutions/mika/issues/286)
- Related code: `crates/mika-agent/src/teams/prompt.rs`, `crates/mika-agent/src/teams/engine.rs`
- Related pattern: `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md`
- Related pattern: `docs/solutions/architecture-patterns/conditional-required-tools-enforcement-via-match-reason.md`
- Related memory: `feedback_prompt_enforcement_fragile` — don't rely on prompt-level enforcement; use structural constraints
- Observed failure: team run `fd7ef7ef` (inner-circle, 5 agents, 2 active) — described in the issue body
