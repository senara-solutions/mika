## resolve-pr-conflicts

Resolve PR merge conflicts by rebasing the branch onto its base branch. This is a tactical operation — not a feature development workflow.

## Tool: `resolve_pr_conflicts`

Spawns a claude-pilot session that fetches origin, rebases the PR branch onto the base branch, and resolves any conflicts. **The pilot does not push.** After the session exits, the handler publishes the result itself, at a single guarded site, with an explicit destination refspec and a lease pinned to a SHA captured before the session started (mika#2520).

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
| `worktree_path` | Deprecated | Absolute path to the existing git worktree. Use `pr_url` instead — **the push is refused on this path**, because it resolves no PR head. |

At least one of `pr_url` or `worktree_path` must be provided. When `pr_url` is given, the handler derives the correct worktree path automatically using the canonical branch-to-path sanitization rule.

`worktree_path` alone resolves neither `headRefName` nor `baseRefName`, so the push target cannot come from the PR. Conflict resolution still runs on that path; the push is refused under the named reason `no_pr_url` and the work stays in the worktree.

### Behavior

1. Resolves the push target from the PR head in **one** `gh pr view --json headRefName,baseRefName` call, and derives the worktree path from that same branch name. The target never comes from `git branch --show-current` or from the worktree's local push configuration
2. **Refuses before spawning** if the resolved target is a protected branch, is the PR's own base branch, or could not be read at all — no LLM turn is spent on a refusal
3. Validates the worktree path exists and is a git working tree
4. Captures the push lease: the remote SHA of the PR branch, read **before** the session. A literal SHA, so the pilot's own `git fetch` cannot void it
5. Copies relay config (`.claude/claude-pilot.json`) into the worktree if missing
6. Spawns claude-pilot with a conflict-resolution prompt (no `/mika` pipeline). The prompt carries the resolved branch and base, and **no publishing instruction at all**
7. Claude Code inside the session: fetches origin, rebases onto the given base, resolves conflicts, runs tests, and stops there
8. After the session, the handler re-checks the worktree (HEAD on the resolved branch, no rebase in flight, clean tree, remote branch still present), re-affirms the target refusals, and pushes once with `--force-with-lease=<branch>:<pre-session SHA> origin HEAD:refs/heads/<branch>`
9. Delivers result via `mika ask --task-id` callback

### Expected Outcomes

- **Success:** Branch is rebased onto the base branch, conflicts resolved, tests pass, branch pushed. The callback names the refspec and the lease SHA
- **Partial failure:** Conflicts were too complex to resolve automatically — rebase aborted, report delivered with details of which files conflicted
- **Refusal:** Nothing was pushed, and the callback says which term refused. The rebase work, if any, is intact in the worktree. The reasons are a stable vocabulary: `protected_branch` (the target resolved to main/master), `branch_is_base` (head and base coincide), `unresolved_branch` / `unresolved_base` (the PR could not be read), `head_mismatch` (the worktree's HEAD is detached or on another branch), `not_publishable` (a rebase is still in flight, the tree is dirty, or the remote branch is gone), `no_lease` (no pre-session lease was taken), `no_pr_url` (the deprecated `worktree_path` input resolves no PR head, so the push is refused there by construction). Each refusal names its own lifting gesture
- **Push failure:** Rebase succeeded but the push was rejected. **Not retried, deliberately** — the one legitimate failure here is a concurrent push, and re-leasing on the new SHA would overwrite that work. A transient failure costs one dispatch; the commits stay in the worktree

### Example

```
resolve_pr_conflicts(
  task_id: "15383984-a3e7-41bf-ac6f-630ba9a89d63",
  pr_url: "https://github.com/senara-solutions/mika/pull/42"
)
```
