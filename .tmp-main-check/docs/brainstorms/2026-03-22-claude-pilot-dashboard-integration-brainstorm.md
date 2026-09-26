---
date: 2026-03-22
topic: claude-pilot-dashboard-integration
---

# Claude-Pilot Dashboard Integration

## What We're Building

Make the autonomous dev loop (mika-dev → claude-pilot → Claude Code → PR) fully observable and actionable through the Mika dashboard. Today, dev runs are invisible — scattered across Telegram notifications, raw log files, and GitHub. The goal: both mika-dev (the agent) and Vincent (the human) can monitor, review, and act on dev work from a single interface.

Additionally, give mika-dev her own GitHub account so she can autonomously merge PRs that pass acceptance testing, closing the loop without human intervention for routine changes.

## Why This Approach

The data already exists but it's fragmented:
- Work items → `tasks` table (SQLite)
- Run logs → `/var/log/claude-pilot/<task-id>.log` (flat files)
- PR/branch → GitHub (separate system)
- Cost/duration → buried in log `[done]` line
- Permission decisions → buried in log `[relay:recv]` lines

Rather than building a separate system, we enrich what exists: structured metadata on work items, a dashboard page to view them, and GitHub integration for merge actions.

## Key Decisions

### 1. Dev Runs page in the dashboard

New page showing claude-pilot runs as a table:

| Column | Source | Notes |
|--------|--------|-------|
| Issue | work item `reference_url` | Links to GitHub issue |
| Branch | run metadata | feat/232/oauth-pkce |
| Status | derived | running / pr-open / merged / failed |
| PR | run metadata | Links to GitHub PR |
| Cost | callback result | e.g., $1.50 |
| Duration | callback timestamps | e.g., 87s |
| Turns | callback result | e.g., 9 |
| Actions | interactive | Merge PR, View Diff, Re-run, Cancel |

Status lifecycle: `running` → `pr-open` → `merged` or `failed`

### 2. Structured run metadata on work items

Extend work items with a `metadata` JSON field (or dedicated columns) to store:

```json
{
  "claude_pilot": {
    "session_id": "5168bdf9",
    "branch": "feat/19/ecr-iam",
    "worktree_path": ".claude/worktrees/feat-19-ecr-iam/mika-cloud",
    "pr_number": 43,
    "pr_url": "https://github.com/senara-solutions/mika-cloud/pull/43",
    "repo": "mika-cloud",
    "cost_usd": 1.50,
    "duration_ms": 87000,
    "turns": 9,
    "log_path": "/var/log/claude-pilot/14a48ba6.log"
  }
}
```

This is populated by:
- mika-dev at launch time (branch, repo, worktree)
- claude-pilot callback result (session_id, cost, duration, turns)
- mika-dev after PR creation (pr_number, pr_url)

### 3. Claude-pilot structured callbacks

Today claude-pilot returns a text summary. Change to structured JSON:

```json
{
  "status": "completed",
  "session_id": "5168bdf9",
  "turns": 9,
  "cost_usd": 1.50,
  "duration_ms": 87000,
  "pr_url": "https://github.com/senara-solutions/mika-cloud/pull/43",
  "branch": "feat/19/ecr-iam"
}
```

This lets mika-dev programmatically extract run results instead of parsing text.

### 4. Mika-dev GitHub account for autonomous merges

Create a GitHub account for mika-dev (mika-dev@getmika.ai) so she can:
- Merge PRs that pass acceptance testing (without waiting for Vincent)
- Comment on PRs with test evidence
- Close issues on merge

**Merge policy (acceptance criteria for autonomous merge):**
- All CI checks pass (green status)
- Acceptance testing passes (build success + criteria from issue)
- No review comments requesting changes
- PR has been open for at least 5 minutes (prevents merge-before-review)
- Only on repos where mika-dev has write access
- Never force-merge or merge to a protected branch without approval

**What mika-dev should NOT do autonomously:**
- Merge PRs that touch security-sensitive code (auth, crypto, permissions)
- Merge PRs that modify CI/CD pipelines or deployment configs
- Merge PRs with failing checks
- Merge PRs that have unresolved review comments

### 5. Dashboard merge action

The dashboard's "Merge" button:
1. Checks CI status via GitHub API
2. Shows confirmation dialog with PR summary
3. Merges via `gh pr merge --merge --delete-branch`
4. Updates work item status to `completed`
5. Triggers worktree cleanup

This works for both Vincent (clicking in browser) and mika-dev (via API call to the dashboard endpoint).

## Cross-Repo Impact

| Repo | Changes |
|------|---------|
| **claude-pilot** | Structured callback format (JSON instead of text) |
| **mika** (agent) | Work item metadata field, `update_work_item` tool accepts metadata, dashboard API endpoints for dev runs |
| **mika** (dashboard) | New Dev Runs page, merge action, run detail view |
| **mika-skills** (self-dev) | Update prompt to populate structured metadata on work items |

## Open Questions

- Should the dashboard show live streaming logs from claude-pilot (WebSocket), or just the final summary?
- Should mika-dev's merge permissions be scoped per-repo, or all read-write repos?
- Should we add a "Sprint" view that groups runs by sprint session?
- How does mika-dev's GitHub account interact with branch protection rules?

## Next Steps

1. Create mika-dev GitHub account (mika-dev@getmika.ai)
2. File issues on mika for: work item metadata, dashboard dev runs page, dashboard merge action
3. File issue on claude-pilot for: structured callback format
4. Update self-dev skill to populate structured metadata
→ `/ce:plan` for implementation details when ready
