---
name: mika
description: MANDATORY quality gate for ALL implementation work — dispatches to repo pipeline
argument-hint: "[repo#issue | brainstorm topic | description | plan]"
---

<!-- SCOPE: mika-platform meta-repo ONLY. This is the cross-repo dispatcher with an inline self-targeting pipeline. -->

Read the meta-repo CLAUDE.md for workspace conventions.

## Operating principle: start with WHY — no assumptions

Before any dispatch, branch creation, planning, or code change, you MUST establish WHY. This is non-negotiable and overrides any urge to "just get started."

**Rules:**

1. **No assumptions, ever.** If something is not explicitly stated or directly observable in evidence, it is unknown. Treat unknowns as blockers, not as gaps to fill with plausible guesses.
2. **Hard evidence only.** Every factual claim that drives a decision must be backed by a concrete artifact: an issue body, a PR diff, a `gh` query result, a file's contents, a test output, a log line, a CLAUDE.md rule. Cite the source inline (`mika#214 body`, `git log`, `crates/.../foo.rs:42`, etc.). Memory, intuition, and "I recall" are not evidence.
3. **Always ask why.**
   - Ask **the user (Vincent)** when intent, priority, scope, or trade-offs are ambiguous — *before* dispatching, *before* picking a target repo, *before* committing to a plan.
   - Ask **the agents** (`/ce:plan`, `/ce:work`, `/ce:review`, `/ce:compound`) to surface their reasoning: why this approach, why this file, why this acceptance criterion. Reject silent decisions.
   - Ask **the codebase**: read the relevant code, tests, and docs before proposing changes. Do not infer behaviour from names.
4. **Write WHY down.** Every plan, branch description, PR body, and compound doc must lead with the WHY (problem statement, evidence, decision rationale) before the WHAT (steps, files, commands).
5. **When in doubt, stop and ask.** A clarifying question to Vincent is always cheaper than a wrong dispatch. Never proceed on a "best guess" — surface the ambiguity instead.

If at any step below you find yourself filling a gap with assumption rather than evidence, **halt and ask**.

## Keyword inference rules

Used by brainstorm, free-text, and plan-as-input dispatch to determine the target repo. Checked in order — first match wins.

- "helm", "chart", "k8s", "kubernetes", "provision" → `mika-cloud`
- "skill", "marketplace", "manifest", "skill.toml" → `mika-skills`
- "dispatcher", "command", "meta-repo", "makefile", "sync script", "worktree cleanup", "audit command", "solution doc", "brainstorm doc", "mika-platform" → `mika-platform` (self-targeting)
- Everything else → `mika` (the core product)
- If ambiguous, ask the user which repo the task targets

## Branch-name derivation

Every dispatch path below that needs to derive a branch name **MUST** invoke the canonical script `scripts/derive-branch-name`. Do not re-derive the slug with LLM reasoning — redispatches of the same ticket must produce the same branch name, otherwise you get orphan worktrees. Truncation is fixed at **40 characters** to match the dev-pilot dispatcher (the runtime authority); see the script's header comment for rationale.

The script enforces this priority order automatically:

1. **Explicit `branch:<name>` prefix** — caller already supplied the branch; pass it via `--explicit`.
2. **Issue body callout** — if an issue was fetched, pass the body via `--body-callout`. The script extracts the branch from the line `> - **Branch:** \`<branch>\``.
3. **Conventional-commit prefix in the title** — `--title "fix(kg): ..."` derives `type=fix`.
4. **Label override** — `--labels "bug,priority:high"` maps `bug → fix`, `chore → chore`.
5. **Default** — `feat`.

Bash invocation pattern (works from both meta-repo root and any meta-repo worktree because `git rev-parse --git-common-dir` returns the shared `.git` directory in both cases):

```bash
SCRIPTS_DIR="$(dirname "$(git rev-parse --git-common-dir)")/scripts"
BRANCH=$("$SCRIPTS_DIR/derive-branch-name" \
  --title "$ISSUE_TITLE" \
  --issue "$ISSUE_NUMBER" \
  --labels "$LABELS" \
  --body-callout "$ISSUE_BODY")
```

When a sub-repo is targeted, dispatch with `branch:<name>` so the sub-repo's `/mika` skips re-derivation at its step 1.

## Brainstorm dispatch

If `$ARGUMENTS` starts with `brainstorm` (case-insensitive), this is an exploratory brainstorm — not implementation.

**Steps:**
1. Strip the `brainstorm` prefix from `$ARGUMENTS` to get the topic.
2. If the topic starts with an explicit repo name (`mika`, `mika-cloud`, `mika-skills`, `mika-platform`), use that as the target repo and strip it from the topic. Otherwise, determine the target repo by keyword inference (see rules above).
3. If the target is `mika-platform`, stay in the meta-repo root. Otherwise, cd into the target repo.
4. Invoke `/ce:brainstorm <topic>` via the Skill tool. The brainstorm doc will land in the target's `docs/brainstorms/`.
5. If you cd'd into a sub-repo, cd back to the meta-repo root.

**No worktree, no branch, no PR, no `/ce:compound`.** Brainstorming is exploratory — it produces a document, not code.

Stop here — brainstorm is complete.

## Direct dispatch

If `$ARGUMENTS` specifies an issue, skip evaluation and dispatch directly.

**Accepted format:**
- `mika#214`, `mika-cloud#50`, `mika-skills#8`, `mika-platform#6` — `repo#number` (no space). The repo is always explicit.
- `#N` (bare number) — probe all repos to find the issue, then dispatch to the repo that has it. If the issue number exists on multiple repos, present all matches and ask the user which repo to target.

### Self-targeting path (target is `mika-platform`)

If the target repo is `mika-platform`:

1. Fetch the issue: `gh issue view <number> --repo senara-solutions/mika-platform --json number,title,body,labels`
2. Derive a branch name per **§ Branch-name derivation** above (priority: `branch:` prefix → body callout → deterministic recipe).
3. Run the **self-targeting pipeline** below, passing `branch:<branch> #<number>` as the arguments.

Stop here after the pipeline completes.

### Sub-repo path (target is mika, mika-cloud, or mika-skills)

1. Determine the target repo (from argument or by probing)
2. Fetch the issue: `gh issue view <number> --repo senara-solutions/<repo> --json number,title,body,labels`
3. Derive a branch name per **§ Branch-name derivation** above (priority: `branch:` prefix → body callout → deterministic recipe).
4. cd into the target repo
5. Read the target repo's `.claude/commands/mika.md` file and follow its instructions directly, passing `branch:<branch> #<number>` as `$ARGUMENTS`. **Do NOT invoke `/mika` via the Skill tool** — the Skill tool always loads the meta-repo dispatcher, causing infinite recursion.
6. After the repo-level pipeline completes, cd back to the meta-repo root and run `/ce:compound` to document what was built and why.

Stop here — the repo-level pipeline handles everything from planning through PR.

## Free-text dispatch

If `$ARGUMENTS` is provided but is NOT an issue reference (i.e., doesn't match `repo#\d+` or `#\d+`), treat it as a free-text task description.

**Steps:**
1. Determine the target repo by keyword inference (see rules above).
2. Derive a branch name per **§ Branch-name derivation** (no issue number, deterministic recipe on the description).

### Self-targeting path (target is `mika-platform`)

3. Run the **self-targeting pipeline** below, passing `branch:<branch> <description>` as the arguments.

Stop here after the pipeline completes.

### Sub-repo path

3. cd into the target repo
4. Read the target repo's `.claude/commands/mika.md` file and follow its instructions directly, passing `branch:<branch> <description>` as `$ARGUMENTS`. **Do NOT invoke `/mika` via the Skill tool** — it always loads the meta-repo dispatcher.
5. After the repo-level pipeline completes, cd back to the meta-repo root and run `/ce:compound` to document what was built and why.

Stop here — the repo-level pipeline handles everything from planning through PR.

## Plan-as-input dispatch

If `$ARGUMENTS` contains or references a pre-written implementation plan (e.g., "implement the langfuse fix plan", a multi-step plan pasted inline, or a plan document path), treat it as a plan-accelerated task.

**Steps:**
1. Determine the target repo by keyword inference (same rules above)
2. Derive a branch name per **§ Branch-name derivation** (from the plan's goal)

### Self-targeting path (target is `mika-platform`)

3. Run the **self-targeting pipeline** below, passing `branch:<branch> <plan summary>` as the arguments.

Stop here after the pipeline completes.

### Sub-repo path

3. cd into the target repo
4. Read the target repo's `.claude/commands/mika.md` file and follow its instructions directly, passing `branch:<branch> <plan summary>` as `$ARGUMENTS`. **Do NOT invoke `/mika` via the Skill tool** — it always loads the meta-repo dispatcher.
5. After the repo-level pipeline completes, cd back to the meta-repo root and run `/ce:compound` to document what was built and why.

**Important — distinguish two scenarios:**

- **Groomed scenario** (issue body has `> - **Plan:**` callout pointing at a committed plan file on the dispatch branch — produced by `/mika-groom-ticket`): `/ce:plan` runs in **deepen / resume mode** against that existing plan. Phase 0.1 of `/ce:plan` ("Resume Existing Plan Work When Appropriate") triggers automatically when the plan file is in `docs/plans/` and matches the topic — confidence check + optional sharpening, NOT re-derivation. Re-running full `/ce:plan` against an already-architect-validated plan-on-branch is wasted churn and risks regenerating a different shape than the one the architect signed off on. The plan-on-branch is the contract (per `/mika-groom-ticket.md` § Discipline); the dispatcher honors it.

- **Un-groomed scenario** (free-text dispatch, plan-as-input from conversation, or no plan-on-branch): `/ce:plan` creates from scratch — adopts/refines the inline plan if one was supplied, or derives fresh from the issue.

**Subsequent steps run in both scenarios.** `/ce:work` → `/ce:review` → `/compound-engineering:resolve_todo_parallel` → `/ce:compound` → PR. The pipeline is a quality gate; only `/ce:plan`'s mode adapts to whether a groomed plan already exists.

Stop here — the repo-level `/mika` handles everything from planning through PR.

---

## Self-targeting pipeline

This pipeline runs when the target repo is `mika-platform` itself. It follows the same structure as sub-repo pipelines but runs inline — there is no separate repo to delegate to.

**Do NOT invoke `/mika` via the Skill tool from within this pipeline.** The pipeline calls `/ce:plan`, `/ce:work`, `/ce:review`, `/ce:compound` — none of these re-invoke `/mika`.

### Issue linking

If the arguments (after stripping any `branch:` prefix) start with `#` followed by a number (e.g. `#6`) or are just a number, treat it as a GitHub issue reference. Run `gh issue view <number> --repo senara-solutions/mika-platform --json number,title,body,labels` to fetch the issue details, then use the issue title and body as the feature description for the planning step. Remember the issue number for the PR step.

### Worktree isolation

1. **Parse branch:** If arguments start with `branch:<name>`, extract `<name>` as the branch name and strip the prefix. Otherwise, derive the branch name per **§ Branch-name derivation** above (priority: body callout → deterministic recipe).
2. **Skip if no branch or no args:** If there are no arguments (backlog eval mode), skip worktree creation and run the pipeline in the current directory.
3. **Detect existing worktree (MANDATORY):** Run `git rev-parse --git-dir` and `git rev-parse --git-common-dir`. If they differ, you are ALREADY inside a worktree. **STOP worktree setup immediately** — set `CREATED_WORKTREE=false` and proceed directly to the Pipeline section below. Do NOT attempt to create, remove, or modify any worktree. Do NOT clean up or recreate. Just use the current directory as-is.
4. **Sync main:** Run `git fetch origin main:main` to fast-forward local `main` to match remote. If it fails (e.g., `main` is checked out with uncommitted changes), fall back to `git fetch origin` and use `origin/main` as the base ref in the next step.
5. **Create worktree:** Compute the worktree path via the canonical script, then create it. Record `ORIGINAL_DIR=$(pwd)`.
   ```bash
   SCRIPTS_DIR="$(dirname "$(git rev-parse --git-common-dir)")/scripts"
   WORKTREE=$("$SCRIPTS_DIR/derive-worktree-path" --branch "$BRANCH" --repo mika-platform)
   ```
   - If the worktree path already exists, remove it first: `git worktree remove --force "$WORKTREE"` (ignore errors).
   - Try: `git worktree add -b "$BRANCH" "$WORKTREE" main`
   - If that fails (branch already exists): `git worktree add "$WORKTREE" "$BRANCH"`
   - cd into the worktree. Set `CREATED_WORKTREE=true`.

### Pipeline

**Branch safety (MANDATORY):** You are already on the correct branch. Run `git branch --show-current` to confirm. Do NOT create, rename, or switch branches. All commits and the PR must use the current branch. This applies to every step below — including `/ce:plan`, `/ce:work`, `/ce:review`, and PR creation.

Run these steps in order. Do not stop between steps — complete every step through to the end.

1. `/ce:plan $ARGUMENTS` (if an issue was detected, pass the issue title + body instead of raw arguments)
2. `/ce:work`
3. `/ce:review`
4. `/compound-engineering:resolve_todo_parallel`
5. `/ce:compound`
6. Run `bash scripts/verify-pipeline.sh` to verify pipeline artifacts exist. If it fails, read the error messages to identify missing artifacts, go back and produce them (run `/ce:plan` if no plan doc, `/ce:work` if no source changes), then re-run verification until it passes.
7. Create a PR if one doesn't already exist:
   ```
   gh pr create --repo senara-solutions/mika-platform --title "<title>" --body "<body>"
   ```
   If a GitHub issue was referenced, include `Closes #<number>` in the PR body.

### Cleanup

8. Do NOT remove the worktree. Worktrees persist until the PR is merged — needed for CI fixes, review feedback, and acceptance testing. Cleanup happens post-merge.
9. Output `<promise>DONE</promise>` when complete.

---

## Step 1: Gather context

If no direct dispatch argument was provided, evaluate the backlog.

For each repo (mika, mika-cloud, mika-skills, mika-platform), run in parallel:

```bash
gh issue list --repo senara-solutions/<repo> --state open --json number,title,labels,milestone,updatedAt,body --limit 20
gh pr list --repo senara-solutions/<repo> --state merged --json number,title,mergedAt --limit 5
```

Also read:
- `docs/brainstorms/` — list recent files, read any from the last 14 days
- `docs/solutions/` — list recent files for awareness of compounded knowledge

## Step 2: Evaluate and present

Analyze the gathered context. Present a prioritized view.

**For each repo with open issues:**
- Group issues by repo
- For each issue: number, title, labels, one-line assessment of why it matters now
- Flag cross-repo dependencies (look for mentions of other repos in issue bodies)
- Note what's unblocked by recently merged PRs

**Cross-repo gaps:**
- Things that should be issues but aren't (informed by brainstorms, solutions, and repo state)

Ask: "What would you like to work on?"

## Step 3: Dispatch

When a task is selected:

1. Determine the target repo from the issue
2. Derive a branch name per **§ Branch-name derivation** above

**If target is `mika-platform`:** Run the self-targeting pipeline above with `branch:<branch> #<issue_number>`.

**If target is a sub-repo:**
3. cd into the target repo
4. Read the target repo's `.claude/commands/mika.md` file and follow its instructions directly, passing `branch:<branch> #<issue_number>` as `$ARGUMENTS`. **Do NOT invoke `/mika` via the Skill tool** — it always loads the meta-repo dispatcher.
5. After the repo-level pipeline completes, cd back to the meta-repo root and run `/ce:compound` to document what was built and why.

**Cross-repo tasks:** If the task spans multiple repos, follow CLAUDE.md conventions:
- Same branch name across all affected repos
- Primary repo first (see CLAUDE.md Common scenarios table)
- After completing the primary, return to meta-repo root and dispatch the secondary
- After all repos are done, run `/ce:compound` once to document the full cross-repo change
