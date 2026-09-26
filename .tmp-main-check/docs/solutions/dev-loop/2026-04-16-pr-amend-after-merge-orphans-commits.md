---
title: Pushing to a feature branch after its PR already merged orphans the commits
date: 2026-04-16
category: dev-loop
status: applied
---

# Pushing to a feature branch after its PR already merged orphans the commits

## What happened

While working on PR #604 (migration of engine-coupled skills), three issues stacked:

1. First push: 3 skills. PR #604 opened against that commit.
2. PR #604 was merged (squash) as commit `68f1a64` on main. The feature branch lived on but was no longer backing an open PR.
3. I then discovered two more engine-coupled skills (`resolve-pr-conflicts`, `self-check`) and **pushed a new commit to the same feature branch** — thinking it would "amend" the PR.
4. GitHub doesn't reopen a merged PR for new commits. The commit became **orphaned**: present on the remote branch, not on main, not backing any PR.
5. In parallel, I deleted those two skills from `mika-skills/main` thinking they were migrated — but they weren't on mika's main. Result: both skills briefly existed nowhere in production.

## The recovery

`git cherry-pick <orphaned-sha>` onto a fresh branch from main, then open a new PR. Cheap recovery, but the root cause was the assumption that pushing to a feature branch always "updates its PR."

## The lesson

**A feature branch is not the same thing as a PR.** GitHub's PR UI binds a PR to a branch at creation, but the binding is one-directional: the PR tracks the branch, the branch doesn't know about the PR. Once a PR is merged/closed, new commits on the branch are orphaned — they don't automatically open a new PR, and they don't show up on main.

Before pushing an "amendment" to a feature branch, verify the PR is still `OPEN`. If it's `MERGED` or `CLOSED`, the correct move is always: **new branch from current main, open a new PR**.

## Procedural change

For future migrations or follow-ups that expand scope mid-PR:
- If PR is still `OPEN`: push the new commit, optionally retitle/re-body the PR to reflect expanded scope.
- If PR is `MERGED`: **always start a fresh branch from current main**. Do not push to the old feature branch. Cherry-pick orphaned work only if you already made the mistake.

## The companion-cleanup corollary

If the mistake has already happened and you also deleted upstream dependencies thinking the migration landed, fix mika first (restore the code on main), then mika-skills catches up. Never delete the source until the destination is confirmed on production main.
