---
title: "fix: Add dispatch-lib brake for dev-groom idempotency-bypass-architect fabrication"
type: fix
status: completed
date: 2026-05-27
---

# fix: Add dispatch-lib brake for dev-groom idempotency-bypass-architect fabrication

## Overview

When the dev-groom pilot finds an already-committed plan on HEAD from a prior groom attempt, it treats the work as complete and exits with a fabricated success message without invoking the architect. This bypasses the architect roundtrip entirely, producing `PIPELINE FAILURE: HEAD unchanged` callbacks because no `Outcome: PLAN_GROOMED` marker is emitted. Three dispatches failed this way on 2026-05-27 (mika#806 and mika#736 x2).

This fix adds a structural brake in dispatch-lib's post-flight checks to detect the fabrication string, plus a structural test to verify the brake exists. The `/mika-groom-plan-only` prompt fix (which removes the instruction that produces the fabrication) is deferred to a follow-on per mika#1319 scope authorization ("Vincent's morning read").

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

- R1. dispatch-lib detects the fabrication string in the session log and classifies it as a distinct PIPELINE FAILURE sub-type (`idempotency-bypass-architect`)
- ~~R2. (deferred — see Scope Boundaries)~~
- ~~R3. (deferred — see Scope Boundaries)~~
- R4. Existing post-flight checks (HEAD unchanged, plan validation, PR existence) remain orthogonal and unchanged
- R5. Structural test in `test-dispatch-lib.sh` verifies the new detection block exists

## Scope Boundaries

- This fix targets the dispatch-lib detection brake and the `/mika-groom-plan-only` prompt only
- No engine-side changes
- No skill-prompt restructure of the dev-groom skill (`skills/bundled/dev-groom/system_prompt.md`) -- that skill's prompt describes the callback shape, not the pilot's behavior
- No force-push authorization changes (tracked separately as mika#1318)

### Deferred to Separate Tasks

- Full skill-prompt restructure (Option A from ticket) -- follow-on after this dispatch-lib brake ships. **Includes the `/mika-groom-plan-only` prompt fix (R2, R3)**, which the mika#1319 issue body explicitly scopes as "Vincent's morning read" follow-on, not tonight's dispatch.
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
- **60-char distinctive substring vs full 87-char fabrication string**: The issue AC specifies "substring-match the bit-identical 87-character fabrication string." This plan uses the distinctive portion `Architect convergence pending via dispatch-lib iterate loop` (60 chars) rather than the full `Plan committed and pushed. Architect convergence pending via dispatch-lib iterate loop.` (87 chars). Rationale: the 60-char substring is the semantically distinctive portion — the `Plan committed and pushed.` prefix is generic phrasing that could appear in legitimate success messages. The 60-char tail is unique to this fabrication class (it references dispatch-lib's internal iterate loop, which no legitimate pilot output would). False-positive risk is negligible for the distinctive portion alone, while matching the full string risks false-negatives if the pilot paraphrases only the generic prefix (e.g., "Plan committed." instead of "Plan committed and pushed."). Either choice satisfies the AC's intent (structural detection of this fabrication); the distinctive substring is the more robust match target. (review-guide.md § KISS — prefer the simpler choice that is also more robust.)
- **Prepend to RESULT, not replace**: follow the existing pattern where post-flight checks prepend `PIPELINE FAILURE:` to `$RESULT`, preserving the original claude-pilot output for diagnostic context.
- **Detection fires for dev-groom only**: guarded by `$SKILL = "dev-groom"` to avoid false positives on other dispatch skills.
- **Prompt fix removes the instruction entirely**: Phase 3 step 8 should not prescribe a specific exit string. The pilot should output whatever confirms the artifacts it produced. The idempotent re-groom path (Phase 2 step 4) should instruct the pilot to re-commit the existing plan (ensuring HEAD advances) rather than silently treating it as done.

## Implementation Units

- [x] **Unit 1: Add fabrication-string detection to dispatch-lib post-flight**

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
- **Double-firing with HEAD-unchanged check is accepted.** The existing `HEAD unchanged` check at line 453 fires first (it's earlier in the function). When both checks fire on the same session, `$RESULT` carries two `PIPELINE FAILURE:` prefixes: `PIPELINE FAILURE: [idempotency-bypass-architect detail] PIPELINE FAILURE: [HEAD unchanged detail]`. This is accepted because: (a) the outcome classifier at line 678 uses `grep -qF "PIPELINE FAILURE:"` which matches on the first occurrence — the doubled prefix does not affect classification; (b) the delivered callback text is diagnostic-only (mika-dev reads the outcome line, not the RESULT body, for dispatch decisions); (c) the more specific idempotency-bypass-architect marker appears first, giving operators the actionable diagnosis at a glance. Adding a dedup guard (e.g., skip HEAD-unchanged when fabrication is detected) would violate the orthogonality of the two checks — the fabrication check detects *why* the pilot misbehaved; the HEAD-unchanged check detects the *structural consequence*. Both diagnostics are independently useful. (review-guide.md § Single Responsibility — each check diagnoses one thing; review-guide.md § Orthogonality — independent checks should not suppress each other.)

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

- ~~**Unit 2: (deferred)** Fix /mika-groom-plan-only prompt — deferred to follow-on ticket per mika#1319 scope authorization ("Vincent's morning read"). R2/R3 are addressed there, not in this dispatch.~~

- [x] **Unit 2: Add structural test for fabrication detection block**

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
| Prompt fix is deferred — fabrication instruction remains in `/mika-groom-plan-only` | Unit 1's dispatch-lib brake catches the known variant structurally. The prompt fix (R2/R3) ships as a follow-on per mika#1319 scope authorization. Until then, the existing HEAD-unchanged check + the new fabrication brake provide two independent defenses. Future paraphrase variants would be caught by the HEAD-unchanged check. |
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

## Revision history

- rev 2 (2026-05-27): addressed F1 by removing Unit 2 (prompt fix to `/mika-groom-plan-only.md`) — operator scope authorization in mika#1319 explicitly defers it to follow-on ("Vincent's morning read"), R2/R3 struck from active requirements, risk table updated; addressed F2 by adding Key Technical Decisions entry justifying 60-char distinctive substring over full 87-char string (semantic distinctiveness, false-negative robustness, review-guide.md § KISS); addressed F3 by expanding Unit 1's Approach with explicit acceptance of double-firing — both diagnostics are independently useful and the outcome classifier is unaffected, citing review-guide.md § Single Responsibility and § Orthogonality.
