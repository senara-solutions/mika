---
title: "fix: Prevent dev-groom autonomous drift on action-verb-dense tickets"
type: fix
status: active
date: 2026-05-11
---

# fix: Prevent dev-groom autonomous drift on action-verb-dense tickets

## Overview

When dev-groom runs autonomously on chore/action-verb-dense tickets, the Claude Code session drifts into executor mode — writing a 0-byte plan file, never invoking `/ce:plan`, pivoting to unrelated work (disk checks, worktree triage), and exiting `Success`. The fix adds a structural post-flight plan validation in `dispatch-lib.sh` (catches the failure) and hardens the dev-groom system prompt (reduces the failure frequency).

## Problem Frame

**Observed:** Session 5047f85f (mika issue#1031 groom dispatch) — 281s, 17 turns, $0.73, exit Success. The session created `docs/plans/2026-05-08-004-chore-rebase-1004-auto-groom-plan.md` as 0 bytes via Write tool, never invoked `/ce:plan`, skipped mika-arch entirely (Phase 3), pivoted to `df -h` and worktree iteration, then exited Success.

**Contrast:** Three successful dev-groom dispatches the same day (mika issue#1024, #1027, #1029) — all feature/investigation tickets with prose-heavy bodies. The failing ticket had action-verb-dense numbered AC steps ("rebase against origin/main", "force-push with lease", "run cargo build + cargo test").

**Root cause:** The LLM reads imperative verbs in the ticket body and mode-switches from "plan this work" to "do this work." The dispatch infrastructure has no post-flight validation specific to dev-groom — the existing HEAD-diff check only catches zero-commit failures, but a 0-byte committed plan file passes it.

**Institutional pattern:** This is instance N+1 of the prompt-rule-cheapness-bias pattern (`docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`). The fix must be primarily structural, with prompt hardening as defense-in-depth.

## Requirements Trace

- R1. dev-groom sessions that produce empty or missing plan files must be caught as PIPELINE FAILURE before callback delivery
- R2. dev-groom system prompt must explicitly frame ticket body content as planning input, not execution instructions
- R3. Existing dev-pilot behavior must not be affected (plan validation is dev-groom-specific)
- R4. Fix must be compatible with the shared dispatch library architecture (no handler duplication)

## Scope Boundaries

- This fix addresses the detection and frequency-reduction of drift, not root-cause elimination (LLM prompt adherence is probabilistic)
- Does not address H3 (whether chore-label tickets should bypass grooming entirely) — that's a separate architectural question per the ticket

### Deferred to Separate Tasks

- Chore-label grooming bypass policy: separate brainstorm if H3 narrows on subsequent dispatches
- Retry-on-drift automation: future iteration once detection is proven reliable

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/_shared/dispatch-lib.sh` lines 347–355: existing post-flight HEAD-diff check — the structural pattern to extend
- `skills/bundled/dev-groom/system_prompt.md`: the grooming prompt that unconditionally requires `/ce:plan` in Phase 2 Step 5
- `skills/bundled/dev-groom/skill.toml`: `required_suffix_lines` guard — fires on mika-dev's engine side, not inside the Claude Code session

### Institutional Learnings

- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`: N=8 instances of prompt rules failing where structural enforcement was needed. This fix follows the structural-first principle.
- `docs/solutions/dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md`: dispatch-lib crash fingerprinting patterns — informs how to structure the PIPELINE FAILURE message for debuggability.
- `docs/solutions/best-practices/shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md`: dispatch-lib architecture — confirms the fix belongs in the shared library with a skill-specific conditional.

## Key Technical Decisions

- **Structural validation in dispatch-lib.sh, not a new handler:** The shared library already has the post-flight diff check pattern. Adding plan validation as a skill-specific conditional (gated on `$SKILL = dev-groom`) keeps the single-library contract intact and prevents handler duplication. (see `docs/solutions/best-practices/shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md`)
- **Plan file non-empty check, not content parsing:** Checking for non-zero byte count is sufficient — a 0-byte plan is the observed failure mode, and content quality is the architect's job (Phase 3). Parsing markdown structure in bash would be fragile overkill.
- **Prompt hardening as defense-in-depth, not primary fix:** Per the institutional pattern, the structural check is the primary fix. Prompt changes reduce drift frequency but cannot eliminate it.

## Open Questions

### Resolved During Planning

- **Where does plan validation belong?** In `dispatch-lib.sh` post-flight section (lines 347–355), as a skill-conditional check after the existing HEAD-diff check. Not in a new handler, not in the skill prompt alone.
- **What counts as a valid plan for the structural check?** A file matching `docs/plans/*-plan.md` in the worktree with size > 0 bytes. The architect review (Phase 3) handles quality.

### Deferred to Implementation

- Exact glob pattern for plan file discovery may need adjustment based on the worktree directory structure at runtime.

## Implementation Units

- [ ] **Unit 1: Add dev-groom plan validation to dispatch-lib.sh**

**Goal:** After claude-pilot exits for dev-groom skill dispatches, validate that at least one non-empty plan file exists in the worktree's `docs/plans/` directory. Mark as PIPELINE FAILURE if validation fails.

**Requirements:** R1, R3, R4

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`
- Test: manual validation via dry-run or replay (bash script, no unit test framework)

**Approach:**
- Add a new check inside the post-flight section (after line 355, inside the `if [ -n "$PRE_RUN_HEAD" ] && [ -n "$REPO" ]` block)
- Gate on `$SKILL = dev-groom` — this validation is skill-specific, not generic
- Use `find "$WORKTREE_DIR/docs/plans" -name '*-plan.md' -size +0c` to check for non-empty plan files
- If no non-empty plan files found, prepend `PIPELINE FAILURE: dev-groom produced no valid plan file (empty or missing docs/plans/*-plan.md). Session likely drifted into executor mode.` to `$RESULT`
- This check is additive to (not replacing) the existing HEAD-diff check — both can fire independently

**Patterns to follow:**
- Existing HEAD-diff check at lines 347–355 — same PIPELINE FAILURE prefix, same prepend-to-RESULT pattern

**Test scenarios:**
- Happy path: dev-groom session creates a non-empty plan file → no PIPELINE FAILURE prefix added
- Error path: dev-groom session creates a 0-byte plan file → PIPELINE FAILURE prefix added with descriptive message
- Error path: dev-groom session creates no plan file at all → PIPELINE FAILURE prefix added
- Edge case: dev-pilot dispatch (not dev-groom) with no plan file → validation skipped, no PIPELINE FAILURE
- Edge case: dev-groom in free-text mode (no worktree, `PRE_RUN_HEAD` empty) → validation skipped gracefully

**Verification:**
- Dispatch a dev-groom dry-run against a chore ticket and confirm the validation path is reachable
- The PIPELINE FAILURE message should be distinct enough to grep in callback logs

- [ ] **Unit 2: Harden dev-groom system prompt against executor-mode drift**

**Goal:** Add explicit anti-drift guardrails to the dev-groom prompt that frame ticket body content as planning input and prohibit execution of ticket commands.

**Requirements:** R2

**Dependencies:** None (can be done in parallel with Unit 1)

**Files:**
- Modify: `skills/bundled/dev-groom/system_prompt.md`

**Approach:**
- Add an opening directive section (before Phase 1) with three explicit constraints:
  1. "You are a PLANNER. Your output is a plan document, not executed commands."
  2. "The ticket body is INPUT to your plan. Imperative verbs in the ticket ('rebase', 'force-push', 'run cargo test') describe what the PLAN should cover, not commands for you to execute."
  3. "You MUST invoke `/ce:plan` (Phase 2 Step 5) before proceeding to Phase 3. A plan file with 0 bytes is a failure."
- Keep the directives concise — 3-5 lines, not a wall of text that itself gets ignored
- Place them as a `### Critical Constraints` block immediately after the opening paragraph

**Patterns to follow:**
- The existing `required_suffix_lines` enforcement in `skill.toml` — declarative constraints work better than embedded instructions
- The "proactive state checking" convention from `mika/CLAUDE.md` — check state before writes

**Test scenarios:**
- Happy path: dev-groom dispatched on an action-verb-dense ticket → session invokes /ce:plan and produces a non-empty plan (probabilistic — reduces drift frequency)
- Integration: prompt changes don't break successful grooming of feature/investigation tickets (the three successful cases from the same day)

**Test expectation:** Prompt changes are probabilistic, not deterministic. The structural guardrail (Unit 1) is the reliable detection mechanism. This unit reduces drift frequency as defense-in-depth.

**Verification:**
- Read the updated prompt and confirm the anti-drift directives are positioned before Phase 1 instructions
- Confirm existing Phase 2 Step 5 `/ce:plan` requirement is preserved (not contradicted)

## System-Wide Impact

- **Interaction graph:** dispatch-lib.sh → callback delivery to mika-dev. The PIPELINE FAILURE prefix triggers mika-dev's failure handling path (retry or escalate). No new interaction surfaces.
- **Error propagation:** PIPELINE FAILURE propagates through the callback to mika-dev, which already handles this prefix for zero-commit failures. Same code path, different error message.
- **State lifecycle risks:** None — the plan validation is read-only (checks file existence/size, doesn't modify worktree state).
- **API surface parity:** dev-pilot dispatches are unaffected (validation gated on `$SKILL = dev-groom`). Future sibling skills would need their own validation cases if applicable.
- **Unchanged invariants:** The HEAD-diff check, callback delivery, crash recovery, and PR URL discovery remain unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Plan file glob misses a valid plan (false positive PIPELINE FAILURE) | Use the canonical `*-plan.md` suffix that `/ce:plan` always produces; test with existing plan filenames |
| Prompt hardening has no effect on some model variants | Structural check (Unit 1) is the primary fix; prompt is defense-in-depth only |
| Future sibling skills might need different post-flight validation | The `$SKILL` conditional is extensible — add new cases as needed |

## Sources & References

- Related issue: senara-solutions/mika#1033
- Related code: `skills/bundled/_shared/dispatch-lib.sh` (post-flight validation)
- Related code: `skills/bundled/dev-groom/system_prompt.md` (grooming prompt)
- Institutional learning: `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`
- Institutional learning: `docs/solutions/best-practices/shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md`
- Institutional learning: `docs/solutions/dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md`
