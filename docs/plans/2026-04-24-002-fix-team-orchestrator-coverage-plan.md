---
title: "Fix: Team orchestrator doesn't distribute work across all agents"
type: fix
status: active
date: 2026-04-24
---

# Fix: Team orchestrator doesn't distribute work across all agents

## Overview

The team engine has no contract requiring the orchestrator to account for the team's roster. Given a multi-member team and a goal that plausibly benefits from diverse perspectives, the orchestrator can assign to one or two members and silently ignore the rest, and the engine accepts the partial assignment unchanged. The fix adds the missing contract: a minimal prompt instruction that asks the orchestrator to consider every member, and a coverage check that re-prompts once when the response silently omits members — falling through with a `warn!` log on second failure. No new schema, no new public enum variant, no new persisted data structures.

**Defer deliberately:** structured skip reasons, dashboard surfacing, and response-wrapper schema. Add them when the warn-log signal proves the retry isn't enough, or when a concrete consumer materializes.

## Problem Frame

`build_orchestrator_context` in `crates/mika-agent/src/teams/prompt.rs` lists team members but never tells the orchestrator to account for all of them. `parse_task_assignments` in `crates/mika-agent/src/teams/engine.rs:1437` validates each assignment (agent name, path safety, length) but has no coverage check. On creative or brainstorming goals, the path of least resistance is to pick the one or two most technically-framed members and skip the rest.

The cited failure (run `fd7ef7ef`, inner-circle, 5-agent team) is one observed instance — orchestrator + 1 specialist active, 3 members unused on a username-brainstorming goal that plainly matched their mandates. The fix addresses the engine contract gap, not a model-specific quirk.

## Requirements Trace

- **R1.** The orchestrator must be prompted to consider every non-orchestrator member before responding. This applies across providers — the contract is engine-level.
- **R2.** If the response silently omits members (neither assigns to them nor explains the omission in free-form reply text), the engine re-prompts once with an explicit list of unaccounted members.
- **R3.** A coverage-retry event must be observable — minimum a `warn!` log line and a boolean signal on the run — sufficient for post-hoc "did the retry fire?" debugging via log grep or `sqlite3`.
- **R4.** The fix must not regress single-member teams, focused single-domain goals that legitimately need one agent, or the conversational-reply path.

## Scope Boundaries

### In scope

- `build_orchestrator_context` in `crates/mika-agent/src/teams/prompt.rs` — one-sentence addition
- A coverage-check helper used inside `parse_task_assignments`'s caller (`decompose`) — returns missing member names, no public enum change
- One re-prompt pass in `decompose` when the helper returns non-empty missing set
- A `coverage_retry_fired: bool` signal — ideally on `TeamRun`, else a scoped `warn!` with structured fields
- Unit tests for the coverage helper; one integration test via `MockLlmProvider` for the retry path

### Deferred to Separate Tasks

- **Callback consolidation (#287):** different bug class
- **Structured `skipped` / `SkipEntry` schema:** add if/when the warn-log rate stays non-trivial after this ships, or when a dashboard/analytics consumer exists
- **Per-member mandate-fit scoring:** larger evaluation feature, out of scope
- **Provider-specific tuning:** if a particular provider's retry rate is an outlier after this lands, a per-provider prompt variant is the established escalation path (see the provider-variant pattern in `crates/mika-agent/CLAUDE.md` Skills System). Not this plan's concern.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/teams/prompt.rs:14-124` — `build_orchestrator_context`
- `crates/mika-agent/src/teams/engine.rs:1437-1525` — `parse_task_assignments`
- `crates/mika-agent/src/teams/engine.rs:674` onward — `decompose()` and its `DecomposeResult` match arms (8 call sites — reason to keep the gap signal internal rather than a new public variant)
- `src/agent_loop.rs` post-condition retry pattern — structurally similar, one retry then fall through

### Institutional Learnings

- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — prior "prompt-only is fragile, add a code guard" precedent. Partially applies: the guard here doesn't block calls, it nudges and retries.
- `feedback_prompt_enforcement_fragile` — don't rely on prompt alone for hard constraints. This fix *does* rely on the prompt to produce the right behavior the first time; the structural layer only activates on observed miss. Acceptable because the fallback is a retry, not a silent pass.

## Key Technical Decisions

- **Prompt first, structural as backstop.** One-sentence addition to the orchestrator prompt is the primary lever. Coverage retry activates only when the prompt fails. **Rationale:** the prompt is what shapes the output, and a roster-awareness instruction is the missing piece. The structural retry is an engine-level safety net for when the prompt isn't enough — we observe its fire rate via `warn!` logs post-ship. If it's silent, the prompt is doing the work; if it fires often, we have data to escalate to structured schemas (deferred above).
- **Gap signal stays internal — no new `DecomposeResult` variant.** `parse_task_assignments` keeps its current two-arm signature; the coverage helper is called from `decompose()` against a successful `Tasks` result. **Rationale:** 8 call sites match on `DecomposeResult` today and don't care about coverage. Widening the enum would force churn at every site. SRP.
- **Re-prompt uses free-form reply, not a new schema.** The nudge says: "You did not assign tasks to: [names]. Either include them or explicitly say why they're not relevant — it's fine to skip members whose mandate doesn't fit the goal." Response is still parsed by the existing logic. **Rationale:** no schema contract to maintain, no wrapper type, no prompt example to keep in sync. If the second response still has missing members, we treat any plausible free-form justification as sufficient — coverage retry is about acknowledgement, not machine-readable reasons.
- **One-shot retry, then fall through with `warn!`.** Matches `agent_loop.rs` post-condition pattern. Hard cap on cost.
- **Observability is a single boolean + a `warn!` line.** No persistence schema change. `TeamRun.coverage_retry_fired: bool` gets serialized via the existing `team_runs.checkpoint` JSON column (additive, backward-compatible). Debugging "did this run's orchestrator miss members?" = grep the log for `team_coverage_gap` with run id. **Rationale:** Explicitly YAGNI — structured skip data is not consumed anywhere yet; build it when a consumer needs it.
- **Prompt tests assert semantics, not wording.** Test checks that the prompt contains a roster-accounting instruction (e.g., searches for "account for" or "every team member"), not a specific literal string. **Rationale:** less brittle, survives wording tweaks.
- **Validate prompt-only effect before merging structural.** As a Phase-0 sanity check, apply just the prompt addition against a reproduction of the failing scenario (5-agent team, creative goal) and record whether coverage improves. Informs whether the structural layer earns its keep right now, or whether it's purely defensive.

## Open Questions

### Resolved During Planning

- **Is a wrapper schema needed now?** No — defer. Simple missing-name list in the re-prompt is enough; the LLM's free-form response already gets parsed by existing code.
- **Is this a provider-specific issue?** Insufficient evidence. The cited failure is one run. The contract gap is real regardless; the fix targets the contract, not any single provider's instruction-following.
- **Should `CoverageGap` be a new `DecomposeResult` variant?** No. 8 match sites, none care. Keep the gap signal internal to `decompose()`.
- **Should retry metadata include structured skip reasons?** No (R3). A bool + warn log with run id and missing members is enough until a consumer requests more.

### Deferred to Implementation

- **Where `coverage_retry_fired` lives on `TeamRun`.** Confirm `team_runs.checkpoint` round-trips new optional fields without migration (it's free-form JSON), else relegate to the `warn!` signal only.
- **Whether the missing-member list should be truncated.** Practically teams are ≤10 members; probably unnecessary.

## Implementation Units

**Sequencing:** Ship Unit 1 (prompt) alone as the first commit. Reproduce the failing scenario against the prompt-only build (see Validation Gate below) and record the result in the PR description. Only then commit Unit 2 (structural retry). This orders the evidence: we'll know whether the prompt is sufficient before spending any complexity budget on the retry path. Unit 3 (observability) lands with Unit 2.

- [ ] **Unit 1: Prompt reinforcement (ships first)**

**Goal:** Bias the orchestrator toward roster-aware responses on the first try so the retry path — if we build one — rarely fires.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/teams/prompt.rs` — add one sentence to the `## Instructions` block in `build_orchestrator_context`
- Test: `crates/mika-agent/src/teams/prompt.rs#tests` — add a semantic assertion

**Approach:**
- Insert this exact sentence after the existing "decompose into tasks for your team members" line, before the response-format block:

  > *"Consider every team member's mandate before responding. It's fine to leave a member out if their expertise doesn't fit the goal, but make that decision deliberately — don't default to the first one or two that come to mind."*

- Wording is locked so the test assertion below is coherent with what ships. Do not rephrase without updating the test.
- No schema change. No example change. Existing response format (`[{agent, task, output_file}]` or `{reply: "..."}`) preserved verbatim.

**Patterns to follow:**
- Existing `## Instructions` block scaffolding in `build_orchestrator_context`
- Existing assertion style in `test_orchestrator_context_includes_team_members`

**Test scenarios:**
- **Happy path:** Prompt contains both `"every"` (case-insensitive) and `"member"` (case-insensitive). Assertion: semantic-pair match, not the whole literal sentence.
- **Regression guard:** Existing `test_orchestrator_context_includes_team_members` still passes
- **Regression guard:** Response-format block ("Respond with a JSON array" / "Respond ONLY with compact") still present

**Verification:**
- `cargo test -p mika-agent teams::prompt` passes including the new assertion.

---

**Validation Gate (between Unit 1 and Unit 2):** Before writing any of Unit 2, reproduce the failing scenario — a 5-member team with a creative/brainstorming goal like run `fd7ef7ef`'s username task — against a build that has Unit 1 but not Unit 2. Record in the PR description:

- Whether the orchestrator assigned tasks to >50% of members (vs 1/4 in the original failure)
- If the behavior improved materially on its own, note it — the structural layer is then quiet defense-in-depth, not the active lever
- If it didn't improve, proceed to Unit 2 with that evidence in hand

This is a manual observation step, not an automated test. One run is enough to inform the scope call; we're not doing a statistical study.

---

- [ ] **Unit 2: Coverage-check helper + `decompose()` retry integration**

**Goal:** Detect silent omissions after `parse_task_assignments` returns `Tasks(...)`, re-prompt once with the missing list, fall through with a `warn!` log on second failure. No changes to `DecomposeResult` or `parse_task_assignments` signature.

**Requirements:** R2, R3, R4

**Dependencies:** Unit 1 (sequencing, not code dependency — ships in the same PR but after)

**Files:**
- Modify: `crates/mika-agent/src/teams/engine.rs` — add a private `missing_members(tasks: &[TaskAssignment], team: &TeamDefinition) -> Vec<String>` helper; update `decompose()` to call it after `Tasks(...)` parse and branch on non-empty missing set
- Modify: `crates/mika-agent/src/teams/engine.rs` — add a private `retry_once_with_coverage_nudge` helper (single responsibility) that builds the nudge message, issues the extra LLM call, and re-parses
- Test: `crates/mika-agent/src/teams/engine.rs` — inline test module covers `missing_members` as a pure function
- Test: `crates/mika-agent/tests/eval/team_coverage.rs` (new) — integration test covers the retry path through `decompose()`

**Approach:**
- `missing_members()` computes `expected = team.agents.filter(name != orchestrator).name` and `assigned = tasks.agent`, returns `expected - assigned` as owned `Vec<String>`
- In `decompose()`, after parsing to `Tasks(tasks)`: if `missing_members(&tasks, team).is_empty()`, proceed as today. Else call `retry_once_with_coverage_nudge(...)` which:
  1. Formats a short nudge: `"You did not assign tasks to: {missing}. Either include them or add a brief note in a `reply` explaining why they're not relevant to this goal. It's fine to skip members whose mandate doesn't fit, but the response must reflect that you considered them."`
  2. Issues one more LLM turn with the extended conversation history
  3. Re-parses the new response with `parse_task_assignments`
  4. Returns the second response's parsed result — see gap→gap decision below
- On second miss (still has missing members): emit the `warn!` (schema below), set `coverage_retry_fired = true`, return the second response's tasks. **The second response wins, even if still partial.** Rationale: the orchestrator saw the nudge; whatever it produced second is its most-recent best effort and strictly more informed than the first. If the second response partially corrected (e.g. added 1 of 3 missing), we want that correction preserved.
- If the retry turn itself returns `Conversational(...)` (e.g. orchestrator reframed everything as a reply), treat it as the final answer — no further nudging. Emit the `warn!` so the signal is still observable.
- Conversational-reply path from the original turn is unchanged — helper never runs.
- Single-member teams after the orchestrator filter produce an empty `expected` set; helper returns empty; no retry.

**Locked `warn!` schema** (observability contract — do not change field names without a plan update):

```rust
warn!(
    team_run_id = %run.id,
    team_id = %team.id,
    missing_members = ?missing,       // Vec<String>, missing after final (retried) response
    retry_recovered = retry_recovered, // bool: true if retry closed the gap, false otherwise
    initial_missing_count = initial_missing.len(),  // usize
    final_missing_count = missing.len(),            // usize
    "team_coverage_gap"
);
```

- Grep pattern: `grep team_coverage_gap server.log | jq`
- Required for debugging "did retry fire on this run?" via logs when Unit 3 is absent or its field deserializes to `false` on an old row.
- Event name is the message field (`"team_coverage_gap"`), matching existing conventions like `kg_extraction_start`, `kg_budget_exhausted` in this crate.

**Execution note:** Write the integration test's harness skeleton first — see Risks below; building a team-engine-scoped test setup is novel work for this crate, and getting the shape right before writing scenarios keeps scope honest.

**Patterns to follow:**
- Existing single-retry pattern in `src/agent_loop.rs` post-condition guards
- `MockLlmProvider` sequence-based test pattern from `crates/mika-agent/tests/eval/test_*.rs` (note: those target `run_agent()`, not `TeamEngine::execute()` — see Risks)

**Test scenarios:**
- **Happy path (full coverage):** 3-member team, orchestrator emits tasks for all 3. Helper returns empty. Retry never fires. No `warn!` emitted. `coverage_retry_fired = false`.
- **Happy path (single-member team):** Team = orchestrator + 1 specialist. Orchestrator assigns 1 task. Helper returns empty. No retry.
- **Edge case (gap → recover):** 3-member team, first response assigns 1, nudge turn assigns all 3. Tests: exactly one extra LLM call recorded; final `tasks.len() == 3`; `coverage_retry_fired = true`; `warn!` emitted with `retry_recovered = true`, `final_missing_count = 0`.
- **Edge case (gap → partial recovery):** 3-member team, first response assigns 1, nudge turn assigns 2. Tests: final `tasks.len() == 2` (second response wins); `retry_recovered = false` (still missing one); `warn!` emitted with `final_missing_count = 1`.
- **Edge case (gap → gap):** 3-member team, both turns return the same 1-of-3 response. Tests: two LLM calls recorded; final tasks = second response's tasks (one member); `retry_recovered = false`; `warn!` emitted with `missing = [2 names]`.
- **Edge case (gap → conversational):** First turn assigns 1, nudge turn returns `{reply: "on reflection the task doesn't need the full team"}`. Tests: `DecomposeResult::Conversational(reply)` returned; `coverage_retry_fired = true`; `warn!` emitted with `retry_recovered = false` and `final_missing_count = initial_missing_count` (the conversational reply is not treated as closing the gap structurally, but the decision to pivot is respected).
- **Conversational reply path (original turn):** First turn returns `{reply: "..."}`. Helper never called. No retry.
- **Empty tasks:** Existing "no valid assignments" branch still returns `Conversational(...)`. Helper never runs.

**Verification:**
- `cargo test -p mika-agent teams::engine` passes (new helper unit tests + existing parse tests unchanged)
- `cargo test -p mika-agent --test eval team_coverage` passes
- Manual sanity: `grep team_coverage_gap server.log | jq '.fields'` in staging after a team run; field shape matches the locked schema.

---

- [ ] **Unit 3: `coverage_retry_fired` on `TeamRun` (if trivial)**

**Goal:** Expose the retry signal beyond the log line — a boolean post-hoc tooling can query without parsing logs.

**Requirements:** R3

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/src/teams/types.rs` — add `#[serde(default)] coverage_retry_fired: bool` to `TeamRun`
- Modify: `crates/mika-agent/src/teams/engine.rs` — populate when Unit 2's retry fires
- Test: integration test in `tests/eval/team_coverage.rs` — after gap-triggered run, `team_run.coverage_retry_fired == true`; after clean run, `false`

**Approach:**
- Proceed only if `team_runs.checkpoint` already round-trips the `TeamRun` struct as JSON (confirm via quick grep in `db.rs` before starting). If it does, `#[serde(default)]` keeps existing rows deserializing — no migration.
- If persistence requires a schema change, **drop this unit** — the `warn!` from Unit 2's locked schema is the observability fallback and fully meets R3.

**Patterns to follow:**
- Existing `#[serde(default)]` fields on `TeamRun` (if any); `db.rs` checkpoint round-trip path

**Test scenarios:**
- **Integration (gap-triggered):** After `MockLlmProvider` sequence triggers retry, `team_run.coverage_retry_fired == true`
- **Integration (clean):** Full-coverage first response → `coverage_retry_fired == false`
- **Backward compat:** Deserialize a `team_runs.checkpoint` row written before this field existed → `coverage_retry_fired == false`

**Verification:**
- Integration tests pass. Spot-check: `sqlite3 ~/.mika/data/mika.db "SELECT json_extract(checkpoint, '$.coverage_retry_fired') FROM team_runs ORDER BY started_at DESC LIMIT 5"`.

## System-Wide Impact

- **Interaction graph:** `decompose()` gains one optional extra LLM turn. `DecomposeResult` signature and consumers are unchanged.
- **Error propagation:** Coverage gap is absorbed inside `decompose()`; external callers see the existing `Tasks(...)` or `Conversational(...)` only.
- **State lifecycle risks:** One extra LLM call per orchestrator turn in the worst case; single retry bound.
- **API surface parity:** No external API change — `run_team` tool, A2A task shape, dashboard endpoints unchanged.
- **Integration coverage:** New tests sit next to existing `tests/eval/` scenarios, use the same `MockLlmProvider` harness.
- **Unchanged invariants:** `TaskAssignment` fields, `parse_task_assignments` signature, `DecomposeResult` variants, `{reply: "..."}` envelope, and all existing engine tests.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Prompt change alone fixes the observed cases — structural layer is idle defense. | This is the intended outcome; the Validation Gate surfaces this before Unit 2 is written. `warn!` frequency post-ship confirms. If perpetually silent, Unit 2 is quiet defense-in-depth at low cost; if it fires often, we have data to escalate. |
| Retry rate stays high (orchestrator ignores both prompt and nudge). | Observable via `warn!` log. Next escalation: structured `skipped` schema (deferred) or per-provider prompt variant. Both explicitly out of scope for this plan. |
| **No existing team-engine test harness.** `tests/eval/*` targets `run_agent()` on single agents; nothing scripts a full `TeamEngine::execute()` with a mock LLM sequence. Unit 2's integration tests therefore need a small new harness (team definition builder, workspace tmpdir, mock LLM, stub/fake dispatcher for specialist turns if feasible, or a team engine variant that lets us observe `decompose()` output without running specialists). | Build the harness skeleton first (before scenarios). If it balloons beyond ~150 lines, narrow Unit 2's integration tests to `decompose()`-only (with a fake LLM passed in via dependency injection) and defer full-engine coverage to a follow-up. Unit-level tests on `missing_members()` remain unaffected. |
| Unit 3's serde-default field breaks existing deserialization. | `#[serde(default)]` guarantees backward compat; unit test covers pre-existing-row case; fallback is to drop Unit 3 and rely on `warn!` for R3. |
| Gap→gap behavior could be surprising — first-response vs second-response choice affects which tasks run. | Explicit decision documented in Unit 2's approach: **second response wins.** Test scenarios cover all three gap→* transitions. If this turns out to be the wrong call (orchestrator regresses on retry more often than it corrects), flip to first-response-wins and update scenarios. |

## Documentation / Operational Notes

- Add a one-sentence note to `crates/mika-agent/CLAUDE.md` under the team-engine section: "Coverage check in `decompose()` re-prompts once if the orchestrator silently omits members; falls through with `warn!` log on second miss."
- No deployment, migration, or rollout concerns.

## Sources & References

- **Origin issue:** [senara-solutions/mika#286](https://github.com/senara-solutions/mika/issues/286)
- Related code: `crates/mika-agent/src/teams/prompt.rs`, `crates/mika-agent/src/teams/engine.rs`
- Related pattern: `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md`
- Related memory: `feedback_prompt_enforcement_fragile`
- Observed failure: team run `fd7ef7ef` — one concrete instance of the contract gap, not a provider-specific conclusion
