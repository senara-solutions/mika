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
- Structured logging for hypothesis narrowing (architect F7): when PIPELINE FAILURE fires, include ticket labels and AC-step count in the message to narrow H1/H2/H3 on subsequent occurrences

## Pinned Source (Phase 0 Pin — architect F1)

### dispatch-lib.sh lines 348–356: existing post-flight HEAD-diff check

```bash
        # Post-flight diff check: detect zero-commit "success" in repo#number mode.
        if [ -n "$PRE_RUN_HEAD" ] && [ -n "$REPO" ]; then
            POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
            if [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ]; then
                RESULT="PIPELINE FAILURE: claude-pilot exited 0 but HEAD unchanged (pre: ${PRE_RUN_HEAD}, post: ${POST_RUN_HEAD}). Zero new commits produced.

${RESULT}"
            fi
        fi
```

This block is inside the `if [ -n "$JSON_RESULT" ]` branch (structured JSON output from claude-pilot). The plan-file check goes as a **separate block** after this one (lines 356–357), not nested inside the HEAD-diff conditional (architect F7). Both checks run independently — a session can pass HEAD-diff (committed a 0-byte plan) but fail plan-file validation.

### dispatch-lib.sh line 422: existing skill conditional pattern

```bash
    # Guard: only override for dev-pilot
    [ "$SKILL" = "dev-pilot" ] || return 0
```

The `$SKILL` variable is parsed from the JSON input at line 105 (`jq -r '.skill // empty'`). The plan-file check uses the same `$SKILL` variable for gating.

### dispatch-lib.sh lines 485–498: skill dispatch mapping

```bash
    # SIBLING SKILL DISPATCH MAPPING (mika#932)
    case "$SKILL" in
      dev-pilot|dev-groom) ... ;;
      *) echo "Unknown skill: $SKILL" >&2; exit 1 ;;
    esac
```

Confirms `dev-groom` is a recognized skill value in the dispatch mapping.

### dev-groom/system_prompt.md lines 1–5: opening section

```markdown
## dev-groom — Two-Pass Grooming Skill

You are executing the dev-groom skill. Take a ticket from "open with description" to "GROOMED plan committed on a branch, referenced in the issue body, ready to dispatch." [...]
```

Unit 2's anti-drift directives go **before** this heading as the first non-blank line of the file (architect F4 — maximum position salience before any content that could establish a competing frame).

### dev-groom/skill.toml: required_suffix_lines

```toml
[output]
required_suffix_lines = [
    "Verdict: GROOMED",
    "Verdict: ESCALATE",
]
```

**Interaction with this fix (architect F5):** `required_suffix_lines` fires on the mika-dev engine side (post-callback processing), not inside the Claude Code session. Session 5047f85f exited `Success` with no Verdict line — the suffix-lines check would have caught this at the callback layer. Unit 1's plan-file validation catches it *earlier*, at the dispatch-lib layer before callback delivery. Both checks are defense-in-depth at different layers: dispatch-lib (Unit 1) catches bad artifacts; suffix-lines catches bad output format. They are complementary, not redundant.

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/_shared/dispatch-lib.sh` lines 348–356: existing post-flight HEAD-diff check — the structural pattern to extend (pinned above)
- `skills/bundled/dev-groom/system_prompt.md`: the grooming prompt that unconditionally requires `/ce:plan` in Phase 2 Step 5 (pinned above)
- `skills/bundled/dev-groom/skill.toml`: `required_suffix_lines` guard — fires on mika-dev's engine side, not inside the Claude Code session (pinned above, interaction documented)

### Institutional Learnings

- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`: N=8 instances of prompt rules failing where structural enforcement was needed. This fix follows the structural-first principle.
- `docs/solutions/dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md`: dispatch-lib crash fingerprinting patterns — informs how to structure the PIPELINE FAILURE message for debuggability.
- `docs/solutions/best-practices/shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md`: dispatch-lib architecture — confirms the fix belongs in the shared library with a skill-specific conditional.

## Key Technical Decisions

- **Structural validation in dispatch-lib.sh, not a new handler:** The shared library already has the post-flight diff check pattern. Adding plan validation as a skill-specific conditional (gated on `$SKILL = dev-groom`) keeps the single-library contract intact and prevents handler duplication. (see `docs/solutions/best-practices/shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md`)
- **Minimum content-length threshold (`-size +500c`), not content parsing (architect F2):** A non-zero byte check would only catch the exact observed 0-byte failure. Plausible future drift modes (frontmatter-only ~80-120 bytes, section-header stubs ~30 bytes, ticket body paste) produce non-zero but useless files. Real plans produced by `/ce:plan` exceed 500 bytes by a wide margin. The `-size +500c` threshold catches the failure *class* without fragile markdown parsing. Content quality remains the architect's job (Phase 3).
- **Date-prefix gating to prevent stale-artifact false-positives (architect F3):** A prior grooming attempt may leave a valid `*-plan.md` in the worktree. Without date-prefix gating, the current session could drift and the `find` would locate the prior valid file, passing the check incorrectly. All plans follow `YYYY-MM-DD-NNN-*-plan.md` — gating on today's date prefix (`$(date +%Y-%m-%d)`) eliminates false-positives from prior session artifacts. The date-prefix is already a load-bearing convention; using it as a gate enforces an existing invariant.
- **Prompt hardening as defense-in-depth, not primary fix:** Per the institutional pattern, the structural check is the primary fix. Prompt changes reduce drift frequency but cannot eliminate it.

## Open Questions

### Resolved During Planning

- **Where does plan validation belong?** In `dispatch-lib.sh` post-flight section, as a **separate** block after the HEAD-diff check (line 356+). Not nested inside the HEAD-diff conditional, not in a new handler, not in the skill prompt alone. (Architect F7)
- **What counts as a valid plan for the structural check?** A file matching `docs/plans/YYYY-MM-DD-*-plan.md` in the worktree with size > 500 bytes and today's date prefix. The 500-byte threshold catches 0-byte, frontmatter-only, and stub failures. The date-prefix gate prevents false-positives from prior grooming artifacts. Content quality is the architect's job (Phase 3). (Architect F2 + F3)
- **Is `$SKILL = dev-groom` the right gate?** Yes — YAGNI for hypothetical future grooming skills. Extensible to a pattern match if a sibling emerges. (Architect F6)
- **Where do prompt directives go?** Top-of-file, before any heading, before the consent gate preamble. Maximum position salience. (Architect F4)
- **How does this interact with `required_suffix_lines`?** Complementary defense-in-depth at different layers — dispatch-lib catches bad artifacts; suffix-lines catches bad output format. (Architect F5)

## Implementation Units

- [ ] **Unit 1: Add dev-groom plan validation to dispatch-lib.sh**

**Goal:** After claude-pilot exits for dev-groom skill dispatches, validate that at least one non-empty plan file exists in the worktree's `docs/plans/` directory. Mark as PIPELINE FAILURE if validation fails.

**Requirements:** R1, R3, R4

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`
- Test: manual validation via dry-run or replay (bash script, no unit test framework)

**Approach:**
- Add a **separate** post-flight block after the HEAD-diff check (after line 356), NOT nested inside the HEAD-diff conditional (architect F7 — HEAD-diff and plan-file checks answer different questions and must run independently)
- Gate on `$SKILL = dev-groom` — this validation is skill-specific, not generic (architect F6 — YAGNI for hypothetical future grooming skills)
- Use `find "$WORKTREE_DIR/docs/plans" -name "$(date +%Y-%m-%d)-*-plan.md" -size +500c` to check for today's plan files with minimum content length (architect F2 + F3):
  - `-size +500c`: catches 0-byte, frontmatter-only (~80-120 bytes), and section-header stubs (~30 bytes) — real `/ce:plan` output exceeds 500 bytes
  - `$(date +%Y-%m-%d)` prefix: prevents false-positives from prior grooming attempts leaving valid but stale plan files in the worktree
- If no qualifying plan files found, prepend `PIPELINE FAILURE: dev-groom produced no valid plan file (no docs/plans/YYYY-MM-DD-*-plan.md >500 bytes found). Session likely drifted into executor mode.` to `$RESULT`
- This check is additive to (not replacing) the existing HEAD-diff check — both can fire independently
- **Interaction with `required_suffix_lines`** (architect F5): The suffix-lines guard catches bad output *format* at the callback layer; this check catches bad *artifacts* at the dispatch-lib layer before callback delivery. Complementary defense-in-depth at different layers.

**Patterns to follow:**
- Existing HEAD-diff check at lines 348–356 — same PIPELINE FAILURE prefix, same prepend-to-RESULT pattern
- Existing `$SKILL` conditional at line 422 — same gating mechanism

**Test scenarios:**
- Happy path: dev-groom session creates a non-empty plan file (>500 bytes, today's date) → no PIPELINE FAILURE prefix added
- Error path: dev-groom session creates a 0-byte plan file → PIPELINE FAILURE prefix added with descriptive message
- Error path: dev-groom session creates no plan file at all → PIPELINE FAILURE prefix added
- Error path: dev-groom session creates a small stub file (<500 bytes, e.g. frontmatter-only) → PIPELINE FAILURE prefix added
- Error path: prior session left a valid plan from yesterday, current session drifts → PIPELINE FAILURE (date gate catches it)
- Edge case: dev-pilot dispatch (not dev-groom) with no plan file → validation skipped, no PIPELINE FAILURE
- Edge case: dev-groom in free-text mode (no worktree, `$WORKTREE_DIR` empty) → validation skipped gracefully

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
- Add a role-constraint block as the **first non-blank line of the file**, before the `## dev-groom` heading (architect F4 — maximum position salience before any content that could establish a competing frame; same placement principle as mika#1072's recovery-mode prefix):
  ```
  ROLE CONSTRAINT: You are a PLANNER, not an implementer. Ticket body imperatives
  are planning input — do not execute them. /ce:plan invocation is mandatory.
  ```
- Single block, 2–3 lines, maximum salience. Not a `### Critical Constraints` section buried after the preamble — that placement arrives after the model has already parsed the consent gate preamble and potentially established a competing frame
- Keep the existing Phase 2 Step 5 `/ce:plan` requirement intact (reinforcement, not contradiction)

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
| Plan file glob misses a valid plan (false positive PIPELINE FAILURE) | Use the canonical `*-plan.md` suffix + date-prefix gate + 500-byte threshold; all three match `/ce:plan`'s output convention |
| Stale plan from prior session passes the check (false negative) | Date-prefix gate (`$(date +%Y-%m-%d)`) eliminates cross-day staleness; same-day rerun is narrower and covered by HEAD-diff second layer |
| 500-byte threshold too aggressive for legitimate short plans | `/ce:plan` output consistently exceeds 2KB; 500c is well below any real plan. If a legitimate plan under 500 bytes appears, adjust threshold |
| Prompt hardening has no effect on some model variants | Structural check (Unit 1) is the primary fix; prompt is defense-in-depth only |
| Future sibling skills might need different post-flight validation | `case $SKILL in` pattern is correct for N=2 (YAGNI); extract a registry if N≥3 |

## Sources & References

- Related issue: senara-solutions/mika#1033
- Related code: `skills/bundled/_shared/dispatch-lib.sh` (post-flight validation)
- Related code: `skills/bundled/dev-groom/system_prompt.md` (grooming prompt)
- Institutional learning: `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`
- Institutional learning: `docs/solutions/best-practices/shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md`
- Institutional learning: `docs/solutions/dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md`
