---
title: "Centralize cross-file invariants in shared code (companion slice)"
date: 2026-04-28
problem_type: workflow_issue
category: workflow-issues
module: dev-pilot, slash-commands
component: dev-pilot-handler, mika-slash-command
tags:
  - centralization
  - drift-class
  - dev-pilot
  - slash-commands
  - cross-repo
  - exit-code-contract
applies_when: >
  Editing the dev-pilot handler or this repo's `/mika` slash command in a way
  that touches branch-name or worktree-path derivation. Defer to the canonical
  mika-platform scripts.
---

# Centralize cross-file invariants in shared code (companion slice)

## Context

mika-platform issue #58 surfaced after the mika#844 dispatch failure on
2026-04-28 (`exit 128, fatal: branch already checked out at $OTHER_PATH`).
Branch-slug derivation was inlined in three independent sites across the
workspace — including this repo's dev-pilot handler (`cut -c1-40`) and
`.claude/commands/mika.md` (`cut -c1-45`). The implicit cross-file invariant
`worktree_path_slug == sanitize(branch_ref)` was nowhere named, only honored
by parallel manual updates — and drift broke a real dispatch.

The full pattern, rationale, and alternatives (six learnings — single source
of truth, CLI agent-readiness, post-refactor dangling references, plan/code
drift handling, `git -C` relative-path bug, SCRIPTS_DIR resolution asymmetry)
are documented in the canonical compound at:

- `mika-platform/docs/solutions/cross-repo-patterns/centralized-derivation-load-bearing-invariant-2026-04-28.md`

This file captures the mika-specific slice so this repo's pipeline gate has
a local compound doc to verify against, and so a contributor reading mika's
solutions store sees the centralization edict and the related correctness
findings.

## Guidance

### Dev-pilot handler

The dev-pilot handler at `skills/bundled/dev-pilot/handlers/run.sh` is the
runtime authority for autonomous dispatch — it creates real branches that
get pushed to GitHub. It **must** invoke the canonical scripts at
`$PLATFORM_DIR/scripts/`:

```sh
LABELS=$(printf '%s' "$ISSUE_JSON" | jq -r '[.labels[].name] | join(",")' 2>/dev/null)

BRANCH=$("$PLATFORM_DIR/scripts/derive-branch-name" \
    --title "$ISSUE_TITLE" \
    --issue "$ISSUE_NUM" \
    --labels "$LABELS" \
    --body-callout "$ISSUE_BODY")

WORKTREE_DIR=$("$PLATFORM_DIR/scripts/derive-worktree-path" --branch "$BRANCH" --repo "$REPO")
```

Note the `LABELS` format: the script accepts CSV via `--labels`, so the
handler joins the JSON array (whereas the original inline code iterated
newline-separated names).

### Slash command

This repo's `.claude/commands/mika.md` resolves `SCRIPTS_DIR` via walk-up
from `$PWD` (not via `git rev-parse --git-common-dir`, which from inside a
sub-repo returns the sub-repo's `.git`, not the meta-repo's):

```bash
SCRIPTS_DIR=""
d="$(pwd)"
while [ "$d" != "/" ]; do
  if [ -x "$d/scripts/derive-branch-name" ]; then SCRIPTS_DIR="$d/scripts"; break; fi
  d=$(dirname "$d")
done
[ -z "$SCRIPTS_DIR" ] && { echo "Error: could not locate mika-platform scripts/" >&2; exit 1; }
```

Walk-up works for both `<meta>/mika/` (main checkout) and
`<meta>/.claude/worktrees/<slug>/mika/` (worktree).

### Two correctness lessons surfaced by CE review on this PR

1. **Post-refactor dangling references.** Collapsing the inline derivation
   block (lines 218-248) into a subprocess call left `${SANITIZED}` dangling
   at line 310 (dry-run cleanup). Under `set -e` (no `-u`), the variable
   expanded to empty and `rmdir` silently targeted the worktrees ROOT.
   Fixed by using `derive-worktree-path --no-repo` for the parent dir.
   General rule: after a collapse, grep every variable defined in the
   removed block and audit each surviving reference. Adding `set -u` to
   the handler would catch this class at runtime.

2. **CLI exit-code contract.** Both `derive-*` scripts originally had a
   `shift 2` silent-exit-1 bug under `set -e` when a value-consuming flag
   appeared as the last argv entry. Outside the documented {0, 2, 3}
   contract that this handler parses for HANDLER CRASH reports. Fixed
   upstream in mika-platform with a `require_value` guard and regression
   tests; relevant here because the handler relies on the contract.

## Why This Matters

Reinstating inline derivation here — even temporarily — recreates the drift
class the centralization eliminated. The invariant is structurally enforced
only when every caller invokes the same shared computation; one inline
recovery defeats the property. The dev-pilot handler is the highest-risk
site because it creates real branches; protecting that surface is what made
the centralization load-bearing.

## When to Apply

- Editing `skills/bundled/dev-pilot/handlers/run.sh` to add new dispatch
  paths or change branch/worktree resolution.
- Editing `.claude/commands/mika.md` to add new slash-command entry points.
- Adding any future skill or handler that needs branch derivation or
  worktree-path computation — invoke the scripts via subprocess, do not
  reimplement.
- Reviewing PRs that touch slug derivation: confirm they invoke the
  canonical scripts rather than reintroducing inline recipes.

## Sources & References

- **Canonical compound (full pattern):** `mika-platform/docs/solutions/cross-repo-patterns/centralized-derivation-load-bearing-invariant-2026-04-28.md`
- **Origin issue:** [senara-solutions/mika-platform#58](https://github.com/senara-solutions/mika-platform/issues/58)
- **Drift symptom:** [senara-solutions/mika#844](https://github.com/senara-solutions/mika/issues/844) — dispatch exit 128
- **Related closure-bound enforcement:** [senara-solutions/mika#841](https://github.com/senara-solutions/mika/issues/841) — positive-consent gate
- **Companion plan (this repo):** `docs/plans/2026-04-28-001-refactor-dev-pilot-derive-scripts-companion-plan.md`
- **Plan-on-branch contract:** `docs/solutions/best-practices/plan-on-branch-load-bearing-contract-2026-04-26.md`
