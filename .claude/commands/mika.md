---
name: mika
description: Mika development workflow with quality gates and documentation audit
argument-hint: "[feature description]"
disable-model-invocation: true
---

<!-- SCOPE: mika core repo ONLY. Do NOT copy this to the meta-repo or other sub-repos. -->

Run these steps in order. Do not do anything else. Do not stop between steps — complete every step through to the end.

**Issue linking:** If `$ARGUMENTS` (after stripping any `branch:` prefix) starts with `#` followed by a number (e.g. `#42`) or is just a number, treat it as a GitHub issue reference. Run `gh issue view <number> --json number,title,body,labels` to fetch the issue details, then use the issue title and body as the feature description for the planning step. Remember the issue number for the PR step.

## Worktree isolation

Before running the pipeline, set up an isolated worktree:

1. **Parse branch:** If `$ARGUMENTS` starts with `branch:<name>`, extract `<name>` as the branch name and strip the `branch:<name>` prefix from `$ARGUMENTS`. Otherwise, derive the branch name from args (issue → `feat|fix|chore/<number>/<kebab-title>`, free-text → `feat/<kebab>`).
2. **Skip if no branch or no args:** If there are no arguments (backlog eval mode), skip worktree creation and run the pipeline in the current directory.
3. **Detect existing worktree:** Run `git rev-parse --git-dir` and `git rev-parse --git-common-dir`. If they differ, you are already in a worktree — skip creation, set `CREATED_WORKTREE=false`.
4. **Create worktree:** Set `WORKTREE=../.claude/worktrees/<sanitized-branch>/mika/` (sanitize branch name: replace `/` with `-`). Record `ORIGINAL_DIR=$(pwd)`.
   - If the worktree path already exists, remove it first: `git worktree remove --force <WORKTREE>` (ignore errors).
   - Try: `git worktree add -b <branch> <WORKTREE> main`
   - If that fails (branch already exists): `git worktree add <WORKTREE> <branch>`
   - cd into the worktree. Set `CREATED_WORKTREE=true`.

## Pipeline

1. `/ralph-loop "finish all slash commands" --completion-promise "DONE"`
2. `/ce:plan $ARGUMENTS` (if an issue was detected, pass the issue title + body instead of raw arguments)
3. `/ce:work`
4. `/ce:review`
5. `/compound-engineering:resolve_todo_parallel`
6. `/mika-doc-audit`
7. `/ce:compound`
8. Create a PR if one doesn't already exist. If a GitHub issue was referenced, include `Closes #<number>` in the PR body.

## Cleanup

9. If `CREATED_WORKTREE=true`: cd back to `ORIGINAL_DIR`, then run `git worktree remove --force <WORKTREE>`.
10. Output `<promise>DONE</promise>` when complete

Start with worktree isolation, then step 1.
