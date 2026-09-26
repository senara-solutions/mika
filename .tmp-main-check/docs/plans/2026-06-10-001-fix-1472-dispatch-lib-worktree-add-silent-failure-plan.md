---
title: "fix(dispatch-lib): worktree-add silent-failure pre-flight + stash-before-remove + structured diagnostic"
status: active
created: 2026-06-10
type: fix
origin: GitHub issue mika#1472
groom_session_id: 5c136813-4316-4bf2-b6cc-78309876f5e0
---

# fix(dispatch-lib): worktree-add silent-failure pre-flight + stash-before-remove + structured diagnostic

## Summary

Add a pre-flight check to `_set_up_worktree` in `skills/bundled/_shared/dispatch-lib.sh` that detects when the target branch is checked out at a non-canonical (pre-`derive-worktree-path`-invariant) path and cleans up the relic with stash-before-remove discipline (mirroring mika#1414). Add a structured `worktree_setup_failed:` diagnostic line when both `worktree add` attempts still fail, so the operator sees the actual reason instead of a silent exit-128 trap.

## Problem frame

mika#1472 captures a silent dispatch failure on mika#1324 (task `d0228cd0`, 2026-06-09 22:21Z, exit code 128). The crash trace shows both `worktree add` attempts failed in sequence:

```
+ git worktree add -b fix/1324/... <dashed-path> origin/fix/1324/...   # FAILS: branch exists locally
+ git worktree add <dashed-path> fix/1324/...                          # FAILS: branch checked out in OTHER worktree
+ _dispatch_lib_exit_trap
+ _EXIT_CODE=128
```

The branch `fix/1324/structural-gate-dispatch-eligible-input` was already checked out at the slashed-path `.claude/worktrees/fix/1324/structural-gate-dispatch-eligible-input/mika` — a relic from before the canonical `worktree_path_slug == sanitize(branch_ref)` invariant (mika-platform/CLAUDE.md § Cross-Repo Development). The dispatch-lib path-collision check passes because the *dashed* target path didn't exist, but the slashed-path worktree blocked both `worktree add` attempts.

Result: task delivered with `HANDLER CRASH (exit code 128). Script failed before building result.` No operator-visible diagnostic of WHY. Dispatch slot freed silently. Ready label removed without comment. The autonomous loop appeared to have processed the ticket while producing no work.

This is the fifth dispatch-lib silent-failure shape: prior closed siblings mika#1364 (force-with-lease gap), #1407 (stale-main mis-diagnosis), #1414 (dirty-worktree on resume), #1415 (worktree-setup clobbers `.claude/commands`).

## Scope

### In scope (this plan, this PR)

1. **Pre-flight cleanup** in `_set_up_worktree`: detect non-canonical worktree paths for the target branch, stash any dirty state with descriptive name, remove the relic.
2. **Structured diagnostic emission** when both `worktree add` attempts still fail, surfaced via stderr before the trap fires.
3. **Tests** in `skills/bundled/_shared/test-dispatch-lib.sh` using the existing structural-AST verification pattern (sed-extract code blocks, assert_contains/assert_eq).

### Deferred to follow-up

- Fixing why slashed-path worktrees exist in the first place (orchestrator-CC ad-hoc creation; pre-invariant relics). The fix here is "be robust to either path shape."
- Diagnostic-emission improvements for OTHER dispatch-lib failure surfaces.
- Restructuring `_dispatch_lib_exit_trap` to capture the last command/rc generally — current shape stays.

## Requirements

- **R1.** `_set_up_worktree` MUST detect when the target branch is already checked out at a path that differs from `$WORKTREE_DIR` (the canonical dashed-slug path).
- **R2.** When the non-canonical path's directory EXISTS on disk AND has uncommitted changes, the code MUST stash them with a descriptive name (`dispatch-lib-stale-worktree-cleanup-<branch-sanitized>-<ts>`) and log the stash ref to stderr BEFORE the destructive operation.
- **R3.** After the stash check, the non-canonical worktree MUST be removed via `git worktree remove --force` so subsequent `worktree add` attempts can proceed on the canonical path.
- **R4.** When the non-canonical path's directory is MISSING on disk (registered in git but path deleted — common for relics), the stash check MAY be skipped; `worktree remove --force` is safe directly.
- **R5.** When both `worktree add` attempts still fail (after the pre-flight cleanup), the script MUST emit a structured line to stderr containing the branch name, both attempts' stderr text, and a `worktree_setup_failed:` prefix BEFORE the trap fires.
- **R6.** Existing path-collision check (for the dashed-path itself) remains unchanged.
- **R7.** Existing `worktree add` attempts remain unchanged in shape; only their failure surface is wrapped.

## Key Technical Decisions

### KTD1. Pre-flight detection via `worktree list --porcelain`

`git worktree list --porcelain` outputs `worktree <path>` followed by `branch refs/heads/<name>` pairs. An awk one-liner can parse this and emit the path for a matching branch — cheap, no extra subprocess calls beyond the one `git` invocation.

### KTD2. Stash discipline mirrors mika#1414

`_clean_worktree_for_rebase` (introduced by mika#1414) already implements the "stash dirty residue with a descriptive name, log the stash ref" pattern. KTD2 says: reuse that pattern verbatim for the stale-relic cleanup. Same naming convention (`dispatch-lib-<context>-<branch-sanitized>-<ts>`), same stderr log shape, same recovery handle for the operator.

### KTD3. Stderr capture for dual-failure diagnostic

The existing `worktree add -b ... || worktree add ...` chain doesn't capture per-command stderr. KTD3 wraps each attempt with `2>/tmp/wt-add-<n>-err` capture; on dual-failure, cat both buffers into the diagnostic line. Temp files are cleaned up on either success or dual-failure (no orphan files in /tmp).

### KTD4. Tests use structural-AST pattern, NOT real git ops

`test-dispatch-lib.sh` verifies dispatch-lib via `sed -n` source extraction + `assert_contains`/`assert_eq` (per the existing 19-test suite). KTD4 says: follow that pattern. No fixture-repo setup, no stubs. The new tests assert that:
- The pre-flight block contains the expected calls (`worktree list --porcelain`, branch-name comparison, `status --porcelain` check, `stash push -u -m`, `worktree remove --force`).
- The wrapped `worktree add` block contains the `worktree_setup_failed:` diagnostic line + stderr-capture redirects.
- Call ordering: pre-flight cleanup → existing path-collision check → wrapped `worktree add` attempts → (on dual-failure) diagnostic emission.

## High-Level Technical Design

```
┌─────────────────────────────────────────────────────────────┐
│ _set_up_worktree (BEFORE this fix)                          │
│                                                             │
│   [ -d <dashed-path> ] && remove --force <dashed-path>      │
│   ls-remote origin → fetch origin                           │
│   worktree add -b <branch> <dashed-path> origin/<branch>    │
│     || worktree add <dashed-path> <branch>                  │
│                                                             │
│ ❌ exits silently with rc=128 when branch is at a non-      │
│    canonical worktree path                                  │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│ _set_up_worktree (AFTER this fix)                           │
│                                                             │
│   ▶ NEW: pre-flight stale-relic cleanup                     │
│   existing_wt = $(git worktree list --porcelain | awk ...)  │
│   if [ -n "$existing_wt" ] && [ "$existing_wt" != "$WT" ]:  │
│     if [ -d "$existing_wt" ]:                               │
│       dirty = git -C "$existing_wt" status --porcelain      │
│       if dirty: stash push -u -m <descriptive-name>         │
│                  log "stashed as <name>; recover with ..."  │
│     git worktree remove --force "$existing_wt"              │
│                                                             │
│   ▶ EXISTING: path-collision check                          │
│   [ -d <dashed-path> ] && remove --force <dashed-path>      │
│   ls-remote origin → fetch origin                           │
│                                                             │
│   ▶ NEW: wrapped worktree add with diagnostic on dual-fail  │
│   if ! worktree add -b <branch> <dashed-path> origin/...    │
│        2>/tmp/wt-add-1-err; then                            │
│     if ! worktree add <dashed-path> <branch>                │
│          2>/tmp/wt-add-2-err; then                          │
│       echo "worktree_setup_failed: branch=$BRANCH" >&2      │
│       echo "  attempt 1: $(cat /tmp/wt-add-1-err)" >&2      │
│       echo "  attempt 2: $(cat /tmp/wt-add-2-err)" >&2      │
│       rm -f /tmp/wt-add-*-err                               │
│       return 1                                              │
│     fi                                                      │
│   fi                                                        │
│   rm -f /tmp/wt-add-*-err                                   │
└─────────────────────────────────────────────────────────────┘
```

Directional only — exact bash quoting, awk-script grammar, and mktemp-vs-fixed paths resolved at execution time.

## Implementation Units

### U1. Pre-flight stale-relic cleanup block

**Goal:** Detect non-canonical worktree paths for the target branch and clean them up with stash-before-remove discipline.

**Requirements:** R1, R2, R3, R4.

**Dependencies:** none.

**Files:**
- `mika/skills/bundled/_shared/dispatch-lib.sh` — modify `_set_up_worktree` function.

**Approach:**
- Insert a new block at the top of `_set_up_worktree` (before the existing dashed-path collision check).
- Detection via `git -C "$SUB_REPO_DIR" worktree list --porcelain | awk -v b="refs/heads/$BRANCH" '/^worktree / {wt = substr($0, 10)} $0 == "branch " b {print wt; exit}'`.
- Stash discipline: `git -C "$existing_wt" status --porcelain` → if non-empty, `git -C "$existing_wt" stash push -u -m "$stash_name"` with name `dispatch-lib-stale-worktree-cleanup-$(echo "$BRANCH" | tr / -)-$(date +%Y%m%dT%H%M%S)`.
- Log: `echo "[dispatch-lib] stashed dirty state from $existing_wt as: $stash_name (recover with: git -C $SUB_REPO_DIR stash list | grep $stash_name)" >&2`.
- Remove: `git -C "$SUB_REPO_DIR" worktree remove --force "$existing_wt" 2>&1 | tail -1 >&2 || true`.

**Patterns to follow:**
- `_clean_worktree_for_rebase` in `dispatch-lib.sh` (introduced by mika#1414) — same stash-naming convention, same stderr log shape.

**Test scenarios** (in `test-dispatch-lib.sh` via structural AST):
- Pre-flight block contains `worktree list --porcelain`.
- Pre-flight block contains the branch-comparison awk script (`refs/heads/$BRANCH` pattern).
- Pre-flight block contains the `[ -d "$existing_wt" ]` directory-exists guard.
- Pre-flight block contains `status --porcelain` check.
- Pre-flight block contains `stash push -u -m` with the descriptive-name pattern.
- Pre-flight block contains `worktree remove --force "$existing_wt"`.
- Pre-flight block is positioned BEFORE the existing dashed-path collision check (call ordering assertion).

**Verification:** `bash skills/bundled/_shared/test-dispatch-lib.sh` passes the new assertions; the pre-flight block can be extracted via sed range markers.

### U2. Wrapped `worktree add` attempts with dual-failure diagnostic

**Goal:** Replace the bare `worktree add ... || worktree add ...` chain with a stderr-captured version that emits a structured `worktree_setup_failed:` diagnostic on dual-failure.

**Requirements:** R5, R6, R7.

**Dependencies:** U1 (so pre-flight has already done its work and reduced the dual-failure surface).

**Files:**
- `mika/skills/bundled/_shared/dispatch-lib.sh` — modify the `worktree add` chain inside `_set_up_worktree`.

**Approach:**
- Wrap each `worktree add` with `2>/tmp/wt-add-<n>-err.<pid>` capture (use `$$` suffix to avoid concurrent-spawn collision).
- On dual-failure, emit:
  ```
  [dispatch-lib] worktree_setup_failed: branch=$BRANCH path=$WORKTREE_DIR
    attempt 1 (with -b): <stderr of attempt 1>
    attempt 2 (without -b): <stderr of attempt 2>
  ```
- Clean up temp files in both the success and dual-failure paths (no orphans in /tmp).
- The trap can still fire after `return 1`, but now with context in the log.

**Patterns to follow:**
- mika#1364's PR introduced `Push: FAILED` in the result field — same shape (structured prefix, actionable content).

**Test scenarios:**
- Wrapped block contains `2>/tmp/wt-add-1-err` and `2>/tmp/wt-add-2-err` stderr-capture redirects.
- Wrapped block contains the literal `worktree_setup_failed:` prefix on stderr.
- Wrapped block contains both `cat /tmp/wt-add-1-err` and `cat /tmp/wt-add-2-err` (the attempts' stderr in the diagnostic).
- Wrapped block has `rm -f /tmp/wt-add-*-err` in both success and dual-failure paths.
- Call ordering: U1's pre-flight runs BEFORE the wrapped `worktree add` block.

**Verification:** `bash skills/bundled/_shared/test-dispatch-lib.sh` passes the new assertions.

### U3. Doc-comment update on `_set_up_worktree`

**Goal:** Future maintainers reading the function understand the pre-flight cleanup rationale + sibling-ticket history.

**Requirements:** documentation hygiene.

**Dependencies:** U1, U2.

**Files:**
- `mika/skills/bundled/_shared/dispatch-lib.sh` — extend the doc comment above `_set_up_worktree`.

**Approach:**
- Add one paragraph citing mika#1472 + the sibling closed dispatch-lib silent-failure tickets (#1364, #1407, #1414, #1415).
- Note that the stash discipline mirrors mika#1414's `_clean_worktree_for_rebase`.

**Test expectation: none — pure doc comment change.**

**Verification:** comment present and grammatically clean; covered by review-time read.

## Risks & Dependencies

- **Risk: `git stash push -u` fails on a worktree with no commits yet.** If the stale worktree's branch is at an initial-commit-only state with untracked files, stash might surface an edge case. **Mitigation:** the `|| true` on the stash command keeps the cleanup proceeding even on stash failure — the dirty state is at worst left in the worktree, which `worktree remove --force` then destroys. Logged via the stderr log line. Operator can recover via reflog if needed.
- **Risk: awk script's `substr($0, 10)` assumes exactly 9 characters of `"worktree "` prefix.** This is part of the stable `worktree list --porcelain` format (documented in `git-worktree(1)`). Unlikely to drift.
- **Dependency: existing 19-test suite in `test-dispatch-lib.sh` is structural-AST-based.** Verified during grooming (Phase 0 Pin); new tests follow the same pattern. If the test infrastructure is refactored to real-git-ops in a future ticket, the new tests will need to be updated alongside.

## Open Questions (deferred to execution)

- Exact stash naming convention: `dispatch-lib-stale-worktree-cleanup-<branch-sanitized>-<ts>` vs a shorter form. Plan adopts the verbose form for grep-ability; execution may tighten if it conflicts with stash-name length limits.
- Whether to also emit a structured log line on SUCCESSFUL pre-flight cleanup (visibility) or only on the diagnostic dual-failure path (noise reduction). Plan adopts log-on-action (visible), execution may trim.

## Sources & Research

- **Origin:** GitHub issue mika#1472 — `fix(dispatch-lib): worktree-add fails silently with exit 128 when branch is checked out in a conflicting path`. Hard evidence: mika.db task `d0228cd0`, result field.
- **Pinned source:** `mika/skills/bundled/_shared/dispatch-lib.sh` — `_set_up_worktree` function.
- **Pinned test infrastructure:** `mika/skills/bundled/_shared/test-dispatch-lib.sh` — 19 structural-AST tests, no real git ops.
- **Sibling pattern:** `_clean_worktree_for_rebase` in `dispatch-lib.sh` (mika#1414) — stash-before-remove discipline.
- **Sibling closed tickets:** mika#1364, #1407, #1414, #1415 (all CLOSED).
- **Detection complement:** mika-platform#153 (probe_branch_divergence — surfaces this class faster).
- **Grooming history:**
  - First-pass session: `5c136813-4316-4bf2-b6cc-78309876f5e0`
  - Initial brief: ITERATE (F1 stash discipline, F2 test infrastructure pinning)
  - Revised brief with Phase 0 Pin: GROOMED in one revision (architect skipped second-pass)
