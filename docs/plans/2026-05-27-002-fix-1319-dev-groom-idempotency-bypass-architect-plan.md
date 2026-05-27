---
title: "fix: Add dispatch-lib brake for dev-groom idempotency-bypass-architect fabrication"
type: fix
status: active
date: 2026-05-27
---

# fix: Add dispatch-lib brake for dev-groom idempotency-bypass-architect fabrication

## Overview

When the dev-groom pilot finds an already-committed plan on HEAD from a prior groom attempt, it treats the work as complete and exits with a fabricated success message without invoking the architect. This bypasses the architect roundtrip entirely, producing `PIPELINE FAILURE: HEAD unchanged` callbacks because no `Outcome: PLAN_GROOMED` marker is emitted. Three dispatches failed this way on 2026-05-27 (mika#806 and mika#736 x2).

This fix adds a structural brake in dispatch-lib's post-flight checks to detect the fabrication string, and fixes the `/mika-groom-plan-only` prompt that instructs the pilot to output it.

## Problem Frame

The failure chain:
1. A prior groom attempt committed a plan on the branch (e.g., Vincent's morning interactive groom)
2. The autonomous dev-groom dispatches the pilot into the same worktree
3. The pilot sees the plan commit on HEAD, concludes "work already done"
4. The pilot outputs `"Plan committed and pushed. Architect convergence pending via dispatch-lib iterate loop."` -- a fabrication (dispatch-lib's `_iterate_groom_loop` receives the architect verdict from within the same dispatch, not separately)
5. dispatch-lib's pre/post HEAD SHA check fires (`HEAD unchanged`), classifying as `PIPELINE_INCOMPLETE`
6. No `Outcome: PLAN_GROOMED` is emitted, so mika#1289's auto-fire hook does not trigger

The fabrication string is bit-identical across all three failures, making it a reliable structural detection target.

## Requirements Trace

- R1. dispatch-lib detects the 87-character fabrication string in the session log and classifies it as a distinct PIPELINE FAILURE sub-type (`idempotency-bypass-architect`)
- R2. The `/mika-groom-plan-only` prompt no longer instructs the pilot to output the fabrication string
- R3. The `/mika-groom-plan-only` prompt's idempotent re-groom path (Phase 2 step 4) gives the pilot actionable instructions for the prior-commit case instead of silently deferring
- R4. Existing post-flight checks (HEAD unchanged, plan validation, PR existence) remain orthogonal and unchanged
- R5. Structural test in `test-dispatch-lib.sh` verifies the new detection block exists

## Scope Boundaries

- This fix targets the dispatch-lib detection brake and the `/mika-groom-plan-only` prompt only
- No engine-side changes
- No skill-prompt restructure of the dev-groom skill (`skills/bundled/dev-groom/system_prompt.md`) -- that skill's prompt describes the callback shape, not the pilot's behavior
- No force-push authorization changes (tracked separately as mika#1318)

### Deferred to Separate Tasks

- Full skill-prompt restructure (Option A from ticket) -- follow-on after this dispatch-lib brake ships
- Engine-side `idempotency-bypass-architect` recognition for structured reaper handling -- future iteration if the brake proves insufficient

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/_shared/dispatch-lib.sh` lines 596-641: existing dev-groom post-flight validation block. Uses `$SESSION_LOG` (`/var/log/claude-pilot/${LOG_ID}.log`) for `/ce:plan` invocation grep. The new detection should follow the same pattern -- grep the session log for the fabrication string
- `skills/bundled/_shared/dispatch-lib.sh` lines 674-698: outcome classification block. `PIPELINE FAILURE:` prefix drives `PIPELINE_INCOMPLETE` outcome
- `skills/bundled/_shared/test-dispatch-lib.sh`: structural grep-based tests using `assert_eq`, `assert_contains`, `assert_not_contains` helpers. Tests verify code structure via grep on dispatch-lib source, not integration execution
- `.claude/commands/mika-groom-plan-only.md`: the pilot's entry command prompt, Phase 2 step 4 (idempotent re-groom) and Phase 3 step 8 (exit text)
- The dev-groom case switch at line 1566-1579 now maps to `/mika-groom-plan-only` (not `/mika-groom-ticket`)

### Institutional Learnings

- `feedback_prompt_enforcement_fragile`: never rely on prompt rules alone for enforcement -- use structural constraints. This fix follows the hybrid pattern: prompt fix (remove the fabrication instruction) + structural brake (dispatch-lib detection)
- `docs/solutions/agent-quirks/dev-groom-fabricated-verdict-2026-05-20.md`: dispatcher skills fabricate verdicts under suffix-line pressure. The `/mika-groom-plan-only` prompt's Phase 3 step 8 literally instructs the pilot to output the fabrication string -- removing it is the primary fix
- `docs/solutions/best-practices/dev-groom-drift-detection-structural-validation-2026-05-11.md`: post-flight validation in dispatch-lib is the established pattern for catching pilot drift

## Key Technical Decisions

- **Session log grep (not PILOT_OUTPUT)**: `PILOT_OUTPUT_RAW` is the structured JSON envelope from claude-pilot's stdout, not the session transcript. The session log at `$SESSION_LOG` contains the full pilot transcript and is already used for the `/ce:plan` invocation check. The fabrication string appears in the pilot's final text output, which is captured in the session log.
- **Substring match, not regex**: the fabrication string is bit-identical across all observed failures. A simple `grep -qF` (fixed-string match) is more robust than regex and matches the existing `/ce:plan` detection pattern.
- **Prepend to RESULT, not replace**: follow the existing pattern where post-flight checks prepend `PIPELINE FAILURE:` to `$RESULT`, preserving the original claude-pilot output for diagnostic context.
- **Detection fires for dev-groom only**: guarded by `$SKILL = "dev-groom"` to avoid false positives on other dispatch skills.
- **Prompt fix removes the instruction entirely**: Phase 3 step 8 should not prescribe a specific exit string. The pilot should output whatever confirms the artifacts it produced. The idempotent re-groom path (Phase 2 step 4) should instruct the pilot to re-commit the existing plan (ensuring HEAD advances) rather than silently treating it as done.

## Implementation Units

- [ ] **Unit 1: Add fabrication-string detection to dispatch-lib post-flight**

**Goal:** Detect the bit-identical fabrication string in the dev-groom session log and classify as a distinct PIPELINE FAILURE sub-type.

**Requirements:** R1, R4

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`

**Approach:**
- Insert a new detection block inside the existing `if [ "$SKILL" = "dev-groom" ]` post-flight validation section (after line 641, before the PR discovery block at line 643). This keeps all dev-groom-specific post-flight checks co-located.
- Pattern: grep `$SESSION_LOG` for the fixed string `Architect convergence pending via dispatch-lib iterate loop` (the distinctive 60-char substring that is unique to this fabrication class).
- On match: prepend `PIPELINE FAILURE: dev-groom session exited without architect roundtrip (idempotency-bypass-architect). Pilot claimed architect convergence is pending via dispatch-lib but dispatch-lib's _iterate_groom_loop runs within this same dispatch — not separately.` to `$RESULT`.
- Fail-open: if `$SESSION_LOG` is unavailable or unreadable, skip with a stderr warning (same pattern as the `/ce:plan` check at lines 611-618).
- The existing `HEAD unchanged` check at line 453 fires first (it's earlier in the function). The new check adds diagnostic specificity -- both may fire on the same session, and the outcome classification at line 678 correctly uses `grep -qF "PIPELINE FAILURE:"` which matches either.

**Patterns to follow:**
- Lines 596-641: dev-groom plan validation block (same guard structure, same `$SESSION_LOG` access pattern, same `RESULT` prepend convention)
- Lines 609-618: session log availability check with fail-open warning

**Test scenarios:**
- Happy path: when session log contains the fabrication string and SKILL=dev-groom, RESULT is prepended with the idempotency-bypass-architect PIPELINE FAILURE marker
- Edge case: when session log is unavailable, detection is skipped with a warning (fail-open)
- Edge case: when SKILL=dev-pilot, the detection block does not fire even if session log contains the string
- Integration: the new PIPELINE FAILURE marker causes the outcome classification block (line 678) to emit `Outcome: PIPELINE_INCOMPLETE`

**Verification:**
- The grep pattern matches against a file containing the fabrication string
- The PIPELINE FAILURE marker includes `idempotency-bypass-architect` for structured recognition

- [ ] **Unit 2: Fix /mika-groom-plan-only prompt — remove fabrication instruction and fix idempotent path**

**Goal:** Remove the instruction that tells the pilot to output the fabrication string, and give the idempotent re-groom path actionable behavior.

**Requirements:** R2, R3

**Dependencies:** None (independent of Unit 1)

**Files:**
- Modify: `.claude/commands/mika-groom-plan-only.md`

**Approach:**
Two changes in the prompt:

1. **Phase 2 step 4 (idempotent re-groom, line 45):** Currently says "If a plan is found, reuse it as the starting point -- this is an idempotent re-groom case." This gives no actionable instruction, causing the pilot to treat it as "work done, exit." Change to instruct the pilot to: read the existing plan, still run `/ce:plan` (which will incorporate the prior plan as context), and produce a fresh commit. This ensures HEAD advances and the iterate loop's architect call has a real commit to work with.

2. **Phase 3 step 8 (exit text, line 59):** Currently prescribes the exact fabrication string. Remove the prescribed text. Replace with a generic instruction: "Output a brief confirmation of what was produced (plan file path and commit SHA)." This avoids creating a new bit-identical fabrication target while still giving the pilot a clear exit behavior.

Also update the "What this command does NOT do" section to explicitly state: "Do NOT claim architect convergence is pending elsewhere. The architect is invoked by dispatch-lib within the same dispatch lifecycle, not as a separate process."

**Patterns to follow:**
- `.claude/commands/mika-revise-plan.md` -- the content-only revise pilot command, which has a similar "do the content work, commit, exit" shape without prescribing specific exit text

**Test scenarios:**
- Test expectation: none -- prompt-only change, no behavioral test. The structural brake in Unit 1 provides defense-in-depth.

**Verification:**
- The prompt no longer contains the substring `Architect convergence pending via dispatch-lib iterate loop`
- Phase 2 step 4 instructs the pilot to run `/ce:plan` even when a prior plan exists
- Phase 3 step 8 does not prescribe a specific exit string

- [ ] **Unit 3: Add structural test for fabrication detection block**

**Goal:** Verify the new detection block exists in dispatch-lib via the established structural grep pattern.

**Requirements:** R5

**Dependencies:** Unit 1

**Files:**
- Modify: `skills/bundled/_shared/test-dispatch-lib.sh`

**Approach:**
- Add a new test section (Test N) following the existing pattern: extract the dev-groom post-flight block from dispatch-lib, assert it contains the fabrication detection string and the PIPELINE FAILURE marker.
- Verify the detection block is guarded by `dev-groom` skill check.
- Verify the detection block references `SESSION_LOG` (same log file used for `/ce:plan` detection).
- Verify the PIPELINE FAILURE marker contains `idempotency-bypass-architect`.

**Patterns to follow:**
- Test 6 (lines 182-227): plan-on-branch detection structural test -- extracts function body, asserts on key strings

**Test scenarios:**
- Happy path: all structural assertions pass against the modified dispatch-lib.sh

**Verification:**
- `bash skills/bundled/_shared/test-dispatch-lib.sh` passes with zero failures

## System-Wide Impact

- **Interaction graph:** The new PIPELINE FAILURE marker flows through the existing callback delivery path (`_deliver_callback` -> `mika ask --task-complete`) to mika-dev. mika-dev's existing `PIPELINE_INCOMPLETE` handling applies. No new callback shape.
- **Error propagation:** The `idempotency-bypass-architect` marker is a sub-type of the existing `PIPELINE FAILURE:` family. The outcome classification block at line 678 treats all PIPELINE FAILURE variants identically (`Outcome: PIPELINE_INCOMPLETE`). Engine reapers (mika#871, mika#1162) operate on the outcome line, not the PIPELINE FAILURE detail text.
- **State lifecycle risks:** None. The detection is read-only (session log grep) and the RESULT prepend is idempotent (prepending to a string that may already carry other PIPELINE FAILURE markers is the established pattern).
- **Unchanged invariants:** The existing HEAD-unchanged check, plan-validation check, and PR-existence check are unmodified. The iterate-groom-loop invocation at line 1601 is unmodified. The dev-groom case switch entry command (`/mika-groom-plan-only`) is unmodified.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Prompt fix doesn't prevent future fabrication variants (LLM paraphrases the string) | Unit 1's dispatch-lib brake catches the known variant structurally. Future paraphrase variants would be caught by the existing HEAD-unchanged check. The brake is defense-in-depth, not sole defense. |
| Session log unavailable in some environments | Fail-open pattern (skip with warning) matches existing `/ce:plan` check. The HEAD-unchanged check is the primary defense and does not depend on the session log. |
| False positive if a legitimate session mentions the fabrication string in diagnostic context | The string is 60 characters of very specific phrasing. Legitimate sessions would not produce this exact substring in normal operation. Risk accepted as negligible. |

## Sources & References

- Related issue: mika#1319 (this ticket)
- Related issue: mika#1318 (companion: force-push authority)
- Related issue: mika#1271 (contract refactor parent)
- Related issue: mika#1289 (engine auto-fire after groom success)
- Memory: `feedback_prompt_enforcement_fragile`
- Memory: `feedback_mika_dev_llm_fabricates_tool_errors`
- Solution: `docs/solutions/agent-quirks/dev-groom-fabricated-verdict-2026-05-20.md`
- Solution: `docs/solutions/best-practices/dev-groom-drift-detection-structural-validation-2026-05-11.md`
