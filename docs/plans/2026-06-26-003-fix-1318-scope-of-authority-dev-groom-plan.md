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
- `claude-pilot-py/src/claude_pilot/tier1.py:112-113` — TIER3 patterns hard-deny `git push --force` and `git push -f`
- `claude-pilot-py/src/claude_pilot/tier1.py:375` — `_FORCE_FLAG_RE` blocks force flags in `is_safe_git_command()`
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

### KTD-3: Detection via reflog `(forced update)` marker

**Decision:** Scan `git reflog show origin/<branch>` for entries containing `(forced update)` that occurred between `PRE_RUN_HEAD` capture and the post-flight check.

**Rationale:** Git's reflog records push operations on remote-tracking refs. A force-push produces an entry with the `(forced update)` suffix. By checking whether any such entries exist for `origin/$BRANCH` in the worktree's reflog, we can detect pilot force-pushes without parsing SDK transcripts. The reflog is local to the worktree and survives until worktree cleanup.

**Limitation:** The reflog on the remote-tracking ref is only updated if `git push` actually ran from this worktree. If the pilot somehow pushed from a different path (extremely unlikely in the worktree-isolated architecture), it wouldn't appear here. This is an acceptable gap — the worktree is the pilot's only working directory.

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
3. Checks `git -C "$WORKTREE_DIR" reflog show "refs/remotes/origin/$BRANCH" 2>/dev/null` for lines containing `(forced update)`.
4. If found, sets `FORCE_PUSH_DETECTED=1` and captures the reflog line(s) into `FORCE_PUSH_EVIDENCE`.
5. Returns 1 (violation detected).

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
- Happy path: dev-groom dispatch with no force-push — guard returns 0, dispatch proceeds normally to iterate loop.
- Force-push detected: reflog contains `(forced update)` for `origin/$BRANCH` — guard returns 1, RESULT contains `STRUCTURAL VIOLATION`, Outcome is `PIPELINE_INCOMPLETE`, `_iterate_groom_loop` is skipped.
- Dev-pilot dispatch: `$SKILL = "dev-pilot"` — guard returns 0 regardless of reflog state (R5).
- No worktree: `$WORKTREE_DIR` is empty — guard returns 0 (free-text mode, no worktree to scan).
- Empty reflog: `origin/$BRANCH` has no reflog entries (first push hasn't happened yet) — guard returns 0.
- Reflog command fails: git reflog exits non-zero (corrupt worktree) — guard returns 0 (fail-open, not fail-closed — a reflog failure shouldn't block a legitimate dispatch).

**Verification:** Run `test_force_push_guard.sh` — all scenarios pass. Deploy and verify Signal M (see U3) appears in server logs on the first dev-groom dispatch.

---

### U3. Signal documentation for post-deploy verification

**Goal:** Add a Signal M entry to `CLAUDE.md` so operators can verify force-push detection is active after deploy.

**Requirements:** R6

**Dependencies:** U2

**Files:**
- `CLAUDE.md`

**Approach:** Add `Signal M — pilot force-push guard` to the existing Signal list in the `### Post-restart safety check (#757)` section. The signal is:

```
grep force_push_guard server.log
```

Two sub-events:
- `force_push_guard.clean` — guard ran, no violation detected (expected on every dev-groom dispatch).
- `force_push_guard.violation` — guard ran, force-push detected (should never appear; investigate immediately if it does).

Since dispatch-lib writes to stderr (not structured JSON logs), the signal is emitted via `echo` to stderr with a structured prefix that the operator can grep. This follows the existing dispatch-lib diagnostic pattern (e.g., `push_branch:` at line 1267).

**Patterns to follow:** Signal L (`identical_diff_circuit_breaker`) is the most recent Signal addition — follow its structure.

**Test scenarios:**
- Test expectation: none — documentation-only change.

**Verification:** Read `CLAUDE.md` and confirm Signal M is present with both sub-event descriptions.

---

## Open Questions

None — all design decisions are resolved. The tier1/TIER3 layer is the primary prevention; this plan adds defense-in-depth detection and prompt-level prohibition.

---

## Sources & Research

- mika#1318 issue body — incident transcript, evidence, latent risk framing
- mika#1318 comment (PR #134 draft) — prompt-layer half already shipped on mika-platform
- `claude-pilot-py/src/claude_pilot/tier1.py:110-133` — existing TIER3 deny patterns (confirmed `git push --force` and `git push -f` are hard-denied)
- `claude-pilot-py/src/claude_pilot/tier1.py:366-382` — `is_safe_git_command()` blocks force flags from auto-approval
- `skills/bundled/_shared/dispatch-lib.sh:1185-1280` — `_push_branch` (sole push site, legitimate force-with-lease for rebased branches)
- `skills/bundled/_shared/dispatch-lib.sh:704-750` — post-flight diff check pattern (structural model for U2)
- `skills/bundled/_shared/dispatch-lib.sh:1883-2000` — `_iterate_groom_loop` (must be skipped on force-push violation)
