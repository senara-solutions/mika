---
title: "refactor: Switch dev-pilot + /mika to mika-platform derive-* scripts (companion)"
type: refactor
status: active
date: 2026-04-28
origin: https://github.com/senara-solutions/mika-platform/issues/58
---

# refactor: Switch dev-pilot + /mika to mika-platform derive-* scripts (companion)

## Overview

Companion to senara-solutions/mika-platform#58. The canonical plan lives in
the meta-repo at
`mika-platform/docs/plans/2026-04-28-001-refactor-centralize-branch-worktree-derivation-plan.md`.

This document captures the mika-side slice so this repo's pipeline-artifact
gate has a plan doc to verify against.

## Problem Frame

Branch-slug derivation drifted across multiple sites in this workspace; the
mika#844 dispatch failure on 2026-04-28 (`exit 128, branch already checked
out at $OTHER_PATH`) was the visible symptom — the dispatcher's `cut -c1-40`
and the slash command's `cut -c1-45` were producing different slugs for the
same input. The mika-platform PR introduces two centralized scripts and this
PR switches the runtime authority (the dev-pilot handler) over to them.

## Scope (this repo)

- `skills/bundled/dev-pilot/handlers/run.sh` — replace inline derivation block
  (originally lines 218-248) with subprocess calls to
  `$PLATFORM_DIR/scripts/derive-branch-name` and
  `$PLATFORM_DIR/scripts/derive-worktree-path`. Format `LABELS` as CSV via
  `jq -r '[.labels[].name] | join(",")'` to match the script's `--labels`
  API. Fix dangling `${SANITIZED}` reference in dry-run cleanup at line 310
  (P0 from CE review on the meta-repo PR) by switching to
  `derive-worktree-path --no-repo` for the parent dir.
- `.claude/commands/mika.md` — replace inline 45-char branch-slug recipe
  with a walk-up SCRIPTS_DIR resolution and subprocess calls to the canonical
  scripts. Walk-up (vs. the meta-repo's `dirname (git rev-parse
  --git-common-dir)` idiom) is required because `git-common-dir` from inside
  this sub-repo's checkout returns the sub-repo's `.git`, not the meta-repo's.

## Out of scope

- The new scripts themselves (live in mika-platform).
- Sub-repo `/mika` slash command edits in mika-cloud and mika-skills (their
  own companion PRs).
- The `--type-override` flag-removal cleanup, naming consistency between
  `mika-platform-*` scripts and the new `derive-*` scripts, and the
  `run_case` test-helper deduplication — all flagged by CE review as P3
  follow-ups, deferred.

## Verification

- `sh -n` syntax check on edited handler PASSES (verified locally).
- `bash` smoke test from dev-pilot context: cross-script invariant
  `WORKTREE_DIR == $PLATFORM_DIR/.claude/worktrees/sanitize($BRANCH)/$REPO`
  holds for a representative ticket title.
- End-to-end dispatch verification deferred to post-merge: requires
  mika-platform PR #59 to merge first so the scripts land in the operator's
  `~/workspace/mika-platform/scripts/` directory.

## Sources & References

- **Origin issue:** [senara-solutions/mika-platform#58](https://github.com/senara-solutions/mika-platform/issues/58)
- **Canonical plan:** `mika-platform/docs/plans/2026-04-28-001-refactor-centralize-branch-worktree-derivation-plan.md`
- **Companion meta-repo PR:** senara-solutions/mika-platform#59
- **Drift symptom:** [senara-solutions/mika#844](https://github.com/senara-solutions/mika/issues/844)
- **Compounded learning:** `mika-platform/docs/solutions/cross-repo-patterns/centralized-derivation-load-bearing-invariant-2026-04-28.md`
