# Iterate-loop ITERATE branch: revise-payload pilot relaunch (mika#1271)

**Ticket:** mika#1271 — Contract refactor: pilot owns content; dispatch-lib owns git workflow + iterate loop.
**Architect verdict:** `flip` on session `0583a902-cd7a-45ab-89be-59e13c8b09ec`; v1 scope `yes`.
**Sub-PR sequence:** **Fourth sub-PR of mika#1271.**
  - PR#1273 (`f2bef21`): `_post_flight_push` → `_push_branch` rename.
  - PR#1274 (`1eb5a03`): Phase A/B/C primitives.
  - PR#1275 (`30917d7`): Phase D state machine `_iterate_groom_loop` + Phase F feature-flag wiring (READY-path only).
  - **This PR**: ITERATE branch real flow — `_launch_revise_pilot` + `/mika-revise-plan` slash command + cleanup-on-GROOMED + tests.

## Goal

Replace the ITERATE-branch WARN-and-fall-through in `_iterate_groom_loop` with the real flow: write architect findings to a tempfile, launch a content-only revise pilot via `/mika-revise-plan`, invoke `mika-arch-second-review` on the revised plan (continuing the architect session), parse the verdict, return 0 on GROOMED. Sweep `.iterate/` findings on GROOMED only; preserve on ESCALATE for forensic access.

## Contract restatement (verified from architect prompts before implementing)

The two-pass max is **structural**, not aspirational:
- `mika-arch-groom-ticket` first-pass: `READY | ITERATE | ESCALATE`. ITERATE means *"revise and re-submit for **second review**"* (verbatim from `skills/bundled/mika-arch-groom-ticket/system_prompt.md` line 55).
- `mika-arch-second-review` second-pass: `GROOMED | ESCALATE`. Line 9: *"No third pass. You may **never** return ITERATE."*

Implication: **one revision cycle max**. No N-round iteration loop. No `MAX_ITERATE_ROUNDS` constant. The state machine has exactly two terminal-bearing branches.

## Implementation

### Changes in this PR

**`skills/bundled/_shared/dispatch-lib.sh`:**
- **New `_launch_revise_pilot(findings_file)`** — invokes `claude-pilot --command /mika-revise-plan @<findings>` with a distinct sub-session log id (`${LOG_ID}-revise-$(date +%s)`). Detects revision via `sha256sum` of the plan file before-and-after. Returns 0 if content changed, 1 otherwise (missing args, no plan, pilot failed to revise). `mtime` deliberately not used (too coarse + sensitive to filesystem semantics).
- **New `_cleanup_iterate_findings()`** — sweeps `$WORKTREE_DIR/.iterate/` on terminal GROOMED success. **Preserves on ESCALATE** for forensic access; worktree TTL handles eventual cleanup. Sweep call placed only in the two GROOMED return paths (READY → GROOMED and ITERATE → GROOMED), never in ESCALATE/failure paths.
- **`_iterate_groom_loop` ITERATE branch rewrite** — from "WARN + return 1 (out-of-v1)" to the real flow:
  1. `mkdir -p $WORKTREE_DIR/.iterate` (chosen over `.claude/` to avoid namespace collision with `_set_up_worktree`'s slash-command snapshot at `$WORKTREE_DIR/.claude/commands/`).
  2. Write architect first-pass `.content` to `$WORKTREE_DIR/.iterate/findings-1.md`.
  3. `_launch_revise_pilot` against the findings file.
  4. On revise success: `_arch_ask mika-arch-second-review` with the same `session_id` from first-pass (architect's session-continuity contract preserves findings in conversation memory across the call).
  5. Parse verdict via `_parse_verdict` (literal-form only; paraphrased is mika#1272).
  6. On GROOMED: `_cleanup_iterate_findings`, return 0. On anything else: WARN with preservation note, return 1.

**`mika-platform/.claude/commands/mika-revise-plan.md`** (landed in a separate direct-to-main commit on mika-platform — `e9c7060`):
- Content-only revise contract. Pilot reads `@<findings>` file, finds plan via `*-${ISSUE_NUM}-*-plan.md` pattern, revises plan in-place, exits.
- No git operations, no architect invocation, no `/ce:plan`. dispatch-lib detects revision via sha256.
- Snapshot copied into worktree by `_set_up_worktree` (line 451-453 of `dispatch-lib.sh` — `cp -r $PLATFORM_DIR/.claude/commands $WORKTREE_DIR/.claude/`). Snapshot semantics per mika#1173.

**`skills/bundled/_shared/test-dispatch-lib.sh`:**
- 15 new test assertions covering `_launch_revise_pilot` guards (no findings / no `WORKTREE_DIR` / no `ISSUE_NUM` / no plan file), code-shape verification (`/mika-revise-plan` invoked, sha256 detection, `@<findings_file>` payload), `_cleanup_iterate_findings` (no-op when absent / sweeps when present / no-op when `WORKTREE_DIR` unset), and `_iterate_groom_loop` ITERATE-branch structure (calls `_launch_revise_pilot`, writes to `.iterate/`, invokes second-pass post-revise). Cleanup symmetry asserted via grep count: exactly 2 calls to `_cleanup_iterate_findings` (both GROOMED paths), 2 invocations of `mika-arch-second-review` (READY-branch + ITERATE-branch).

### Verified invariant: no plan-on-origin coupling between passes

Grepped `skills/bundled/` and `crates/mika-agent/src/skills/` for any code that reads plan state from origin between architect passes (`git fetch.*plan`, `origin/.*plan`, `gh.*view.*plan`, `fetch.*architect`). Only match is a comment at `dispatch-lib.sh:1050` discussing `gh issue view` in a different context. **Confirmed:** plan content travels via `@-file` body in `_arch_ask`; no push-between-passes needed. The revised plan stays on the worktree's disk; second-pass reads it from the file passed in the prompt.

### Behavior unchanged when feature flag is off

`MIKA_DISPATCH_USE_ITERATE_LOOP` defaults to unset. With the default, `_iterate_groom_loop` is never called and the existing pilot-owns-architect path runs as before. This sub-PR doesn't change the default; production behavior is preserved until operator-driven exercise confirms the new path.

## Acceptance criteria

- [ ] **AC1:** `_launch_revise_pilot(findings_file)` is defined in `skills/bundled/_shared/dispatch-lib.sh`. Invokes `claude-pilot --command /mika-revise-plan @<findings_file>` and returns 0 iff the plan file's sha256 changed.
- [ ] **AC2:** `_cleanup_iterate_findings()` sweeps `$WORKTREE_DIR/.iterate/` and is a no-op when the directory doesn't exist or `WORKTREE_DIR` is unset.
- [ ] **AC3:** `_iterate_groom_loop`'s ITERATE branch writes findings to `$WORKTREE_DIR/.iterate/findings-1.md`, launches the revise pilot, and on success invokes `mika-arch-second-review` with the first-pass `session_id`.
- [ ] **AC4:** Cleanup symmetry — `_cleanup_iterate_findings` is called on the two GROOMED return paths (READY-success and ITERATE-success), and **NOT** on any ESCALATE or failure path. Verified by grep count: exactly 2.
- [ ] **AC5:** Session-id symmetry — `mika-arch-second-review` is invoked on both READY and ITERATE branches, each time threaded with the `session_id` captured from first-pass response. Verified by grep count: exactly 2 invocations of `mika-arch-second-review`.
- [ ] **AC6:** `/mika-revise-plan` slash command exists at `mika-platform/.claude/commands/mika-revise-plan.md` (landed in commit `e9c7060` on mika-platform main); copied into per-task worktrees by `_set_up_worktree`.
- [ ] **AC7:** `bash -n` syntax check passes on both `dispatch-lib.sh` and `test-dispatch-lib.sh`.
- [ ] **AC8:** 15 new test assertions pass. Pre-existing failure count (6 on main) unchanged.

## Risks

- **Revise pilot may also fail to commit** (per today's wrote-but-no-commit class). Under the new contract this is correct: the pilot revises content; dispatch-lib owns commit/push. Detection via sha256 of plan content is independent of whether the pilot committed; the existing `_push_branch` finalizer (PR#1268) handles the eventual push if any commits exist downstream.
- **Findings file as `@-file` payload — size limits.** Architect responses can be long (100s of lines of prose + F-list). `mika ask @<file>` reads the entire file as message body. No empirical evidence of size limits hit in today's exercise; risk is theoretical. Mitigation if it appears: truncate findings to F-list lines + verdict before passing.
- **`.iterate/` directory ownership.** Confirmed via grep that no other writer touches `.iterate/`. `_set_up_worktree` only creates `.claude/`, not `.iterate/`. Race-free by construction.
- **Revise pilot exit code unreliable.** Same class as the wedge mika#1268 fixed; the pilot may exit non-zero even after successful revise. Sha256 detection is independent of exit code — we don't rely on exit code for the success signal. (Belt-and-braces; the WARN log includes exit code for diagnostic purposes.)

## What does NOT ship in this sub-PR

- **ESCALATE flow** with structured `PIPELINE FAILURE` marker — still WARN + return 1, falls through. Next sub-PR.
- **Canonical body-callout writer** (`_write_canonical_callout`) — Class D shim still writes downstream. Later sub-PR; depends on ITERATE + ESCALATE landing first.
- **mika#1272** (paraphrased dispositions) — separate ticket; queued after the state machine is exercising end-to-end.
- **Class D body-callout shim retire** — final sub-PR of mika#1271.
- **Feature flag removal** — terminal sub-PR.

## Test plan

All AC items verified by tests in `test-dispatch-lib.sh`. No integration test invokes real `mika ask` or `claude-pilot`. Operator-driven exercise on a real ticket with `MIKA_DISPATCH_USE_ITERATE_LOOP=1` is the live validation.

## Provenance

- mika#1271 parent ticket, milestone#26.
- Architect contract verdict: session `0583a902-cd7a-45ab-89be-59e13c8b09ec` (`flip` / `(i) Retire` / `yes` across three rounds).
- Friend-peer review on the ITERATE-flow design (today): ratified option (a) for the new slash command; sharpened cleanup asymmetry (sweep-on-GROOMED-preserve-on-ESCALATE); enforced session-id symmetry across READY and ITERATE branches; pushed the "read architect prompts before guessing" rule that surfaced the structural-two-pass-max contract.
- Architect-prompt verification: `mika-arch-groom-ticket/system_prompt.md:55` (ITERATE → second review) + `mika-arch-second-review/system_prompt.md:9` (no third pass).
- Companion: `mika-platform e9c7060` (slash command).
