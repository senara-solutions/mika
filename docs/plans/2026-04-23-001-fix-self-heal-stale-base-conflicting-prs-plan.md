---
title: "fix: Self-heal stale-base + CONFLICTING PRs"
type: fix
status: active
date: 2026-04-23
---

# fix: Self-heal stale-base + CONFLICTING PRs

## Overview

Add a rebase-or-abort guard to the claude-pilot handler so branches pre-committed from a stale main are caught up before claude-pilot runs. Wire the existing `resolve-pr-conflicts` skill into self-dev so mika-dev can self-heal PRs that reach CONFLICTING state.

## Problem Frame

PR mika#746 sat in `mergeable=CONFLICTING` for 8+ hours because two structural gaps prevented self-healing:

1. **Stale-base at dispatch:** When `run.sh` falls through to `git worktree add <WORKTREE> <branch>` (branch already exists from plan pre-commit), it checks out the branch without rebasing onto `origin/main`. Sequential tickets dispatched after upstream merges inherit stale bases, causing avoidable conflicts.

2. **Capability not loaded:** The `resolve-pr-conflicts` bundled skill exists with a `resolve_pr_conflicts` tool, but self-dev doesn't declare it as a dependency (so the tool is never in mika-dev's inventory) and self-dev's prompt never checks mergeable state before merge attempts.

## Requirements Trace

- R1. `run.sh` must auto-rebase existing branches onto `origin/main` after worktree creation
- R2. On rebase conflict, `run.sh` must capture conflicted files BEFORE aborting and exit with structured `STATUS=REBASE_CONFLICT` result
- R3. `self-dev/skill.toml` must list `resolve-pr-conflicts` as a dependency
- R4. `self-dev/system_prompt.md` must route to `resolve_pr_conflicts` when a PR is CONFLICTING — single sentence, no routing-table duplication

## Scope Boundaries

- NOT fixing PR #746's current conflict (manual resolution by Vincent)
- NOT adding dashboard observability (tracked as #744)
- NOT adding a webhook for `mergeable` state transitions (GitHub doesn't emit one)

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/claude-pilot/handlers/run.sh` lines 233–247: fetch + worktree creation block
- `skills/bundled/claude-pilot/handlers/run.sh` lines 36–93: EXIT trap uses `$RESULT` variable — structured result must populate RESULT before `exit 1`
- `skills/bundled/self-dev/skill.toml` line 8–13: dependencies array
- `skills/bundled/self-dev/system_prompt.md`: Callback Entry Point section covers pre-merge handling
- `skills/bundled/resolve-pr-conflicts/system_prompt.md`: already documents routing decision table and tool contract

### Institutional Learnings

- `feedback_prompt_enforcement_fragile.md`: structural > prompt-level enforcement — fix 2 (dependency) and fix 3 (single-sentence routing) honor this principle

## Key Technical Decisions

- **Capture conflicts before abort:** `git diff --name-only --diff-filter=U` must run BEFORE `rebase --abort` because abort resets the index — the conflict markers are gone after abort
- **Structured discriminator:** Use `STATUS=REBASE_CONFLICT` as first line of RESULT so the EXIT trap's callback delivery sends it verbatim — orthogonal to mika-dev's callback-rendering logic (it can pattern-match the prefix)
- **Single-sentence delegation:** self-dev prompt gets one sentence pointing to `resolve_pr_conflicts` — the skill's own `system_prompt.md` already owns the routing table, input schema, and escalation criteria

## Open Questions

### Resolved During Planning

- **Where to insert rebase guard in run.sh?** After line 247 (the worktree add fallthrough block), before line 249 (config copy). At this point `origin/main` is already fresh (line 233 fetches it) and `$WORKTREE_DIR` is set.
- **Should both the "reuse existing worktree" and "create new worktree" paths get the guard?** Yes — both paths can result in a branch behind `origin/main`. The reuse path (line 240–241) checks out the branch without rebasing; the create-new fallthrough path (line 245) also checks out without rebasing.

### Deferred to Implementation

- Exact stderr message wording — implementation can tune the diagnostic text

## Implementation Units

- [ ] **Unit 1: Add rebase-or-abort guard to run.sh**

**Goal:** After worktree creation/reuse, detect if the branch is behind `origin/main` and auto-rebase. On conflict, capture file list and exit with structured result.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/claude-pilot/handlers/run.sh`

**Approach:**
- Insert the guard after the worktree creation block (after line 247) and also after the reuse path (line 241)
- Use `git rev-list --count HEAD..origin/main` to detect commits behind
- On `BEHIND > 0`, attempt `git rebase origin/main`
- On rebase success, log to stderr
- On rebase failure: capture `git diff --name-only --diff-filter=U` BEFORE calling `rebase --abort`, then populate `RESULT` with `STATUS=REBASE_CONFLICT` discriminator and `exit 1` (the EXIT trap delivers the callback)
- The guard runs inside the `repo#number` block only — free-text mode has no worktree

**Patterns to follow:**
- EXIT trap at lines 36–93 already handles `$RESULT` — populate it before `exit 1`
- Line 233 already fetches `origin main` — no additional fetch needed
- Stderr logging pattern: `echo "..." >&2` (matches existing handler style)

**Test scenarios:**
- Happy path: branch 3 commits behind origin/main, no conflicts → rebase succeeds, stderr shows "Rebased <branch> onto origin/main (3 commits caught up)"
- Happy path: branch already up-to-date (BEHIND=0) → guard is a no-op, proceeds normally
- Error path: branch behind origin/main with conflicting files → captures conflict file list, RESULT starts with `STATUS=REBASE_CONFLICT`, exit 1 triggers callback delivery
- Edge case: `git rev-list` fails (e.g., detached HEAD) → BEHIND defaults to 0, guard is a no-op

**Verification:**
- The rebase guard block exists after both worktree paths
- RESULT format starts with `STATUS=REBASE_CONFLICT` on conflict
- Conflict file list is captured before `rebase --abort`

- [ ] **Unit 2: Add resolve-pr-conflicts dependency to self-dev**

**Goal:** Make the `resolve_pr_conflicts` tool available in mika-dev's tool inventory when self-dev is active.

**Requirements:** R3

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/self-dev/skill.toml`

**Approach:**
- Add `"resolve-pr-conflicts"` to the `dependencies` array (line 8–13)
- The BFS dependency resolver loads it whenever self-dev activates

**Patterns to follow:**
- Existing dependency entries: `"build-mika"`, `"deploy-mika"`, `"claude-pilot"`, `"browser-control"`

**Test scenarios:**
- Happy path: self-dev activates → `resolve_pr_conflicts` tool appears in mika-dev's tool inventory (verified by `build.rs` bundling both skills and the dependency resolver loading transitively)

**Verification:**
- `skill.toml` dependencies array includes `"resolve-pr-conflicts"`

- [ ] **Unit 3: Add mergeable-check routing to self-dev prompt**

**Goal:** Insert a single sentence in self-dev's pre-merge path so mika-dev checks `mergeable` state and delegates to `resolve_pr_conflicts` when CONFLICTING.

**Requirements:** R4

**Dependencies:** Unit 2 (tool must be available)

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md`

**Approach:**
- Insert one sentence in the "On success" callback handling section (after metadata extraction, before merge/close-out) — this is the natural point where a PR is about to be acted on
- The sentence directs: check `mergeable` via `gh pr view`, invoke `resolve_pr_conflicts` if CONFLICTING
- No routing-table duplication — the `resolve-pr-conflicts` skill's own `system_prompt.md` documents the full routing table, input schema, and behavior

**Patterns to follow:**
- Existing delegation style in self-dev: single-sentence references to other skills (e.g., "Permission requests... intercepted automatically by the `permission-policy` skill")
- `resolve-pr-conflicts/system_prompt.md` already documents the `resolve_pr_conflicts` tool contract

**Test scenarios:**
- Happy path: self-dev prompt contains a sentence about checking `mergeable` state and invoking `resolve_pr_conflicts`
- Edge case: sentence does NOT duplicate the routing table from `resolve-pr-conflicts/system_prompt.md`

**Verification:**
- Exactly one sentence added referencing `mergeable` and `resolve_pr_conflicts`
- No routing-table duplication from the resolve-pr-conflicts skill

## System-Wide Impact

- **Interaction graph:** The rebase guard runs before claude-pilot launches — on conflict, the EXIT trap delivers the RESULT callback to mika-dev. mika-dev sees `STATUS=REBASE_CONFLICT` and can act (escalate or invoke `resolve_pr_conflicts`)
- **Error propagation:** Rebase conflicts exit via `exit 1` → EXIT trap → `deliver_callback` → mika callback. Standard path, no new failure modes
- **Unchanged invariants:** Free-text mode in run.sh is not affected. The resolve-pr-conflicts tool contract is unchanged. The EXIT trap's callback delivery mechanism is unchanged

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Rebase guard adds latency to every dispatch | `git rev-list --count` is fast (~ms); rebase only runs when behind |
| Rebase could introduce subtle merge artifacts | This is the same rebase any developer would do; conflicts are caught and reported |

## Sources & References

- Related issue: #747
- Related PR: #746 (motivating case — CONFLICTING state)
- Related issue: #744 (dashboard observability gap — separate concern)
- Related doc: `feedback_prompt_enforcement_fragile.md`
