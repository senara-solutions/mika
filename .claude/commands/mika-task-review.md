---
name: mika-task-review
description: Review an agent's task — task tree, claude-pilot logs, PRs, and pipeline quality
argument-hint: "<agent_name | task_id>"
---

Review an agent's work by investigating the task tree, claude-pilot logs, and resulting PRs.

**Argument is required.** Either:
- An **agent name** (e.g., `mika-dev`) — finds the latest manual task for that agent
- A **task ID** (UUID format, 36 chars with dashes) — reviews that specific task

## Step 1: Find the task

Detect argument type: if `$ARGUMENTS` matches UUID format (contains dashes, 36 chars), treat as task ID. Otherwise, treat as agent name.

**If agent name:**
```sql
SELECT id, label, status, source, created_at
FROM tasks
WHERE agent_id = '$ARGUMENTS' AND trigger_type = 'manual'
ORDER BY created_at DESC LIMIT 1
```

**If task ID:**
```sql
SELECT id, label, status, source, agent_id, created_at
FROM tasks
WHERE id = '$ARGUMENTS'
```

Run against `~/.mika/data/mika.db`. Store the `agent_id` for Step 7.

## Step 2: Get the task tree

Query the main task and all callback children:

```sql
SELECT id, label, status, trigger_type, action_type, substr(created_at,1,19) as created
FROM tasks
WHERE id = '<task_id>' OR parent_task_id = '<task_id>'
ORDER BY created_at
```

Present as a table: main task status, number of sessions (callback children), each session's status.

## Step 3: Read claude-pilot logs

For each callback child task, check if a log exists at `/var/log/claude-pilot/<task_id>.log`. Also check for logs with custom names (agents sometimes use descriptive names).

For each log found:
- Read the first 5 lines (config, session, prompt)
- Read the last 10 lines (completion status, cost, turns)
- Extract: session ID, turns, cost, duration from the `[done]` line
- Check if the prompt used `/mika` (bad — should use individual `/ce:*` commands)

List all `/var/log/claude-pilot/*.log` files modified today to catch any logs with non-standard names.

## Step 4: Check the branch and PR

From the log prompts or task labels, identify the branch name. Then:

```bash
gh pr list --repo senara-solutions/<repo> --state all --head <branch> --json number,title,state,url
```

If a PR exists, check:
- CI status
- Files changed (does it include `docs/plans/`? `docs/solutions/`?)
- Conflicts

## Step 5: Assess pipeline quality

Check which pipeline artifacts exist in the PR/branch:

| Artifact | Check | Status |
|----------|-------|--------|
| Plan doc | `docs/plans/` file in PR diff | ? |
| Source changes | Non-doc files in PR diff | ? |
| Review findings | `todos/` files in branch | ? |
| Compound doc | `docs/solutions/` file in PR diff | ? |

## Step 6: Present summary

Format the output as:

```
## Task Review: <label>

**Agent:** <agent_id> | **Task:** <id> | **Status:** <status> | **Source:** <source>
**Created:** <timestamp>

### Sessions
| # | Task ID | Purpose | Turns | Cost | Duration | Status |
|---|---------|---------|-------|------|----------|--------|
...

### Pipeline Artifacts
| Artifact | Present | Notes |
|----------|---------|-------|
...

### PR
- URL, status, CI, conflicts

### Issues Found
- Any workflow problems (skipped sessions, /mika used instead of /ce:*, plan not committed, etc.)

### Suggestions
- Specific improvements for the next run
```

## Step 7: Reference

For turn-level inspection (last message + tool calls), use `/mika-turn-review <agent_name>` instead.
