---
title: "ci: sync crate-local docs/slash-commands.md to unblock PR #1068"
type: fix
status: active
date: 2026-05-12
---

# ci: sync crate-local docs/slash-commands.md to unblock PR #1068

## Overview

PR #1068 (mika#1066 TUI fix) is APPROVED by mika-qa but blocked on the `Docs Sync` CI check. The implementation updated `docs/slash-commands.md` (the canonical source) but did not sync the change to `crates/mika-agent/docs/slash-commands.md` (the crate-local copy required for `cargo publish`). This plan addresses the one-line drift so CI passes and the PR auto-merges.

## Problem Frame

The `Docs Sync` CI job (`ci.yml` § `docs-sync`) runs `scripts/sync-agent-docs.sh` then checks `git diff --exit-code crates/mika-agent/docs/`. The PR modified the `/quit` command description in the canonical doc but did not propagate via the sync script. Multiple re-dispatches of mika#1066 returned "DONE" because claude-pilot scoped the impl as complete — the doc-sync gap is downstream of impl.

## Requirements Trace

- R1. `crates/mika-agent/docs/slash-commands.md` matches `docs/slash-commands.md` byte-for-byte on the PR #1068 branch
- R2. `Docs Sync` CI check passes on PR #1068
- R3. PR #1068 auto-merges (already APPROVED by mika-qa)

## Scope Boundaries

- This fix targets only `slash-commands.md` — no other docs are out of sync on this branch
- The fix lands on the **existing PR #1068 branch** (`fix/1066/tui-silently-drops-enter-while-busy-exit`), not on a new branch
- The existing worktree is at `/data/workspace/mika-platform/.claude/worktrees/fix-1066-tui-silently-drops-enter-while-busy-exit/mika`

## Context & Research

### Relevant Code and Patterns

- `scripts/sync-agent-docs.sh` — canonical sync script that copies all docs from `docs/` to `crates/mika-agent/docs/`
- `.github/workflows/ci.yml` § `docs-sync` job — runs the sync script then checks for uncommitted changes
- The specific drift: line 116 of `docs/slash-commands.md` has an expanded `/quit` description that `crates/mika-agent/docs/slash-commands.md` lacks

## Key Technical Decisions

- **Use `scripts/sync-agent-docs.sh` rather than manual `cp`**: The sync script is the canonical mechanism and syncs all docs atomically. Using it ensures no other files have drifted on this branch.

## Implementation Units

- [ ] **Unit 1: Run sync script and commit on PR #1068 branch**

**Goal:** Bring crate-local docs into sync with canonical docs on the PR #1068 branch

**Requirements:** R1, R2, R3

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/docs/slash-commands.md` (sync from canonical)

**Approach:**
- Run `bash scripts/sync-agent-docs.sh` in the PR #1068 worktree to sync all crate-local docs
- Stage only `crates/mika-agent/docs/slash-commands.md` (the known drift)
- If the sync script changes additional files, stage those too — they represent previously-undetected drift
- Commit with message `docs: sync crate-local slash-commands.md after mika#1066 impl`
- Push to the PR #1068 branch

**Patterns to follow:**
- The sync script itself is the pattern — it is referenced in CI error output as the remediation step

**Test expectation:** none — CI's `docs-sync` job is the verification mechanism

**Verification:**
- `git diff --exit-code crates/mika-agent/docs/` produces no output after running the sync script
- `Docs Sync` CI check passes on PR #1068 after push
- PR #1068 auto-merges once all checks pass

## System-Wide Impact

- **Interaction graph:** None — this is a file sync, not a behavioral change
- **Error propagation:** N/A
- **State lifecycle risks:** None
- **API surface parity:** The crate-local docs exist for `cargo publish` parity; this fix restores that parity

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Worktree for PR #1068 may have been cleaned up | Verified it exists at the expected path |
| Other docs may have drifted on this branch | Using `sync-agent-docs.sh` (not manual `cp`) catches all drift atomically |

## Sources & References

- Related PRs/issues: mika#1066, mika PR#1068
- CI workflow: `.github/workflows/ci.yml` § `docs-sync`
- Sync script: `scripts/sync-agent-docs.sh`
