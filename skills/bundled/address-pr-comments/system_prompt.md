## address-pr-comments

Address PR review comments by fetching them from GitHub and running a focused claude-pilot session in the existing worktree. This is a tactical operation — not a feature development workflow.

## Tool: `address_pr_comments`

Spawns a claude-pilot session that reads PR review comments, makes the requested changes, runs tests, and pushes.

### When to Use

Use `address_pr_comments` when:
- A PR has review comments from human reviewers that need to be addressed
- Vincent asks to "address review comments" or "handle PR feedback" for a specific PR
- mika-dev detects unresolved review comments on an open PR

### Inputs

| Field | Required | Description |
|-------|----------|-------------|
| `pr_url` | Yes | Full GitHub PR URL (e.g., `https://github.com/senara-solutions/mika/pull/42`) |
| `worktree_path` | Yes | Absolute path to the existing git worktree for the PR branch |
| `task_id` | Yes | UUID from `create_task` for log correlation |

### Behavior

1. Validates the worktree path exists and is a git working tree
2. Checks PR state — skips if merged or closed
3. Fetches review comments via `gh api` (line-level + review body text, filters out bots)
4. If zero actionable comments found, returns early with success
5. Copies relay config (`.claude/claude-pilot.json`) into the worktree if missing
6. Constructs a focused prompt from the review comments
7. Spawns claude-pilot in free-text mode (no `/mika` pipeline)
8. Delivers result via `mika ask --task-id` callback

### Expected Outcomes

- **Success:** Review comments addressed, tests pass, changes pushed to the branch
- **No comments:** Zero actionable review comments found — returns success with "Nothing to address"
- **Partial failure:** Some comments were too complex to address — report delivered with details
- **PR closed/merged:** Skipped — returns early with explanation

### Example

```
address_pr_comments(
  pr_url: "https://github.com/senara-solutions/mika/pull/42",
  worktree_path: "/home/user/workspace/mika-platform/.claude/worktrees/feat-42-add-health-endpoint/mika",
  task_id: "15383984-a3e7-41bf-ac6f-630ba9a89d63"
)
```
