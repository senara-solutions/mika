---
name: mika
description: Mika core development workflow with quality gates
argument-hint: "[feature description]"
disable-model-invocation: true
---

<!-- SCOPE: mika repo ONLY. Do NOT copy this to the meta-repo or other sub-repos. -->

Run these steps in order. Do not do anything else. Do not stop between steps — complete every step through to the end.

**Issue linking:** If `$ARGUMENTS` (after stripping any `branch:` prefix) starts with `#` followed by a number (e.g. `#214`) or is just a number, treat it as a GitHub issue reference. Run `gh issue view <number> --repo senara-solutions/mika --json number,title,body,labels` to fetch the issue details, then use the issue title and body as the feature description for the planning step. Remember the issue number for the PR step.

## Worktree isolation

Before running the pipeline, set up an isolated worktree:

1. **Parse branch:** Determine the branch name using this priority order (first match wins):
   a. **Explicit `branch:` prefix:** If `$ARGUMENTS` starts with `branch:<name>`, extract `<name>` and strip the prefix.
   b. **Issue body callout:** If an issue was fetched above, search the issue body for a line matching `> - **Branch:**` followed by a backtick-wrapped branch name (e.g., `` `feat/214/agent-health-endpoint` ``). Extract that branch name. This is how pre-planned tickets communicate their branch — **always use it when present**.
   c. **Derive via canonical script:** Only if (a) and (b) both miss, invoke the canonical script `mika-platform/scripts/derive-branch-name`. The script enforces priority order (body callout → conventional-commit prefix → label override → default `feat`) and the canonical 40-char truncation. See senara-solutions/mika-platform#58 for context on the drift class this eliminates.
      ```bash
      # Walk up from $PWD to locate mika-platform/scripts/ — works from
      # both <meta>/mika/ (main checkout) and <meta>/.claude/worktrees/<slug>/mika/.
      SCRIPTS_DIR=""
      d="$(pwd)"
      while [ "$d" != "/" ]; do
        if [ -x "$d/scripts/derive-branch-name" ]; then SCRIPTS_DIR="$d/scripts"; break; fi
        d=$(dirname "$d")
      done
      [ -z "$SCRIPTS_DIR" ] && { echo "Error: could not locate mika-platform scripts/" >&2; exit 1; }

      BRANCH=$("$SCRIPTS_DIR/derive-branch-name" \
        --title "${ISSUE_TITLE:-$ARGUMENTS}" \
        --issue "${ISSUE_NUMBER:-}" \
        --labels "${LABELS:-}" \
        --body-callout "${ISSUE_BODY:-}")
      ```
2. **Skip if no branch or no args:** If there are no arguments, skip worktree creation and run the pipeline in the current directory.
3. **Detect existing worktree (MANDATORY):** Run `git rev-parse --git-dir` and `git rev-parse --git-common-dir`. If they differ, you are ALREADY inside a worktree. **STOP worktree setup immediately** — set `CREATED_WORKTREE=false` and proceed directly to the Pipeline section below. Do NOT attempt to create, remove, or modify any worktree. Do NOT clean up or recreate. Just use the current directory as-is.
4. **Sync main:** Run `git fetch origin main:main` to fast-forward local `main` to match remote. This ensures the worktree branches from the latest code. If it fails (e.g., `main` is checked out with uncommitted changes), fall back to `git fetch origin` and use `origin/main` as the base ref in the next step.
5. **Create worktree:** Compute the worktree path via `derive-worktree-path` (enforces the invariant `slug == sanitize(branch_ref)`). Record `ORIGINAL_DIR=$(pwd)`.
   ```bash
   WORKTREE=$("$SCRIPTS_DIR/derive-worktree-path" --branch "$BRANCH" --repo mika)
   ```
   - If the worktree path already exists, remove it first: `git worktree remove --force "$WORKTREE"` (ignore errors).
   - Try: `git worktree add -b "$BRANCH" "$WORKTREE" main`
   - If that fails (branch already exists): `git worktree add "$WORKTREE" "$BRANCH"`
   - cd into the worktree. Set `CREATED_WORKTREE=true`.

## Pipeline

**Branch safety (MANDATORY):** You are already on the correct branch. Run `git branch --show-current` to confirm. Do NOT create, rename, or switch branches. All commits and the PR must use the current branch. This applies to every step below — including `/ce:plan`, `/ce:work`, `/ce:review`, and PR creation.

1. `/ce:plan $ARGUMENTS` (if an issue was detected, pass the issue title + body instead of raw arguments)
2. **Ensure `## Acceptance criteria` section exists in the plan.** `/ce:plan` does not emit this section — it must be added after plan generation. Rules:
   - If the referenced issue body has an `## Acceptance criteria` section, transcribe its criteria verbatim into a new `## Acceptance criteria` section in the plan (placed after `## Definition of Done`).
   - If the issue body has no `## Acceptance criteria` section (or no issue was referenced), derive concrete, testable acceptance criteria from the plan's Requirements and Verification Contract sections.
   - Do NOT rename `## Definition of Done` to `## Acceptance criteria`. Both sections must coexist.
   - The section must contain at least one markdown checkbox item (`- [ ] <criterion>`).
3. `/ce:work`
4. `/ce:review`
5. `/compound-engineering:resolve_todo_parallel`
6. `/ce:compound`
7. Run `bash scripts/verify-pipeline.sh` to verify pipeline artifacts exist. If it fails, read the error messages to identify missing artifacts, go back and produce them (run `/ce:plan` if no plan doc, `/ce:work` if no source changes), then re-run verification until it passes.
8. Create a PR if one doesn't already exist:
   ```
   gh pr create --repo senara-solutions/mika --title "<title>" --body "<body>"
   ```
   If a GitHub issue was referenced, include `Closes #<number>` in the PR body.

## Cleanup

9. Do NOT remove the worktree. Worktrees persist until the PR is merged — needed for CI fixes, review feedback, and acceptance testing. Cleanup happens post-merge.
10. Output `<promise>DONE</promise>` when complete.

Start with worktree isolation, then step 1.
