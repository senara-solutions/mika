---
issue: 1646
type: fix
date: 2026-06-30
---

# Plan — fix(mika-dev): destructive actions re-execute on cached rationale (mika#1646)

## Problem

mika-dev's LLM closed PR #1644 twice within 9 minutes (11:08:54Z + 11:15:49Z, 2026-06-29) on the SAME fabricated "duplicate of mika#1638" rationale — even though a human operator comment with diff-grounded contradiction was sitting in the thread between the two closes. The re-act without re-grounding is the load-bearing defect.

Per Mika Prime: *"If a different upstream error arrives next week with a human correction in the thread, mika-dev will close again."*

## Architectural lineage

- mika#1331 — assert-grounded engine guard (the class this extends to destructive actions)
- mika#1133 — dev-groom fabrication guard (parallel structural shape on Verdict claims)
- mika#1645 — sibling: qa's emission-side equivalence-claim grounding (this is the action-side guard)
- PR #1644 — founding incident
- mika-dev autopsy session `4669385f-ea0b-41af-b224-0cb1b0022fb1`

## Fix shape (engine guards, parallel to mika#1331) — architect-ratified (session `676e9497-b971-4fbd-9d6a-ab85694bcd98`, GROOMED)

### Layer A — Pre-destructive-action grounding (mika#1331-class engine guard)

Before mika-dev's LLM emits `gh pr close`, `gh issue close`, or any destructive terminal action, the EndTurn engine guard requires:

1. A `gh pr view <N> --json files` (or `gh pr diff`, `gh issue view --json files`) tool call in the current session's tool_calls.
2. The close comment body (the `--body` text passed to `gh pr close --comment` or equivalent) cites the file-list comparison as the justification.

Without (1) AND (2), the guard blocks with `block[destructive_grounding]`. Same shape as assert-grounded.

### Layer B — Thread-state re-ground on repeated destructive action

When the LLM is about to take a destructive action on a PR/issue target, the guard checks: was the same action taken on the same target within the last **`MIKA_DEV_REPEAT_ACTION_WINDOW_SECS` (default 1800s = 30min)** (architect F1)? If yes:

1. Require a `gh issue view <N> --json comments` (or `gh pr view <N> --json comments`) tool call this turn.
2. The new action's body MUST explicitly acknowledge the prior action AND reference any comments posted SINCE the prior action timestamp.

If contradicting comments are present and not explicitly acknowledged, the guard blocks with `block[destructive_repeat_unreviewed]` and emits an audit event for operator surface.

### Layer C — Audit trail

Every destructive action (allowed or blocked) writes a `destructive_action_groundings` row to `audit_events` with the grounding tool-call IDs. Post-hoc queryable.

### Future expansion (architect F2 — documented for discoverability, NOT in this PR's scope)

The destructive-action set in Layer A is intentionally bounded to `gh pr close` + `gh issue close`. Future expansion candidates if these surfaces exhibit the same cached-rationale defect:
- `gh pr merge --admin` (and any admin-merge path)
- `gh issue delete` / `gh pr delete`
- `gh pr label remove` (especially `ready` label removal that interrupts dispatch flow)

Out-of-scope unless n≥2 occurrences emerge per evidence-gated-expansion discipline.

## Implementation outline

0. **Prerequisite (architect F3 BLOCKING):** verify `audit_events.event_type` schema before adding the new event type. If ENUM-constrained, the implementation requires a schema migration. If free-form VARCHAR/TEXT, no migration needed. Probe: `sqlite3 ~/.mika/data/mika.db "SELECT sql FROM sqlite_master WHERE name='audit_events';"` — branch the plan based on findings.

1. **New engine guard module:** `crates/mika-agent/src/agent_loop/guards/destructive_grounding.rs`. Two checks (Layer A + B) inside one module. Register in the EndTurn guard chain alongside assert-grounded.

2. **Equivalence keyword + destructive-action detection:**
   - Detect destructive action: scan pending tool_calls for `gh pr close`, `gh issue close` (with optional flags). Regex against the command string. Implementer first task: grep `crates/mika-agent/src/tools/` for the gh-shell tool surface.
   - Detect repeated-same-action: SQL on `tool_calls` table — `WHERE tool_name = 'run_gh' AND session_id = ? AND created_at > datetime('now', '-' || $WINDOW_SECS || ' seconds') AND input LIKE '%pr close %N%'` (parameterized).

3. **Audit row:** add `destructive_action_groundings` event type to audit_events per Step 0's findings. Either reuse existing schema (VARCHAR event_type) or add migration (ENUM event_type).

4. **Calibration scenario:** `crates/mika-agent/tests/eval/calibration_fixtures/mika-dev/destructive_action_thread_reground.md` — replay PR #1644 timeline (qa hold[review] → mika-dev close → operator re-open with diff comment → second qa hold[review]). Assert mika-dev does NOT re-close. Required tool calls + acknowledgment in the new action body.

5. **Register scenario:** in `tests/eval/calibration_fixtures/mika-dev/manifest.yaml` AND in `crates/mika-agent/src/calibration/roles/mika_dev.rs`. Both registrations explicit (no auto-discovery).

## Acceptance criteria

- **AC1** — Pre-destructive grounding: before any `gh pr close` / `gh issue close` call from mika-dev's LLM, the engine guard requires a `gh pr view --json files` (or `gh pr diff`) tool call AND the close comment must cite the file-list comparison. Without both, the guard blocks with `block[destructive_grounding]`.

- **AC2** — Thread-state re-ground: when a destructive action repeats on a target within `MIKA_DEV_REPEAT_ACTION_WINDOW_SECS` (default 1800s) of a prior identical action, mika-dev's protocol requires reading comments since the prior action's timestamp. If contradicting comments are present without explicit acknowledgment in the new action body, the action is blocked with `block[destructive_repeat_unreviewed]` pending operator surface.

- **AC3** — Audit event: every destructive action carries a `destructive_action_groundings` audit_events row with the grounding tool-call IDs that authorized it. Auditable post-hoc.

- **AC4** — Regression scenario: replay today's PR #1644 timeline (qa hold[review] → mika-dev close → operator re-open with diff comment → second qa hold[review] event) against the fixed protocol. mika-dev must NOT re-close. Operator surface required.

- **AC5** — Calibration scenario in mika-dev's role suite. Add fixture at `tests/eval/calibration_fixtures/mika-dev/destructive_action_thread_reground.md`. Register in `manifest.yaml` and `roles/mika_dev.rs`.

## Out of scope

- mika-qa's emission-side defect — tracked at mika#1645 (complementary fix).
- Verdict-handler engine code (confirmed correct on hold[review] mapping).
- Destructive-action expansion beyond pr/issue close — see Future expansion section.

## Files involved

- `crates/mika-agent/src/agent_loop/guards/destructive_grounding.rs` — NEW guard module
- `crates/mika-agent/src/agent_loop/guards/mod.rs` — register in EndTurn chain
- `crates/mika-agent/src/server/audit_events.rs` — verify/extend audit schema per Step 0
- `crates/mika-agent/tests/eval/calibration_fixtures/mika-dev/destructive_action_thread_reground.md` — NEW fixture
- `crates/mika-agent/tests/eval/calibration_fixtures/mika-dev/manifest.yaml` — register scenario
- `crates/mika-agent/src/calibration/roles/mika_dev.rs` — wire scenario

## Verification

- Existing mika-dev calibration scenarios stay green.
- New scenario (AC5) passes.
- Synthetic guard-fire tests:
  - `gh pr close 1` without grounding tool call → block[destructive_grounding]
  - `gh pr close 1` twice within 30min (default window) → second blocks with block[destructive_repeat_unreviewed]
- Tunability check: setting `MIKA_DEV_REPEAT_ACTION_WINDOW_SECS=10` and repeating after 15s → first repeat allowed (outside window).

## References

- mika#1331 — engine guard parent class
- mika#1133 — dev-groom fabrication guard sibling
- mika#1645 — qa-side equivalence grounding (complementary)
- mika-dev autopsy session `4669385f-ea0b-41af-b224-0cb1b0022fb1`
- Mika Prime bearing 2026-06-29 ~11:25Z
- mika-arch grooming session `676e9497-b971-4fbd-9d6a-ab85694bcd98` (ITERATE → 3 architect concerns applied → GROOMED)
- PR #1644 — founding incident
