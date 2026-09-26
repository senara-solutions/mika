---
title: "fix: dispatch-lib.sh should pass /ce:work for groomed plans"
type: fix
status: active
date: 2026-05-11
issue: 1074
---

# fix: dispatch-lib.sh should pass /ce:work for groomed plans

## Overview

When `dispatch_claude_pilot` dispatches a session for a ticket with a groomed plan-on-branch, it currently always passes `--command "/mika"` for the `dev-pilot` skill. The model must then "decide" to invoke `/ce:work` — but it sometimes narrates instead of acting, causing the narrate-then-exit failure class. This fix detects the plan callout in the issue body and passes `--command "/ce:work <PLAN_PATH>"` directly, eliminating the model's opportunity to narrate instead of act.

## Problem Frame

The grooming pipeline writes a callout into the issue body:
```
> - **Plan:** `docs/plans/<filename>.md` (committed on branch @ `<sha>`)
```

The `/mika` slash command (Claude Code side) already detects this and skips `/ce:plan`, jumping to `/ce:work`. But this detection happens *inside* the Claude Code session — the model must parse the issue, detect the plan, and invoke `/ce:work`. This is where the narrate-then-exit failure occurs: the model recognizes it should call `/ce:work` but instead narrates "Proceeding to /ce:work" and calls `end_turn`.

The structural fix is to move plan-on-branch detection *upstream* to `dispatch-lib.sh` (the shell script that launches `claude-pilot`), so the entry command is already `/ce:work <PLAN_PATH>` and the model has no decision to make.

## Requirements Trace

- R1. `dispatch_claude_pilot` in `_shared/dispatch-lib.sh` parses the issue body for plan callout before choosing the entry command
- R2. When plan callout is found AND the file exists in the worktree, `ENTRY_COMMAND` is set to `/ce:work <PLAN_PATH>` instead of `/mika`
- R3. When no plan callout is found OR the file doesn't exist, behavior is unchanged (`ENTRY_COMMAND="/mika"`)
- R4. The plan path is validated (`test -f`) before overriding the entry command

## Scope Boundaries

- Only the `dev-pilot` skill arm is affected; `dev-groom` is unchanged
- The plan callout detection uses the same pattern as `/mika` command and self-dev prompt: `> - **Plan:**` followed by a backtick-wrapped path containing `docs/plans/`
- No changes to `claude-pilot` itself, only to the entry command passed to it
- The existing `/mika` command's plan-on-branch detection (prompt-level) remains as defense-in-depth — this change makes it rarely exercised but does not remove it

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/_shared/dispatch-lib.sh` — the skill-to-entry-command mapping is at lines 457-462, a case switch. The `_set_up_worktree()` function (line 162-293) already parses the issue body into `$ISSUE_BODY` and sets up `$WORKTREE_DIR`
- `skills/bundled/self-dev/system_prompt.md` line 253 — the bypass predicate is `Plan: docs/plans/` (substring match including the path prefix to avoid false positives on "Plan:" in prose)
- `.claude/commands/mika.md` line 17 — the exact callout shape: `> - **Plan:** \`docs/plans/<filename>.md\` (committed on branch @ <sha>)`

### Institutional Learnings

- `docs/solutions/best-practices/auto-groom-on-dispatch-2026-05-06.md` — documents the grooming-as-dispatch-phase pattern and the plan callout bypass predicate
- 10+ prior prompt-enforcement failures document the unreliability of prompt-only enforcement for this failure class (referenced in #1074)

## Key Technical Decisions

- **Detection placement:** After `_set_up_worktree()` and before `_run_claude_pilot()` — because `_set_up_worktree()` populates `$ISSUE_BODY` and `$WORKTREE_DIR` which are both needed for detection and validation
- **Override scope:** Only for `SKILL=dev-pilot` — the `dev-groom` arm has its own entry command (`/mika-groom-ticket`) which should never be overridden
- **Validation with `test -f`:** The plan path must exist in the worktree before overriding — if the branch was rebased and the plan file was lost, fall back to `/mika` gracefully
- **Regex pattern:** Use `grep -oP` to extract the path from the callout, matching the exact shape `> - **Plan:** \`(docs/plans/[^\`]+)\`` — this is consistent with the self-dev prompt's bypass predicate requiring `docs/plans/` prefix

## Open Questions

### Resolved During Planning

- **Where to put the override logic?** After `_set_up_worktree()` in `dispatch_claude_pilot()`. The case switch sets the default, then the plan-on-branch detection overrides it conditionally. This keeps the case switch clean and the override logic localized.
- **Should this be a new function or inline?** New helper function `_detect_plan_on_branch()` — consistent with the `_`-prefixed internal helper pattern in the file, and keeps `dispatch_claude_pilot()` readable.

### Deferred to Implementation

- Exact `grep` flags for portable regex extraction (GNU vs BSD grep compatibility — the handler runs on Linux so `grep -oP` with PCRE is available)

## Implementation Units

- [ ] **Unit 1: Add plan-on-branch detection helper**

**Goal:** Add `_detect_plan_on_branch()` that parses `$ISSUE_BODY` for the plan callout, extracts the path, validates it exists in `$WORKTREE_DIR`, and sets `ENTRY_COMMAND` to `/ce:work <path>` when found.

**Requirements:** R1, R2, R3, R4

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`

**Approach:**
- Add a new `_detect_plan_on_branch()` helper function following the existing `_`-prefixed convention
- The function checks: (1) `SKILL` is `dev-pilot`, (2) `ISSUE_BODY` is non-empty, (3) the callout pattern matches, (4) extracted path passes `test -f` in `WORKTREE_DIR`
- When all checks pass, override `ENTRY_COMMAND` to `/ce:work <path>` and emit an informational stderr message
- When any check fails, return silently (no-op, preserving default behavior)
- Call this function in `dispatch_claude_pilot()` after `_set_up_worktree()` and before `_handle_dry_run()`

**Patterns to follow:**
- `_set_up_worktree()` for variable naming and stderr logging conventions
- The self-dev prompt's bypass predicate pattern: substring match on `Plan: docs/plans/`

**Test scenarios:**
- Happy path: issue body contains `> - **Plan:** \`docs/plans/feat-plan.md\``, file exists in worktree -> ENTRY_COMMAND overridden to `/ce:work docs/plans/feat-plan.md`
- Edge case: issue body contains the Plan callout but the file does NOT exist in worktree -> ENTRY_COMMAND remains `/mika`
- Edge case: issue body contains "Plan:" in prose but NOT as the structured callout (no `docs/plans/` prefix) -> ENTRY_COMMAND remains `/mika`
- Edge case: `SKILL` is `dev-groom` (not `dev-pilot`) -> function is a no-op regardless of issue body
- Edge case: `ISSUE_BODY` is empty (free-text mode, no issue fetched) -> function is a no-op
- Edge case: plan path contains spaces or special characters -> correctly extracted and validated

**Verification:**
- `bash skills/bundled/_shared/test-dispatch-lib.sh` passes with new test cases
- The case switch (lines 457-462) is unchanged — the override is applied after it

- [ ] **Unit 2: Add test cases for plan-on-branch detection**

**Goal:** Extend `test-dispatch-lib.sh` with structural tests verifying the plan-on-branch detection logic.

**Requirements:** R1, R2, R3, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `skills/bundled/_shared/test-dispatch-lib.sh`

**Approach:**
- Add a new test section (Test 6) following the existing pattern of structural verification via `sed`/`grep` on the source file
- Verify the helper function exists, checks for `dev-pilot` skill, uses the correct callout pattern, validates file existence, and overrides `ENTRY_COMMAND`
- Verify the function is called at the right point in `dispatch_claude_pilot()` (after `_set_up_worktree`, before `_handle_dry_run`)

**Patterns to follow:**
- Existing test structure in `test-dispatch-lib.sh` — `assert_contains`/`assert_not_contains`/`assert_eq` helpers, section-based organization, structural verification via source inspection

**Test scenarios:**
- Happy path: test script runs successfully with all new assertions passing
- Integration: verify the function call ordering in `dispatch_claude_pilot()` matches the expected sequence

**Verification:**
- `bash skills/bundled/_shared/test-dispatch-lib.sh` exits 0 with all tests passing

## System-Wide Impact

- **Interaction graph:** `dispatch-lib.sh` -> `claude-pilot` -> Claude Code session. The entry command change is the only interaction affected. Downstream, `/ce:work` receives the plan path and executes it — this path is already exercised when `/mika` detects plan-on-branch prompt-side
- **Error propagation:** If plan detection fails (file missing, pattern doesn't match), the fallback is the current behavior (`/mika`). No new failure modes introduced
- **State lifecycle risks:** None — the detection is stateless, reading `$ISSUE_BODY` which is already populated
- **Unchanged invariants:** The case switch contract (mika#932) is unchanged. The `dev-groom` arm is unaffected. The `_set_up_worktree()` function is unmodified

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| False positive plan detection on prose containing "Plan:" | Pattern requires `docs/plans/` prefix and backtick-wrapped path, consistent with self-dev bypass predicate |
| Plan file exists but is stale/wrong branch | `test -f` validates presence; staleness is a pre-existing concern not introduced by this change |
| `grep -oP` not available | Handler runs on Linux (Gentoo) where GNU grep with PCRE is standard; add fallback comment |

## Sources & References

- Related issues: #1074, #1072 (prompt-level mitigation)
- Related code: `skills/bundled/_shared/dispatch-lib.sh`, `skills/bundled/self-dev/system_prompt.md`
- Related learnings: `docs/solutions/best-practices/auto-groom-on-dispatch-2026-05-06.md`
