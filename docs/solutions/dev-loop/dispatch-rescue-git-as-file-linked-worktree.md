---
module: mika-skills
tags: [dispatch-lib, auto-rescue, linked-worktree, git-worktree, mktemp, enotdir, autonomous-loop, mika-1341, mika-1296]
problem_type: runtime_error
category: dev-loop
date: 2026-05-30
---

# Dispatch auto-rescue silently fails in linked worktrees (`.git`-as-file scratch path)

## Problem

The autonomous loop's dirty-worktree auto-rescue (`skills/bundled/_shared/dispatch-lib.sh`, mika#1282/#1296/#1310) silently failed to commit in **every linked git worktree** — i.e. every autonomous `dev-pilot` run, which always operates in `.claude/worktrees/<branch>/<repo>`. The pilot wrote the correct edit, but the rescue commit never landed: no branch pushed, no PR. The loop could not land any implementation.

## Symptoms

A groomed ticket dispatched through the loop produced:

```
PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook (non-rustfmt).
Hook output: <rescue capture was empty — likely no hook output, falling back to git diagnostic>
git status:
 M Dockerfile.agent          <- the edit WAS applied
 ?? .claude/commands/...      <- injected scaffold (red herring)
```

Fingerprint: **"non-rustfmt" rejection + empty hook capture + HEAD unchanged + edit left uncommitted.**

## Root cause

`dispatch-lib.sh` wrote its commit-output capture file to:

```sh
RESCUE_COMMIT_ERR="$WORKTREE_DIR/.git/mika-rescue-commit-err"
```

**In a linked git worktree, `.git` is a FILE — a `gitdir:` pointer — not a directory.** (Only the main checkout has a `.git` directory.) So the redirect on the rescue commit:

```sh
git -C "$WORKTREE_DIR" commit -m "..." > "$RESCUE_COMMIT_ERR" 2>&1
```

could not **open** its output target (`ENOTDIR: not a directory`). Per POSIX shell semantics, **a failed output redirection means the command is never executed and exits non-zero, with no output written.** That single failure cascaded through the rescue block's error-classification logic:

1. `git commit` never runs → HEAD unchanged → edit uncommitted → no PR.
2. `if git commit ...` exits non-zero → falls to `elif grep -q "rustfmt" "$RESCUE_COMMIT_ERR"` → the capture file was never created → no match → falls into the **"non-rustfmt"** `else` branch.
3. `cat "$RESCUE_COMMIT_ERR"` → file absent → empty → triggers the mika#1310 **"rescue capture was empty"** git-status diagnostic fallback.

Every observed symptom traced to this one line.

## What didn't work (ruled out with evidence)

- **Injected `.claude/commands/*.md` pollution** (the original hypothesis): already excluded from the changeset by mika#1288's `git add -A -- ':!.claude/commands/'`. The files appear only in the *diagnostic dump*, which made them *look* causal. Red herring.
- **`lefthook` missing from PATH**: `.git/hooks/pre-commit` falls through to `echo "Can't find lefthook in PATH"` and returns **0** (non-fatal). With only `Dockerfile.agent` staged, no `lefthook.yml` job glob even matches. The hook was never the rejecter.
- **Mis-scoped ticket**: filed as `claude-pilot-py#24`, but the entire fix surface was in `mika`. Triaging by the failure string — the `PIPELINE FAILURE` text originates in `mika/skills/bundled/_shared/dispatch-lib.sh` — located the real repo.

## Solution

One-line change — write the scratch file to a guaranteed-valid path:

```sh
# Before (ENOTDIR in linked worktrees):
RESCUE_COMMIT_ERR="$WORKTREE_DIR/.git/mika-rescue-commit-err"

# After:
RESCUE_COMMIT_ERR="$(mktemp "${TMPDIR:-/tmp}/mika-rescue-commit-err.XXXXXX")"
```

`mktemp` preserves the original mika#1296 intent (keep the scratch off the working tree, away from `.iterate/`) while guaranteeing a real, writable path in both linked and non-linked checkouts. The named template also preserves the literal token `mika-rescue-commit-err`, which several tests use as a `sed` anchor to extract the rescue block.

The existing `rm -f "$RESCUE_COMMIT_ERR"` cleanup in each terminal branch is unchanged (a `mktemp` file is a normal file).

## Why this works

The alternative `mktemp` family resolves a path in `$TMPDIR`/`/tmp` — always a real directory — so the redirect always opens and `git commit` actually runs. The alternative considered (`"$(git -C "$WORKTREE_DIR" rev-parse --git-dir)/..."`, which resolves to the real per-worktree git dir `<repo>/.git/worktrees/<name>`) also works but writes into git internals and adds a subprocess; `mktemp` is simpler and strictly safer.

Edge case: if `mktemp` itself fails (empty result), the subsequent redirect fails the same way — but only when `/tmp` is itself broken, versus the old code's **100% failure in every linked worktree**. Strictly better; no guard needed.

## Prevention

1. **`.git` is a file in linked worktrees.** Any code that builds paths under `"$WORKTREE_DIR/.git/"` (scratch files, hook outputs, config) works in the main checkout but breaks (`ENOTDIR`) in a worktree. The autonomous loop **always** uses linked worktrees, so `.git`-as-directory assumptions are latent landmines. Use `mktemp`, or `git rev-parse --git-dir` when a git-local path is genuinely required.
2. **Failed redirects fail silently.** `cmd > badpath` exits non-zero *without running `cmd`* and *without output*. Downstream logic that greps/cats the (never-created) capture file then misclassifies the failure. Watch for the "empty capture + unexpected error class" fingerprint.
3. **Don't trust reimplementation-mirror tests.** This bug shipped because the existing test (`test_rescue_hook_failure_invariant`) reimplemented the rescue logic inline against a `mktemp -d` **non-linked** repo (where `.git` is a directory), so it never exercised the failing environment. The regression test added here has two parts: **Test C** is *source-coupled* — it greps the real `RESCUE_COMMIT_ERR` assignment and asserts it uses `mktemp`, not `.git/` (fails on the buggy code, passes on the fix); **Test D** builds a real linked worktree and proves the `.git`-is-a-file / ENOTDIR mechanics. Prefer source-coupled or real-environment tests over reimplemented mirrors that can diverge from the production environment.

## Related

- mika#1296 (`0eababa7`, 2026-05-26) — introduced the `.git/` scratch path. The intent (keep scratch off the working tree) was right; the location was invalid in worktrees.
- mika#1310 — the combined stdout+stderr capture this scratch file feeds.
- mika#1282 — the dirty-worktree rescue mechanism itself.
- mika#1288 — the `.claude/commands` pathspec exclusion (the red herring in this bug).
- mika#1327 — the upstream groom brake that wedged the loop until 2026-05-29, keeping this downstream bug latent for 3 days.
- `docs/solutions/workflow-issues/failed-pilot-worktree-contamination-signature-2026-05-18.md` — sibling worktree-hygiene signature.
- **Deploy note:** `dispatch-lib.sh` is copy-deployed (seeded to `~/.mika`, not compiled into the binary). Main-merged ≠ live until `make deploy` re-seeds bundled skills. Diff the `~/.mika` copy against source before declaring the loop unwedged.
- Plan: `docs/plans/2026-05-30-003-fix-1341-dispatch-rescue-git-as-file-scratch-plan.md`.
