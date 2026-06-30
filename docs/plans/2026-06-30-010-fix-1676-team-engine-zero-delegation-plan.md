---
issue: 1676
type: fix
date: 2026-06-30
---

# Plan — fix(team-engine): team-run completes with zero delegation (mika#1676)

## Problem

A team-run can finish with `team_runs.status='completed'` while delegating to **zero** members — the orchestrator silently absorbs the whole goal and executes it alone. The completed row is indistinguishable from a fully-delegated success, so the failure is invisible without a manual audit. Founding incident: Litha's `odds-engine` team, run `team-49640b61-…`, 2026-06-29 — orchestrator session ran a 20-step agentic loop (21 llm_calls, 37 tool_calls) doing the work itself, with response_text literally saying "Now dispatching to all three specialists in parallel" before never spawning any member.

This is the third defect from the Litha-team-debugging cascade (sibling tickets mika#1652 + mika#1653 already closed); it was previously recorded as an n=1 observation. The original observation's intent-disambiguation criterion ("did the orchestrator try to delegate?") is now answered concretely: yes — the response text explicitly intended delegation, but the parser routed it to Conversational.

## Architectural lineage

- mika#1652 — team_runs orphan reaper (CLOSED) — sibling structural gap, catches stuck runs.
- mika#1653 — a2a_call → 503 + delegate_task semantics (CLOSED) — clarified that team-mode reach is via decompose-JSON, not `a2a_call`.
- #286 — existing coverage-retry mechanism for omitted members in decompose; structurally adjacent to this gap.
- #569 — prose-style tool-call detection (parallel `extract_first_json_object` tolerance applied at agent-loop layer).
- #876 — KG extraction parse tolerance (same shape applied to entity extractor).

## Root cause (three coupled layers, body-confirmed)

1. **Layer 1 — Conversational fallthrough.** `parse_task_assignments()` (`crates/mika-agent/src/teams/engine.rs:1718-1806`) returns `DecomposeResult::Conversational` on any non-clean outcome: missing `[...]` array (1730-1733), `serde_json::from_str` failure (1738-1741), zero validated assignments (1800-1803). Each path emits a `warn!` and otherwise looks normal. Models that narrate prose around their JSON or pretty-print or wrap in markdown land here silently.
2. **Layer 2 — short-circuit to `completed`.** `DecomposeResult::Conversational` short-circuits both the first-decompose path (`engine.rs:656-659`) and re-decompose path (`engine.rs:711-715`) with `self.run.deliverable = Some(reply)` + `return Ok(())`. The execute loop never runs. Caller's `Ok` arm flips `RunStatus::Running → Completed`. Persisted as `status='completed'` by `finalize_and_shutdown()`.
3. **Layer 3 — no delegation gate / no observability column.** `team_runs` schema has no `delegation_count`, no `completed_without_delegation` flag. The completed row is structurally indistinguishable from a fully-delegated success. Manual audit was needed to detect this failure class.

## Fix shape (three coupled units — A primary, B observability backstop, C parse-tolerance reducing how often A has to fire)

### Unit A (primary) — delegation gate

Refuse to transition a team-run to `Completed` when decompose returned `Conversational` for an actionable goal. Architect F1-pinned: fail-open with a **specific committed actionability regex**, not a vague keyword list.

**Actionability detection (architect F1 BLOCKING — pinned):** the gate fires when the orchestrator's decompose response_text matches EITHER:

- **(a)** the regex `(?i)\b(dispatching?|decompos\w*|delegat\w*|assign(?:ed|ing)?\s+(?:to|for)|hand(?:ing|ed)?\s+(?:off|over)\s+to|split\w*\s+(?:work|task)|parallel\w*\s+(?:delegat|specialis|member))\b` (anchored intent-to-delegate keywords), OR
- **(b)** the response contains a JSON-array-shape (`\[\s*\{` followed by `\}\s*\]` later) that failed `serde_json::from_str::<Vec<TaskAssignment>>` validation (intent-to-delegate-via-JSON, parse failed).

Plain prose with neither (a) nor (b) routes to `Conversational` cleanly — preserves the trivial-goal path ("what time is it"). False-negatives in the keyword list are covered by Unit B's observability column. False-positives are bounded: the gate fires one retry, not an immediate fail.

**Gate behavior:**
- **First-decompose path** (`engine.rs:656-659`): On `Conversational` AND actionability-detected, run one decompose retry with a prompt reinforcement ("Emit a JSON array of task assignments. If the goal does not require delegation, respond with `[]`."). On second `Conversational` (still actionability-detected), transition to terminal state `RunStatus::FailedNoDelegation` (architect F2: single terminal state; phase captured in `failure_context` JSON column, not enum proliferation).
- **Re-decompose path** (`engine.rs:711-715`): Same gate, same retry. On terminal failure, also `RunStatus::FailedNoDelegation` with `failure_context = {"phase": "revision_after_critic"}` (architect F2).
- **First-decompose without actionability signals** OR **trivial goals**: route to `Conversational` cleanly (current behavior), the run completes normally as orchestrator-solo, Unit B's `solo_absorption=1` flag still gets set so it's queryable.

**Retry-flag separation from #286:** `coverage_retry_fired` (#286, omitted-member case) and `conversational_retry_fired` (this fix, JSON-shape-failure case) are separate fields on `TeamRun`. Different retry budgets, different correction prompts, cleanly co-existing.

### Unit B (observability backstop) — delegation_count + completed_without_delegation

Add structural visibility so this failure mode is queryable post-hoc regardless of whether Unit A is the right call for every case:

- **B1 — schema.** New columns on `team_runs`: `delegation_count INTEGER NOT NULL DEFAULT 0` (filled at execute-tasks dispatch time), `solo_absorption INTEGER NOT NULL DEFAULT 0` (boolean flag set when the run completed without delegation), `failure_context TEXT` (nullable JSON, holds `{"phase": "first_decompose"|"revision_after_critic"}` for `FailedNoDelegation` runs — architect F2). Schema migration v39→v40. Backfill pre-v40 rows with NULL (treated as "unknown" by queries).
- **B2 — write site.** `execute_tasks()` increments `delegation_count` for each spawned member session. `finalize_and_shutdown()` sets `solo_absorption = 1` when `delegation_count == 0` and the goal was emitted as actionable (vs trivial-conversational).
- **B3 — dashboard surface.** Existing dashboard `/api/v1/team-runs` and per-run detail surfaces should display these fields. Cheap surface: badge/icon on the run row.

The point of Unit B: even if Unit A's gate has false-positives or false-negatives, operators can still find these runs via DB query (`SELECT * FROM team_runs WHERE solo_absorption = 1`). Decouples detection from prevention.

### Unit C (parse tolerance) — reduce how often Unit A's gate has to fire

Apply existing prose-tolerance machinery to `parse_task_assignments()` as a **preprocessor that feeds existing schema validation** (architect F3-pinned — explicit integration point):

- **C1 — apply `extract_first_json_array` preprocessor.** Add a helper that locates the first balanced `[…]` in surrounding prose (analogous to #876's `extract_first_json_object` for braces). The helper runs BEFORE `parse_task_assignments`'s current array-extraction regex (1730) and feeds its output into the existing parse path.
- **C2 — fence stripping + validation sequencing (architect F3 explicit).** Strip markdown fences (```json …```) before parse. Then: **attempt `serde_json::from_str::<Vec<TaskAssignment>>` on the extracted/stripped content. On success → proceed to task spawning (bypassing Unit A gate). On failure → fall through to current `parse_task_assignments` logic (which may return `Conversational`, then Unit A's gate evaluates).** Incidental JSON in prose without `agent`+`task`+`output_file` fields fails validation and routes correctly.
- **C3 — prompt reinforcement.** The team-mode decompose prompt (`teams/prompt.rs:94-145`) already says "decompose into tasks." Add one explicit line: "Emit the JSON array as your sole output, on its own line. No prose before or after." Defense-in-depth, won't bind glm-5.2 alone but reduces fire-rate.

Unit C reduces the rate at which Unit A's gate has to fire, but does NOT replace it — prompt-only contracts don't bind across model classes (counter-evidence: `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`). C is in-scope to reduce noise; A is in-scope to make the noise non-silent.

### PR sequencing (architect F4 ratified)

Ship as **two PRs**:
- **PR 1 — Units B + C.** Pure observability + parse tolerance. Zero terminal-state change. Provides Litha-team partial unblock and observability backstop while the higher-stakes Unit A undergoes review.
- **PR 2 — Unit A.** Depends on PR 1's schema columns (reads `failure_context`) but no shared-code conflicts. Lands after PR 1 deploy + bake-in.

## Implementation outline

0. **Pre-implementation survey:** read `teams/engine.rs:640-730` (the orchestration loop) + `teams/engine.rs:1718-1806` (parse_task_assignments) + `db/teams.rs` (schema) + #286's coverage-retry implementation. Architect chooses A1/A2/A3 shape and confirms B's column shape.

1. **Unit C (cheapest, ship first as defense-in-depth):**
   - Extend `parse_task_assignments` with `extract_first_json_array` fallback and markdown-fence stripping.
   - Add one prompt line to `teams/prompt.rs` decompose section.
   - Tests: 4 fixture parses (clean JSON, JSON-in-fences, JSON-in-prose, narrative-only).

2. **Unit B (schema + write sites):**
   - Schema migration v39→v40 — additive ALTER TABLE on `team_runs` for `delegation_count INTEGER NOT NULL DEFAULT 0` and `solo_absorption INTEGER NOT NULL DEFAULT 0`. Per #1262 precedent (v38→v39, additive nullable + DEFAULT, no table rebuild).
   - `execute_tasks()` (existing function, around `engine.rs:660-690`) — increment `delegation_count` via DB write per spawned member.
   - `finalize_and_shutdown()` (`engine.rs:586-599`) — set `solo_absorption = 1` when delegation_count == 0 AND first-decompose returned Conversational.
   - Dashboard API: add `delegation_count`, `solo_absorption` fields to `TeamRunRow` serialization.
   - Tests: schema migration round-trip, write-site coverage, query exercise.

3. **Unit A (the gate, architect-shaped):**
   - One retry of decompose on Conversational-fallthrough for an actionable goal (per architect's A2 decision on actionability detection).
   - On second Conversational: new `RunStatus::FailedNoDelegation` variant, written as `status='failed_no_delegation'` in `team_runs`. Reason captured in `deliverable` or new `failure_reason` column (architect-bearing).
   - Tests: replay run #1 fixture (orchestrator response_text + zero spawns) → expect retry → expect terminal FailedNoDelegation if retry-Conversational; expect normal completion if retry produces valid Tasks.

4. **Replay harness:**
   - Calibration scenario in `crates/mika-agent/src/calibration/roles/mika_*` — TBD, possibly `mika-orchestrator` once mika#1641 lands, or stand-alone team-engine test fixture.
   - Replay the founding-incident decompose response text against the fixed parser. Assert (a) Unit C catches the JSON if it's there, (b) Unit A fires the retry if it isn't, (c) Unit B's columns are populated regardless.

## Acceptance criteria

- **AC1 — Unit C parse tolerance.** `parse_task_assignments` parses (a) markdown-fenced JSON, (b) JSON with surrounding prose, (c) prose-only narrative all without changing the `DecomposeResult::Conversational` semantic for prose-only — but lifting the rate at which valid JSON-in-prose still routes to Conversational. Fixture tests cover all three.

- **AC2 — Unit B observability.** `team_runs` schema gains `delegation_count INTEGER NOT NULL DEFAULT 0` + `solo_absorption INTEGER NOT NULL DEFAULT 0` columns (v39→v40 migration). `execute_tasks()` increments `delegation_count` per spawned member. `finalize_and_shutdown()` sets `solo_absorption = 1` when first-decompose was Conversational AND delegation_count == 0. Both fields are queryable via SQL and exposed on the dashboard API.

- **AC3 — Unit A delegation gate.** On `Conversational` fallthrough during first-decompose for an actionable goal (actionability = response matches keyword regex `(?i)\b(dispatching?|decompos\w*|delegat\w*|assign(?:ed|ing)?\s+(?:to|for)|hand(?:ing|ed)?\s+(?:off|over)\s+to|split\w*\s+(?:work|task)|parallel\w*\s+(?:delegat|specialis|member))\b` OR contains JSON-array-shape that failed `Vec<TaskAssignment>` validation — architect F1 pinned): the engine runs one retry of decompose with prompt reinforcement. On second `Conversational`, the run transitions to terminal `RunStatus::FailedNoDelegation` (`status='failed_no_delegation'` in DB), not `Completed`. Re-decompose path uses same gate, same terminal state, with `failure_context = {"phase": "revision_after_critic"}` (architect F2 — single terminal state, phase as JSON detail).

- **AC4 — Regression scenario.** Replay run `team-49640b61-…`'s decompose response text against the fixed engine. Assert: (a) the response either parses to Tasks (if Unit C tolerance suffices) or triggers the retry (if it doesn't), (b) zero spawns paired with first-decompose Conversational produces `FailedNoDelegation` not `Completed`, (c) `solo_absorption=1` is written even if the run somehow reaches `Completed` (Unit B remains a backstop).

- **AC5 — Dashboard surface.** The dashboard team-run detail view shows `delegation_count` and a "solo absorption" warning badge when `solo_absorption=1`. Existing `/api/v1/team-runs` responses carry the new fields.

## Out of scope

- **Per-skill prompt-tuning for glm-5.2 specifically.** The model class contributes to the failure rate, but the structural gaps in Layers 1-2-3 exist regardless. Model-swap calibration is a separate axis (mika#1632 / mika#1633).
- **`a2a_call` reach for local members.** Already fixed by mika#1653 (suppressed from team-mode tool array). This ticket does not re-relitigate that.
- **Orphan reaper.** Already fixed by mika#1652. The reaper catches stuck `in_progress` runs; this ticket prevents the `completed` mis-classification upstream.
- **Removing the `Conversational` variant entirely.** Honest orchestrator-solo behavior on trivial-conversational goals (no actionable decompose) must remain a valid terminal path. The fix shape preserves this distinction.

## Files involved

- `crates/mika-agent/src/teams/engine.rs:656-715` — Unit A short-circuit guard
- `crates/mika-agent/src/teams/engine.rs:1718-1806` — Unit C parse tolerance
- `crates/mika-agent/src/teams/prompt.rs:94-145` — Unit C prompt reinforcement
- `crates/mika-agent/src/db/teams.rs` — Unit B schema migration v39→v40 + write methods
- `crates/mika-agent/src/db/migrations.rs` — Unit B migration registration
- `crates/mika-agent/src/server/team_runs.rs` — Unit B dashboard API serialization
- `dashboard/src/components/team-runs/` — Unit B UI surface
- `crates/mika-agent/tests/teams/engine_tests.rs` — Unit A/B/C tests (or co-located in engine.rs)

## Verification

- `cargo test -p mika-agent --test eval` — full eval matrix stays green.
- `cargo test -p mika-agent` — team engine tests cover new parse paths + delegation gate.
- Schema migration round-trip test: open v39 DB → apply v40 migration → verify new columns exist with correct defaults → write/read round-trip on both columns.
- Replay test: feed run #1's orchestrator decompose response_text to `parse_task_assignments` → assert outcome (Tasks if Unit C suffices, retry-then-FailedNoDelegation if it doesn't).
- Synthetic test: zero-member-spawn run → assert `solo_absorption=1` written by `finalize_and_shutdown`.

## References

- mika#1676 founding evidence — run `team-49640b61-ec98-4be1-89c8-c11aac59a5b7` (orchestrator-solo, 0 delegations, status=completed)
- mika#1652 (CLOSED) — orphan reaper, sibling structural gap
- mika#1653 (CLOSED) — a2a_call→503, sibling
- mika#286 — coverage_retry pattern for omitted members in valid JSON (composes with Unit A's retry-on-Conversational)
- mika#569 — prose-style tool-call detection (parallel pattern at agent-loop layer)
- mika#876 — `extract_first_json_object` for KG extraction parse tolerance (Unit C's sibling for arrays)
- mika#1632 / mika#1633 — model-swap calibration (orthogonal axis, NOT scope expansion)
- Observation doc: `mika-platform/docs/solutions/agent-quality/2026-06-29-team-run-completes-without-delegation.md`
- `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate.md` — why Unit C alone is insufficient; Unit A is load-bearing
