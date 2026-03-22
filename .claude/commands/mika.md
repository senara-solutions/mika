---
name: mika
description: MANDATORY quality gate for ALL implementation work — dispatches to repo pipeline
argument-hint: "[#issue | description | plan]"
---

<!-- SCOPE: mika-platform meta-repo ONLY. This is the cross-repo dispatcher. Do NOT replace with a repo-level workflow. -->

Read the meta-repo CLAUDE.md for workspace conventions.

## Direct dispatch

If `$ARGUMENTS` specifies an issue, skip evaluation and dispatch directly.

**Accepted formats:**
- `mika #214` or `mika-cloud #50` — repo + issue number
- `#214` or `214` — issue number only; determine the repo by checking each repo: `gh issue view <number> --repo senara-solutions/<repo>` (try mika first, then mika-cloud, then mika-skills)

**Steps:**
1. Determine the target repo (from argument or by probing)
2. Fetch the issue: `gh issue view <number> --repo senara-solutions/<repo> --json number,title,body,labels`
3. Derive a branch name following the convention: `feat|fix|chore/<number>/<short-description>` (derive type from issue labels — `bug` → fix, `enhancement` → feat, default → feat)
4. cd into the target repo
5. Read the target repo's `.claude/commands/mika.md` file and follow its instructions directly, passing `branch:<branch> #<number>` as `$ARGUMENTS`. **Do NOT invoke `/mika` via the Skill tool** — the Skill tool always loads the meta-repo dispatcher, causing infinite recursion.
6. After the repo-level pipeline completes, cd back to the meta-repo root and run `/ce:compound` to document what was built and why.

Stop here — the repo-level pipeline handles everything from planning through PR.

## Free-text dispatch

If `$ARGUMENTS` is provided but is NOT an issue number (i.e., doesn't match `#\d+`, `\d+`, or `<repo> #\d+`), treat it as a free-text task description.

**Steps:**
1. Determine the target repo by keyword inference:
   - "helm", "chart", "deploy", "k8s", "kubernetes", "provision" → `mika-cloud`
   - "skill", "marketplace", "manifest", "skill.toml" → `mika-skills`
   - Everything else → `mika` (the core product)
   - If ambiguous, ask the user which repo the task targets
2. Derive a branch name: `feat/<short-kebab-description>` (from the description, no issue number)
3. cd into the target repo
4. Read the target repo's `.claude/commands/mika.md` file and follow its instructions directly, passing `branch:<branch> <description>` as `$ARGUMENTS`. **Do NOT invoke `/mika` via the Skill tool** — it always loads the meta-repo dispatcher.
5. After the repo-level pipeline completes, cd back to the meta-repo root and run `/ce:compound` to document what was built and why.

Stop here — the repo-level pipeline handles everything from planning through PR.

## Plan-as-input dispatch

If `$ARGUMENTS` contains or references a pre-written implementation plan (e.g., "implement the langfuse fix plan", a multi-step plan pasted inline, or a plan document path), treat it as a plan-accelerated task.

**Steps:**
1. Determine the target repo by keyword inference (same rules as free-text dispatch above)
2. Derive a branch name: `feat/<short-kebab-description>` (from the plan's goal)
3. cd into the target repo
4. Read the target repo's `.claude/commands/mika.md` file and follow its instructions directly, passing `branch:<branch> <plan summary>` as `$ARGUMENTS`. **Do NOT invoke `/mika` via the Skill tool** — it always loads the meta-repo dispatcher.
5. After the repo-level pipeline completes, cd back to the meta-repo root and run `/ce:compound` to document what was built and why.

**Important:** All pipeline steps still run. A pre-written plan accelerates `/ce:plan` (the planner can adopt or refine it) but does NOT skip `/ce:plan` or any subsequent step (work → review → TODOs → doc audit → compound → PR). The pipeline is a quality gate, not a planning tool.

Stop here — the repo-level `/mika` handles everything from planning through PR.

## Step 1: Gather context

If no direct dispatch argument was provided, evaluate the backlog.

For each repo (mika, mika-cloud, mika-skills), run in parallel:

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
2. Derive a branch name: `feat|fix|chore/<issue_number>/<short-description>`
3. cd into the target repo
4. Read the target repo's `.claude/commands/mika.md` file and follow its instructions directly, passing `branch:<branch> #<issue_number>` as `$ARGUMENTS`. **Do NOT invoke `/mika` via the Skill tool** — it always loads the meta-repo dispatcher.
5. After the repo-level pipeline completes, cd back to the meta-repo root and run `/ce:compound` to document what was built and why.

**Cross-repo tasks:** If the task spans multiple repos, follow CLAUDE.md conventions:
- Same branch name across all affected repos
- Primary repo first (see CLAUDE.md Common scenarios table)
- After completing the primary, return to meta-repo root and dispatch the secondary
- After all repos are done, run `/ce:compound` once to document the full cross-repo change
