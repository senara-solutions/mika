# dev-groom autonomous pilot switches to content-only slash command (mika#1271)

**Ticket:** mika#1271 — Contract refactor: pilot owns content; dispatch-lib owns git workflow + iterate loop.
**Architect verdict:** `flip` on session `0583a902-cd7a-45ab-89be-59e13c8b09ec`.
**Sub-PR sequence:** **Sub-PR 8 of mika#1271 — terminal sub-PR of the contract refactor.**
  - PRs #1273 → #1274 → #1275 → #1276 → #1277 → #1278 → #1279 → #1280.
  - **This PR (8)**: autonomous-loop pilot switches from `/mika-groom-ticket` to `/mika-groom-plan-only` (a content-only sibling). Closes the cost regression introduced when 7a flipped the iterate loop default-on (pilot's architect + dispatch-lib's architect doubling) and the body-callout-overlap (pilot's organic write + canonical writer overlap).
  - **Companion mika-platform commit**: creates `mika-platform/.claude/commands/mika-groom-plan-only.md`.

## Goal

Close mika#1271 by completing the contract refactor's content-side. The structural plumbing (sub-PRs 1–7b) is in place: dispatch-lib owns the architect convergence + canonical body callout. This PR makes the autonomous pilot's content-only contract real by routing it through a content-only slash command instead of the full operator-facing pipeline.

## What changes

### New (companion repo: mika-platform)

`mika-platform/.claude/commands/mika-groom-plan-only.md` — content-only autonomous-loop slash command. Phase 1 (read ticket, derive branch) + Phase 2 (set up worktree state, run `/ce:plan`, commit plan, push branch) + Phase 3 (exit cleanly). No architect calls. No body-callout writes. No issue comments. Companion to `/mika-revise-plan` (sub-PR 4 pattern).

### Single-line code change (this repo: mika)

`skills/bundled/_shared/dispatch-lib.sh::dispatch_claude_pilot` — change `ENTRY_COMMAND` for the `dev-groom` skill from `/mika-groom-ticket` to `/mika-groom-plan-only`. Surrounding comments updated to explain the split (autonomous-loop uses content-only; operator-direct keeps the full pipeline).

### What stays unchanged

- `/mika-groom-ticket` slash command — the operator-facing pipeline is preserved verbatim. Vincent's direct `/mika-groom-ticket mika issue#N` runs all phases including architect calls and body-callout writes.
- The dev-groom skill prompt (`mika/skills/bundled/dev-groom/system_prompt.md`) — the mika-dev-facing dispatcher prompt doesn't reference any specific slash command; it tells mika-dev to call `run_claude_pilot_groom`, and the handler routes based on `ENTRY_COMMAND` (which now picks `/mika-groom-plan-only`).
- The iterate loop (`_iterate_groom_loop`), the canonical writer (`_write_canonical_callout`), and the escalate helper (`_escalate_groom`) — all sub-PR 3–7b structural plumbing.

## Why a parallel slash command (not refactoring `/mika-groom-ticket`)

Three options were considered:

1. **Mode-detection branching inside `/mika-groom-ticket`** — add a top-level guard that skips Phases 2.5/3/4 and steps 19–20 when running inside claude-pilot. Single slash command, two code paths. Rejected: mode detection in slash commands isn't first-class (no clean `MIKA_AUTONOMOUS_MODE` env var the slash command can read; the pilot's claude-pilot session doesn't expose a reliable autonomous-marker).
2. **Argument-flag inside `/mika-groom-ticket`** — accept `--autonomous` and branch on it. dispatch-lib invokes with the flag. Rejected: makes `/mika-groom-ticket` a multi-modal artifact whose interactive shape needs to ignore a flag it doesn't use. Worse readability for operator-direct use.
3. **Parallel slash command** ✅ — `/mika-groom-plan-only` mirrors `/mika-revise-plan` (sub-PR 4 pattern). Each slash command does one thing. dispatch-lib chooses which one via `ENTRY_COMMAND`. Operator-direct invocation routes to `/mika-groom-ticket` (unchanged). Autonomous-loop dispatch routes to `/mika-groom-plan-only`. Cleanest separation.

## Acceptance criteria

- [ ] **AC1:** `mika-platform/.claude/commands/mika-groom-plan-only.md` exists with the content-only contract documented (Phase 1 → 2 → 3; no architect, no body callout, no comment).
- [ ] **AC2:** `mika/skills/bundled/_shared/dispatch-lib.sh` `dispatch_claude_pilot` function sets `ENTRY_COMMAND="/mika-groom-plan-only"` in the `dev-groom)` case branch.
- [ ] **AC3:** `/mika-groom-ticket` is unchanged (the operator-facing pipeline preserved).
- [ ] **AC4:** `CLAUDE_PILOT_MIN_TOOL_CALLS` default for dev-groom remains 3 (content-only path still produces well above that threshold: issue view + /ce:plan + file edits + git add/commit/push).
- [ ] **AC5:** Comments in `dispatch-lib.sh` document the autonomous/operator split.
- [ ] **AC6:** `bash -n` exit 0 on both `dispatch-lib.sh` and `test-dispatch-lib.sh`.
- [ ] **AC7:** Test suite pass count unchanged from sub-PR 7b's 127 (the ENTRY_COMMAND string is not directly asserted by any test; the change is a configuration swap).

## Cost-regression resolution evidence

After this PR lands and is deployed:

| Per-groom cost element | Before (sub-PR 7b state) | After (sub-PR 8 state) |
|---|---|---|
| Pilot's architect call (Phase 3 + Phase 4 of `/mika-groom-ticket`) | 2 calls per groom | **0 calls** |
| dispatch-lib's architect call (`_iterate_groom_loop`) | 2 calls per groom | 2 calls per groom |
| **Total architect calls per groom** | **4** | **2** ← steady state |
| Pilot's body-callout write (`gh issue edit` in step 19 of `/mika-groom-ticket`) | 1 write per groom | **0 writes** |
| dispatch-lib's canonical write (`_write_canonical_callout`) | 1 write per groom | 1 write per groom |
| **Total body-callout writes per groom** | **2 (overlapping)** | **1 (sole writer)** |

The contract refactor's stated goal — *"pilot owns content; dispatch-lib owns git workflow + iterate loop"* — is now structurally true.

## What remains after sub-PR 8

- mika#1272 — paraphrased dispositions (separate ticket; iterate loop's `_parse_disposition` already tolerates canonical forms; paraphrased-tolerant variant ships independently).
- Live-exercise on the new content-only slash command — first operator-driven groom dispatch under sub-PR 8 confirms the autonomous flow stays clean.

After sub-PR 8 lands and the autonomous flow is exercised once, mika#1271 closes.

## Risks

- **Mode-detection drift:** if a future maintainer reverts `ENTRY_COMMAND` to `/mika-groom-ticket` thinking the slash commands should be unified, the cost regression returns silently. Mitigation: the comment block in `dispatch-lib.sh` explicitly cites mika#1271 sub-PR 8 + the rationale.
- **Operator-loop invocation regression:** the only thing protecting operator-direct grooming is that operators run `/mika-groom-ticket` directly (in their Claude Code session), bypassing dispatch-lib. If an operator workflow accidentally routes through dispatch-lib's autonomous path, they'd get content-only behavior. Acceptable risk — operator-direct grooming via `/mika-groom-ticket` is the documented path in `mika-platform/CLAUDE.md` and `/mika-groom-ticket`'s own description.
- **Idempotency on already-groomed re-dispatch:** when the operator dispatches the same ticket twice, the autonomous loop hits an already-groomed body. `/mika-groom-plan-only` reuses an existing plan if found (step 4); the iterate loop's canonical writer's idempotency check skips the re-write. Same idempotency story as sub-PR 6.

## Test plan

Structural tests in `test-dispatch-lib.sh` pass (127 / 6 pre-existing failures unchanged). The `ENTRY_COMMAND` swap is a configuration change; no assertion directly targets the string.

Live exercise will happen post-merge via either operator-driven groom dispatch or an autonomous `ready`-labelled ticket.

## Provenance

- mika#1271 parent ticket, milestone#26 — closes after this lands + live-exercises.
- Sequence: PRs #1273 → #1274 → #1275 → #1276 → #1277 → #1278 → #1279 → #1280 → **this** (terminal sub-PR).
- Architect contract: session `0583a902-cd7a-45ab-89be-59e13c8b09ec` (`flip` / `(i) Retire` / `yes`).
- Companion mika-platform commit: creates `/mika-groom-plan-only` slash command.
- Pattern precedent: sub-PR 4 (`/mika-revise-plan` — content-only slash command for ITERATE revise) — same shape.
