---
name: mika
description: Mika development workflow with quality gates and documentation audit
argument-hint: "[feature description]"
disable-model-invocation: true
---

<!-- SCOPE: mika core repo ONLY. Do NOT copy this to the meta-repo or other sub-repos. -->

Run these steps in order. Do not do anything else. Do not stop between steps — complete every step through to the end.

**Issue linking:** If `$ARGUMENTS` (after stripping any `branch:` prefix) starts with `#` followed by a number (e.g. `#42`) or is just a number, treat it as a GitHub issue reference. Run `gh issue view <number> --json number,title,body,labels` to fetch the issue details, then use the issue title and body as the feature description for the planning step. Remember the issue number for the PR step.

**Plan-on-branch detection:** After fetching an issue body (or when a plan path is supplied directly in `$ARGUMENTS`), check whether a groomed plan-on-branch already exists. The grooming pipeline (`/mika-groom-ticket`) writes a callout into the issue body of this exact shape:

```
> - **Plan:** `docs/plans/<filename>.md` (committed on branch @ `<sha>`)
```

Parse the issue body for that callout. Extract the `<path>` (the value inside the first backtick-wrapped argument).

- **If the callout is present AND `<path>` exists in the worktree** (verify with `test -f <path>`): set `PLAN_PATH=<path>` and **skip Step 1 (`/ce:plan`)** in the Pipeline below. Run `/ce:work <PLAN_PATH>` directly (Step 2). Frame the prompt explicitly so claude-pilot consumes the plan as the contract:

  > This plan was groomed and committed by the architect. It is the contract for this implementation. If any acceptance criterion is unclear or unsatisfiable (e.g., conflicts with an existing parser, breaks a downstream consumer, depends on something that doesn't exist), **send_message** to mika-dev surfacing the ambiguity — do not silently scope-reduce. Do not write a new plan file in `docs/plans/`. The existing plan-on-branch is the single source of truth.

- **If the callout is absent OR `<path>` does not exist in the worktree:** fall back to the current flow — run `/ce:plan $ARGUMENTS` (Step 1) followed by `/ce:work` (Step 2).

This branch is also valid when `$ARGUMENTS` directly contains a `docs/plans/...md` path (e.g., `/mika implement the langfuse fix plan` where the operator names the path): treat it the same as a callout match.

## Worktree isolation

Before running the pipeline, set up an isolated worktree:

1. **Parse branch:** Determine the branch name using this priority order (first match wins):
   a. **Explicit `branch:` prefix:** If `$ARGUMENTS` starts with `branch:<name>`, extract `<name>` and strip the prefix.
   b. **Issue body callout:** If an issue was fetched above, search the issue body for a line matching `> - **Branch:**` followed by a backtick-wrapped branch name (e.g., `` `feat/687/domain-graph-builder` ``). Extract that branch name. This is how pre-planned tickets communicate their branch — **always use it when present**.
   c. **Derive from args (deterministic):** Only if (a) and (b) both miss, derive the branch name using this **exact bash recipe** — do not re-derive with the LLM or substitute your own kebab-casing, since redispatches must produce the same slug:
      ```bash
      # Inputs: $ISSUE_NUMBER (optional, set when an issue was fetched), raw title/text
      raw="${ISSUE_TITLE:-$ARGUMENTS}"
      if printf '%s' "$raw" | grep -qE '^(feat|fix|chore|docs|eval|test|refactor|perf)(\([^)]+\))?: '; then
        type=$(printf '%s' "$raw" | sed -nE 's/^([a-z]+)(\([^)]+\))?: .*$/\1/p')
        body=$(printf '%s' "$raw" | sed -E 's/^[a-z]+(\([^)]+\))?: *//')
      else
        type=feat
        body="$raw"
      fi
      slug=$(printf '%s' "$body" | tr '[:upper:]' '[:lower:]' \
        | LC_ALL=C sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' \
        | cut -c1-45 | sed -E 's/-[^-]*$//; s/-+$//')
      if [ -n "${ISSUE_NUMBER:-}" ]; then
        BRANCH="${type}/${ISSUE_NUMBER}/${slug}"
      else
        BRANCH="${type}/${slug}"
      fi
      ```
2. **Skip if no branch or no args:** If there are no arguments (backlog eval mode), skip worktree creation and run the pipeline in the current directory.
3. **Detect existing worktree (MANDATORY):** Run `git rev-parse --git-dir` and `git rev-parse --git-common-dir`. If they differ, you are ALREADY inside a worktree. **STOP worktree setup immediately** — set `CREATED_WORKTREE=false`, run `command -v lefthook >/dev/null 2>&1 && lefthook install` (non-blocking — skip silently if lefthook is not installed), and proceed directly to the Pipeline section below. Do NOT attempt to create, remove, or modify any worktree. Do NOT clean up or recreate. Just use the current directory as-is.
4. **Sync main:** Run `git fetch origin main:main` to fast-forward local `main` to match remote. This ensures the worktree branches from the latest code. If it fails (e.g., `main` is checked out with uncommitted changes), fall back to `git fetch origin` and use `origin/main` as the base ref in the next step.
5. **Create worktree:** Set `WORKTREE=../.claude/worktrees/<sanitized-branch>/mika/` (sanitize branch name: replace `/` with `-`). Record `ORIGINAL_DIR=$(pwd)`.
   - If the worktree path already exists, remove it first: `git worktree remove --force <WORKTREE>` (ignore errors).
   - Try: `git worktree add -b <branch> <WORKTREE> main`
   - If that fails (branch already exists): `git worktree add <WORKTREE> <branch>`
   - cd into the worktree. Set `CREATED_WORKTREE=true`.
6. **Install lefthook hooks:** Run `command -v lefthook >/dev/null 2>&1 && lefthook install` to ensure pre-commit hooks (fmt, clippy, secrets scan) are active in the worktree. Non-blocking — skip silently if lefthook is not installed.

## Pipeline

**Branch safety (MANDATORY):** You are already on the correct branch. Run `git branch --show-current` to confirm. Do NOT create, rename, or switch branches. All commits and the PR must use the current branch. This applies to every step below — including `/ce:plan`, `/ce:work`, `/ce:review`, and PR creation.

**Git discipline (MANDATORY):** Do NOT run `git pull`, `git pull origin main`, `git merge main`, `git merge origin/main`, or any similar catch-up operation during the pipeline. The Worktree isolation step above already rebased this branch onto `origin/main` via the handler's startup guard. If `origin/main` advances while the pipeline is running, the resulting conflict is resolved **post-pipeline** through the `resolve_pr_conflicts` skill — never by pulling main into the branch mid-session. Mid-session merges create duplicate-hash copies of upstream commits and produce a `mergeable=CONFLICTING` PR on GitHub even though the content is identical. Allowed git commands during the pipeline: `git add`, `git commit`, `git push` (including `--force-with-lease` for amended commits), `git status`, `git log`, `git diff`, `git fetch origin` (read-only). Everything else — especially `pull` and `merge` — is out of scope.

1. `/ce:plan $ARGUMENTS` (if an issue was detected, pass the issue title + body instead of raw arguments) — **skip when a plan-on-branch was detected above; jump straight to Step 2 with `/ce:work <PLAN_PATH>`**
2. `/ce:work` — when a plan-on-branch was detected, invoke as `/ce:work <PLAN_PATH>` with the contract framing from the Issue linking section above
3. `/ce:review`
4. `/compound-engineering:resolve_todo_parallel`
5. `/mika-doc-audit`
6. `/ce:compound`
7. Run `bash scripts/verify-pipeline.sh` to verify pipeline artifacts exist. If it fails, read the error messages to identify missing artifacts, go back and produce them (run `/ce:plan` if no plan doc, `/ce:work` if no source changes, `/ce:compound` if no compound doc), then re-run verification until it passes.
8. Create a PR if one doesn't already exist:
   ```
   gh pr create --title "<title>" --body "<body>"
   ```
   If a GitHub issue was referenced, include `Closes #<number>` in the PR body.

## Cleanup

9. Do NOT remove the worktree. Worktrees persist until the PR is merged — needed for CI fixes, review feedback, and acceptance testing. Cleanup happens post-merge.
10. Output `<promise>DONE</promise>` when complete

Start with worktree isolation, then step 1.
