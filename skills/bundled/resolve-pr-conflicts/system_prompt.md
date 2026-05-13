## resolve-pr-conflicts

Resolve PR merge conflicts by rebasing the branch onto its base branch. This is a tactical operation — not a feature development workflow.

## Tool: `resolve_pr_conflicts`

Spawns a claude-pilot session that fetches origin, rebases the PR branch onto the base branch, resolves any conflicts, and pushes with `--force-with-lease`.

### When to Use

Use `resolve_pr_conflicts` when:
- A PR has merge conflicts that need resolving before merge or CI can pass
- A branch needs to be synced with main (or another base branch) after upstream changes
- mika-dev detects a PR's `mergeable` state is `CONFLICTING`

### When NOT to Use

Do **not** use `resolve_pr_conflicts` for:
- Feature development, bug fixes, or any code changes beyond conflict resolution → use `run_claude_pilot` (self-dev) instead
- Creating new branches or worktrees → the worktree must already exist
- Force-pushing without rebase → this tool always rebases first

### Routing Decision (mika-dev)

| Situation | Route to |
|-----------|----------|
| PR has merge conflicts | `resolve_pr_conflicts` (pass `pr_url`) |
| New feature or bug fix | `run_claude_pilot` via self-dev |
| PR needs code changes from review feedback | `run_claude_pilot` with iteration context |
| Branch just needs to be up-to-date with main | `resolve_pr_conflicts` |

### Inputs

| Field | Required | Description |
|-------|----------|-------------|
| `task_id` | Yes | UUID from `create_task` for log correlation |
| `pr_url` | Preferred | Full GitHub PR URL — handler derives the worktree path from the PR's branch name |
| `worktree_path` | Deprecated | Absolute path to the existing git worktree. Use `pr_url` instead. |

At least one of `pr_url` or `worktree_path` must be provided. When `pr_url` is given, the handler derives the correct worktree path automatically using the canonical branch-to-path sanitization rule.

### Behavior

1. Derives the worktree path from the PR's branch name (when `pr_url` provided) or validates the explicit `worktree_path`
2. Validates the worktree path exists and is a git working tree
3. Copies relay config (`.claude/claude-pilot.json`) into the worktree if missing
4. Spawns claude-pilot with a conflict-resolution prompt (no `/mika` pipeline)
5. Claude Code inside the session: fetches origin, detects base branch from PR metadata, rebases, resolves conflicts, runs tests, pushes with `--force-with-lease`
6. Delivers result via `mika ask --task-id` callback

### Expected Outcomes

- **Success:** Branch is rebased onto the base branch, conflicts resolved, tests pass, branch pushed
- **Partial failure:** Conflicts were too complex to resolve automatically — rebase aborted, report delivered with details of which files conflicted
- **Push failure:** Rebase succeeded but push was rejected (concurrent push) — retried once, then reported

### Example

```
resolve_pr_conflicts(
  task_id: "15383984-a3e7-41bf-ac6f-630ba9a89d63",
  pr_url: "https://github.com/senara-solutions/mika/pull/42"
)
```
