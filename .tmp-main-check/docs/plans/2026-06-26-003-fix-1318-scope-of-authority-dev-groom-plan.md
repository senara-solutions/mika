---
title: "fix: Scope-of-authority guard for dev-groom pilot force-push (mika#1318)"
type: fix
origin: https://github.com/senara-solutions/mika/issues/1318
ticket: mika issue#1318
date: 2026-06-26
depth: Standard
---

# fix: Scope-of-authority guard for dev-groom pilot force-push

## Summary

On 2026-05-27, a dev-groom pilot session ran `git push --force-with-lease` from inside its content-discovery decision branch, destroying substrate-fix work on the remote. The same pattern reproduced on mika#736. The pilot's git fluency was sufficient to invoke the destructive command but insufficient to read which side was ahead. Recovery was coincidental (mika#1289 auto-fire hook + mika#1311 worktree-setup), not by design.

Three defense-in-depth layers are needed: prompt-level prohibition in the dev-groom skill, structural post-flight detection in dispatch-lib, and forensic audit trail for force-push events.

---

## Problem Frame

The dev-groom pilot operates inside a worktree with full bash access. The pilot's authorized scope is content-only: read the ticket, generate a plan, commit it. Git push is explicitly dispatch-lib's responsibility (`_push_branch`). However, the dev-groom skill prompt (`system_prompt.md`) contained no explicit force-push prohibition, and the pilot reasoned its way to `git push --force-with-lease` when it observed local/remote divergence.

**Existing defenses (already shipped):**
- `claude-pilot-py/src/claude_pilot/tier1.py` — TIER3 patterns hard-deny `git push --force` and `git push -f` (grep `TIER3_DENIED`); `_FORCE_FLAG_RE` blocks force flags in `is_safe_git_command()` (grep `_FORCE_FLAG_RE`). Line numbers subject to drift.
- `/mika-groom-plan-only` command prompt (mika-platform PR #134) — explicit force-push prohibition text
- `dispatch-lib.sh::_push_branch` — sole git-push site for dev-groom dispatches

**Gap analysis:** The tier1/TIER3 defenses were added after the incident and should prevent recurrence at the claude-pilot layer. However:
1. The dev-groom skill prompt (what mika-dev sees when dispatching) has no prohibition — mika-dev could theoretically dispatch a session with instructions that encourage push behavior.
2. No post-flight detection exists — if tier1 is bypassed (regression, new flag pattern, compound command escape), the destructive push succeeds silently.
3. No audit trail — force-push events are only visible in SDK transcripts, not in structured logs queryable by operators.

---

## Requirements

- **R1.** The dev-groom skill prompt must explicitly prohibit force-push commands from the pilot session scope.
- **R2.** dispatch-lib must detect post-flight whether a force-push occurred during the pilot session (defense-in-depth against tier1 bypass).
- **R3.** When a force-push is detected, dispatch-lib must mark the dispatch as a structural violation in the RESULT envelope and surface it in the callback.
- **R4.** The detection must distinguish pilot force-pushes (violation) from dispatch-lib's own `_push_branch` force-with-lease (legitimate).
- **R5.** dev-pilot skill must not be affected — dev-pilot's `_push_branch` usage is legitimate.
- **R6.** A post-deploy signal must exist so operators can verify force-push detection is active (Signal pattern per CLAUDE.md conventions).

---

## Key Technical Decisions

### KTD-1: Reflog-based detection over hook-based interception

**Decision:** Use `git reflog` scanning in the post-flight check rather than a server-side `pre-receive` hook or a git client-side `pre-push` hook.

**Rationale:** The pilot runs inside a temporary worktree that is cleaned up after dispatch. A client-side `pre-push` hook in the worktree would need to be seeded by `_set_up_worktree` — another fragile injection point. Server-side hooks require GitHub repository admin access and affect all pushers, not just pilot sessions. Reflog scanning at post-flight is dispatch-lib's native scope: it already reads `PRE_RUN_HEAD`, `POST_RUN_HEAD`, and the worktree's git state. The reflog shows push events with `(forced update)` markers that the scan can key on.

**Trade-off:** Reflog-based detection is post-hoc — the force-push has already landed on the remote by the time we detect it. This is acceptable because tier1/TIER3 is the primary prevention layer; the reflog guard is forensic defense-in-depth. If a force-push is detected, the RESULT envelope carries the violation and the operator is alerted — recovery is still manual (the same as today), but now it's detected rather than silent.

### KTD-2: Guard scoped to dev-groom skill only

**Decision:** The post-flight reflog guard runs only when `$SKILL = "dev-groom"`. Dev-pilot dispatches are not guarded because `_push_branch`'s force-with-lease after a rebase is legitimate for implementation branches.

**Rationale:** Dev-groom pilots should never push at all — push is dispatch-lib's job. Dev-pilot pilots may push as part of the `/mika` pipeline (PR creation). The asymmetry in authority is the design: dev-groom is content-only, dev-pilot is content+workflow. Extending the guard to dev-pilot would require distinguishing "pilot pushed via gh pr create" (legitimate) from "pilot force-pushed arbitrarily" (violation) — a harder discrimination that isn't needed given dev-pilot's broader authority.

### KTD-3: Detection via remote-ref comparison (not reflog)

**Decision:** Compare the remote branch state before and after the pilot session using `git ls-remote`. Before launching `_run_claude_pilot`, capture `PRE_RUN_REMOTE_HEAD=$(git ls-remote origin "refs/heads/$BRANCH" | cut -f1)` (empty string if branch doesn't exist on remote yet). After the pilot exits (but before `_push_branch`), re-query: `POST_RUN_REMOTE_HEAD=$(git ls-remote origin "refs/heads/$BRANCH" | cut -f1)`. If the two differ, the pilot pushed to the remote — a scope-of-authority violation for dev-groom (regardless of whether it was a force-push or a normal push, since the pilot should never push at all).

**Rationale:** The original plan proposed `git reflog show refs/remotes/origin/$BRANCH` scanning for `(forced update)` markers. However, the `(forced update)` marker is reliably observed in `git push` stderr output (as shown in the mika#1318 incident transcript), but its presence in `git reflog show` output for remote-tracking refs is not empirically verified in our environment. The reflog on remote-tracking refs is a local cache that may behave differently across git versions and worktree configurations. The `git ls-remote` approach queries the actual remote state — it is authoritative, version-independent, and trivially verifiable. It also catches *any* pilot push (not just force-pushes), which is the correct detection surface for dev-groom: the pilot should never push at all.

**Trade-off:** `git ls-remote` requires network access to the remote. This is acceptable because `_push_branch` (which runs immediately after this check) also requires network access — if the remote is unreachable, the entire post-flight phase fails anyway. The additional round-trip adds ~1s latency.

**Limitation:** If the remote branch was updated by an unrelated actor (another developer, CI) between `PRE_RUN_REMOTE_HEAD` capture and the post-flight check, a false positive occurs. This is extremely unlikely for dev-groom branches (which are worktree-isolated, short-lived, and named after the issue number) and would only result in a conservative PIPELINE_INCOMPLETE — not data loss. The operator can inspect the RESULT envelope's evidence field and dismiss the false positive.

### Recovery posture (what happens after the guard fires)

When the force-push guard detects a pilot push and fires:

**Remote state:** The pilot's destructive push remains on the remote branch. dispatch-lib does NOT push over it (the guard fires *before* `_push_branch`), and does NOT attempt to restore `PRE_RUN_REMOTE_HEAD`. Automatic remote recovery is deliberately out of scope — a `git push --force` to restore prior state carries its own blast radius and should be an operator decision.

**Local state:** The worktree retains the pilot's commits. `POST_RUN_HEAD` reflects the pilot's final local state. The worktree is not cleaned up early — it remains available for operator forensics until the normal cleanup path runs.

**Callback state:** The RESULT envelope delivered to mika-dev contains `STRUCTURAL VIOLATION: pilot push detected (mika#1318)` with evidence (both `PRE_RUN_REMOTE_HEAD` and `POST_RUN_REMOTE_HEAD` SHAs). Outcome is `PIPELINE_INCOMPLETE — push violation`. mika-dev surfaces this to the operator.

**Operator recovery steps:**
1. Inspect the RESULT envelope to confirm the violation (false positive check).
2. If genuine: `git push --force origin <PRE_RUN_REMOTE_HEAD>:refs/heads/<branch>` to restore the remote branch to its pre-pilot state.
3. If the pre-pilot state was the correct plan commit from a prior groom pass: re-dispatch the groom or manually fix and push.
4. If the branch had unrelated work (the mika#1318 incident pattern): recover from the other contributor's local state or reflog.

The guard is forensic, not restorative. It converts a silent failure into a loud, actionable one. (Citation: review-guide.md § Single Responsibility — the guard's single responsibility is detection + reporting, not recovery.)

---

## Scope Boundaries

### In scope
- Dev-groom skill prompt update with explicit force-push prohibition
- Post-flight reflog guard in dispatch-lib for dev-groom dispatches
- Structured RESULT envelope annotation for force-push violations
- Signal documentation for post-deploy verification

### Deferred to Follow-Up Work
- Active force-push prevention via git hook injection in worktrees (lower priority given tier1/TIER3 prevention)
- Audit-event emission to the engine's `audit_events` table (requires Rust-side changes; the RESULT envelope annotation is sufficient for v1)
- Extending the guard to dev-pilot dispatches (not needed given dev-pilot's broader authority scope)

---

## Implementation Units

### U1. Dev-groom skill prompt — explicit force-push prohibition

**Goal:** Add a clear, structural prohibition of `git push --force*` to the dev-groom skill prompt so that mika-dev's dispatched sessions carry the prohibition in their system prompt.

**Requirements:** R1

**Dependencies:** None

**Files:**
- `skills/bundled/dev-groom/system_prompt.md`

**Approach:** Add an `### Authority bounds` subsection to the existing `system_prompt.md` that explicitly prohibits `git push --force`, `git push --force-with-lease`, `git push -f`, and any other destructive remote operations from inside pilot sessions. Frame it as a scope-of-authority statement: the pilot's scope is content-only; git push of any kind is dispatch-lib's responsibility. Reference mika#1318 as the founding incident.

This mirrors the existing prohibition in `/mika-groom-plan-only` (mika-platform PR #134) but at the skill-prompt layer — the skill prompt governs mika-dev's understanding of what the pilot is allowed to do, while the entry command governs the pilot's own understanding.

**Patterns to follow:** The dev-pilot `system_prompt.md` is terse and imperative; the dev-groom prompt should match that register. The `/mika-groom-plan-only` command's Phase 2 step 7 has the canonical force-push prohibition language.

**Test scenarios:**
- Test expectation: none — prompt-only change, no behavioral code modified.

**Verification:** Read the updated `system_prompt.md` and confirm the prohibition is present and unambiguous. Verify the skill's `skill.toml` version is bumped to reflect the prompt change.

---

### U2. Post-flight reflog guard in dispatch-lib

**Goal:** After the pilot session exits and before `_push_branch` runs, scan the worktree's reflog for force-push events. If detected, annotate the RESULT envelope with a structural violation and skip `_push_branch` (no point pushing over a force-pushed state).

**Requirements:** R2, R3, R4, R5

**Dependencies:** None (can be implemented in parallel with U1)

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh`
- `skills/bundled/_shared/tests/test_force_push_guard.sh` (new)

**Approach:** Add a new internal helper `_check_pilot_force_push` that:

1. Guards on `$SKILL = "dev-groom"` — returns 0 (no violation) for all other skills (R5).
2. Guards on `$WORKTREE_DIR` being set and existing.
3. Compares `PRE_RUN_REMOTE_HEAD` (captured before pilot launch via `git ls-remote origin "refs/heads/$BRANCH" | cut -f1`) with `POST_RUN_REMOTE_HEAD` (queried after pilot exit via the same command). If they differ, the pilot pushed to the remote — a scope-of-authority violation.
4. If detected, sets `PUSH_VIOLATION_DETECTED=1` and captures both SHAs into `PUSH_VIOLATION_EVIDENCE`.
5. Returns 1 (violation detected).

**Callsite convention (per review-guide.md § KISS):** `dispatch_claude_pilot()` calls `_check_pilot_force_push` **unconditionally** — the function handles skill-scoping internally via the `$SKILL = "dev-groom"` early-return guard. This avoids duplicating the skill-check logic at the callsite and keeps the dispatch function's flow linear.

**Pre-capture site:** `PRE_RUN_REMOTE_HEAD` must be captured in `dispatch_claude_pilot()` *before* `_run_claude_pilot` is invoked. Add it adjacent to the existing `PRE_RUN_HEAD` capture (local HEAD), so both pre-run anchors are co-located.

Call `_check_pilot_force_push` in `dispatch_claude_pilot()` after `_run_claude_pilot` returns and after the post-flight diff check block, but before the `_iterate_groom_loop` call. If violation detected:
- Prepend a `STRUCTURAL VIOLATION: pilot force-push detected (mika#1318)` block to RESULT with the reflog evidence.
- Set the Outcome to `PIPELINE_INCOMPLETE — force-push violation`.
- Skip `_iterate_groom_loop` and `_push_branch` — the dispatch is poisoned.
- Fall through to `_deliver_callback` so mika-dev receives the violation.

The guard placement (after post-flight diff check, before iterate loop) ensures:
- `PRE_RUN_HEAD` / `POST_RUN_HEAD` are already computed.
- The reflog exists in the worktree (not yet cleaned up).
- `_push_branch` does not overwrite evidence by pushing again.

**Patterns to follow:** The existing post-flight diff check (`PRE_RUN_HEAD = POST_RUN_HEAD`) and policy-deny pre-check (Class C disambiguation) at dispatch-lib.sh:704-750 are the structural pattern — a guard that reads evidence from the worktree/stderr and annotates RESULT before the next dispatch phase.

**Test scenarios:**
- Happy path: dev-groom dispatch with no pilot push — `PRE_RUN_REMOTE_HEAD == POST_RUN_REMOTE_HEAD`, guard returns 0, dispatch proceeds normally to iterate loop.
- Pilot push detected: `PRE_RUN_REMOTE_HEAD != POST_RUN_REMOTE_HEAD` for a dev-groom dispatch — guard returns 1, RESULT contains `STRUCTURAL VIOLATION`, Outcome is `PIPELINE_INCOMPLETE`, `_iterate_groom_loop` is skipped.
- Dev-pilot dispatch: `$SKILL = "dev-pilot"` — guard returns 0 regardless of remote state (R5, early-return on skill check).
- No worktree: `$WORKTREE_DIR` is empty — guard returns 0 (free-text mode, no worktree to scan).
- Branch not on remote: `PRE_RUN_REMOTE_HEAD` is empty (branch doesn't exist on remote yet), `POST_RUN_REMOTE_HEAD` is also empty — guard returns 0 (no push occurred). If `POST_RUN_REMOTE_HEAD` is non-empty, pilot created the remote branch — violation detected.
- Network failure: `git ls-remote` exits non-zero — guard returns 0 (fail-open, not fail-closed — a network failure shouldn't block a legitimate dispatch; `_push_branch` will fail independently if the remote is truly unreachable).

**Verification:** Run `test_force_push_guard.sh` — all scenarios pass. Deploy and verify the pilot push guard Signal (see U3) appears in server logs on the first dev-groom dispatch.

---

### U3. Signal documentation for post-deploy verification

**Goal:** Add a new Signal entry to `CLAUDE.md` so operators can verify force-push detection is active after deploy.

**Requirements:** R6

**Dependencies:** U2

**Files:**
- `CLAUDE.md`

**Approach:** Add the next available Signal letter (verify against `CLAUDE.md` at implementation time — currently Signal L is the last entry, so the next is Signal M; if another PR ships a Signal M before this one, use Signal N) as `Signal <letter> — pilot push guard` to the existing Signal list in the `### Post-restart safety check (#757)` section. The signal is:

```
grep pilot_push_guard server.log
```

Two sub-events:
- `pilot_push_guard.clean` — guard ran, no violation detected (expected on every dev-groom dispatch).
- `pilot_push_guard.violation` — guard ran, pilot push detected (should never appear; investigate immediately if it does).

Since dispatch-lib writes to stderr (not structured JSON logs), the signal is emitted via `echo` to stderr with a structured prefix that the operator can grep. This follows the existing dispatch-lib diagnostic pattern (e.g., `push_branch:` at line 1267).

**Patterns to follow:** Signal L (`identical_diff_circuit_breaker`) is the most recent Signal addition — follow its structure.

**Test scenarios:**
- Test expectation: none — documentation-only change.

**Verification:** Read `CLAUDE.md` and confirm the pilot push guard Signal is present with both sub-event descriptions, using the correct next-available letter.

---

## Open Questions

None — all design decisions are resolved. The tier1/TIER3 layer is the primary prevention; this plan adds defense-in-depth detection and prompt-level prohibition.

---

## Sources & Research

- mika#1318 issue body — incident transcript, evidence, latent risk framing
- mika#1318 comment (PR #134 draft) — prompt-layer half already shipped on mika-platform
- `claude-pilot-py/src/claude_pilot/tier1.py` — existing TIER3 deny patterns (confirmed `git push --force` and `git push -f` are hard-denied) and `is_safe_git_command()` force-flag blocking. Line numbers approximate as of plan date (2026-06-26); grep for `TIER3_DENIED` and `_FORCE_FLAG_RE` for current locations.
- `skills/bundled/_shared/dispatch-lib.sh` — `_push_branch` (sole push site, legitimate force-with-lease for rebased branches), post-flight diff check pattern (structural model for U2), `_iterate_groom_loop` (must be skipped on push violation). Line numbers approximate; grep for function names for current locations.

---

## Acceptance criteria

- AC1 (structural): `system_prompt.md` contains `### Authority bounds` subsection explicitly prohibiting all `git push` variants
- AC2 (structural): `_check_pilot_force_push` present in `dispatch-lib.sh`; guards on `$SKILL = "dev-groom"` early-return; compares `PRE_RUN_REMOTE_HEAD` vs post-run `git ls-remote` result
- AC3 (structural): on violation, RESULT prefixed `STRUCTURAL VIOLATION:`; `_iterate_groom_loop` and `_push_branch` skipped; `_deliver_callback` called
- AC4 (structural): `$SKILL = "dev-pilot"` early-return — dev-pilot unaffected
- AC5 (structural/doc): Signal M `pilot_push_guard` in `CLAUDE.md` with `.clean` / `.violation` sub-events
- AC6 (structural): `skill.toml` version bumped to 0.3.0
- AC7 (CI-deferred): `test_force_push_guard.sh` all 7 scenarios pass

---

## Revision history

- rev 2 (2026-06-26): addressed F1 by replacing unverified reflog `(forced update)` detection with authoritative `git ls-remote` remote-ref comparison (pre/post pilot session); addressed F2 by adding "Recovery posture" section documenting remote state, local state, callback state, and operator recovery steps after the guard fires (citation: review-guide.md § Single Responsibility); addressed F3 by stating explicitly that `dispatch_claude_pilot()` calls `_check_pilot_force_push` unconditionally with internal skill-scoping (citation: review-guide.md § KISS); addressed F4 by replacing hardcoded "Signal M" with "next available letter" + verification instruction to check CLAUDE.md at implementation time; addressed F5 by replacing brittle line-number references with grep-for-symbol instructions and noting references are approximate as of plan date.
