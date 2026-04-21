You have access to the `git_ops` tool for git operations. Use it for fetching, rebasing, merging, pulling, switching branches, and managing worktrees — NOT for committing or creating PRs.

## When to Use

- **Pull from main:** `{"operation": "pull", "repo_path": "/path/to/repo"}`
- **Pull from specific ref:** `{"operation": "pull", "repo_path": "/path/to/repo", "base": "origin/develop"}`
- **Switch branch:** `{"operation": "checkout", "repo_path": "/path/to/repo", "branch": "feat/my-feature"}`
- **Rebase onto main:** `{"operation": "rebase", "repo_path": "/path/to/repo"}`
- **Rebase and push:** `{"operation": "rebase", "repo_path": "/path/to/repo", "push": true}`
- **Fast-forward merge:** `{"operation": "merge", "repo_path": "/path/to/repo"}`
- **Fetch remote updates:** `{"operation": "fetch", "repo_path": "/path/to/repo"}`
- **Create worktree:** `{"operation": "worktree_add", "repo_path": "/path/to/repo", "path": "/path/to/worktree", "branch": "feat/new-branch"}`
- **Create worktree from specific base:** `{"operation": "worktree_add", "repo_path": "/path/to/repo", "path": "/path/to/worktree", "branch": "feat/new-branch", "base": "origin/main"}`
- **Remove worktree:** `{"operation": "worktree_remove", "repo_path": "/path/to/repo", "path": "/path/to/worktree"}`
- **List worktrees:** `{"operation": "worktree_list", "repo_path": "/path/to/repo"}`
- **Prune stale worktrees:** `{"operation": "worktree_prune", "repo_path": "/path/to/repo"}`

## Operations

### fetch
Downloads remote refs without modifying the working tree. Always safe.

### pull
Fetches the remote first, then attempts a **fast-forward only** merge. Equivalent to `git pull --ff-only`. If fast-forward is not possible, the pull fails cleanly with no changes to the working tree. The user should rebase first if fast-forward is not possible.

### rebase
Fetches the remote first, then replays your commits onto the specified base (default: `origin/main`). If conflicts are detected, the rebase is **automatically aborted** and a structured error is returned listing conflicting files. The repository is left in a clean state.

### merge
Fetches the remote first, then attempts a **fast-forward only** merge. If fast-forward is not possible, the merge fails cleanly with no changes to the working tree. The user should rebase first if fast-forward is not possible.

### checkout
Switches to an existing branch using `git switch`. If the branch exists only on the remote, a local tracking branch is created automatically. Fails cleanly if the branch does not exist or if there are uncommitted changes that conflict with the switch.

### worktree_add
Creates a new worktree at the specified `path` with a new branch named `branch` based on `base` (default: `origin/main`). If the branch already exists, attaches the worktree to the existing branch instead.

### worktree_remove
Removes the worktree at the specified `path` (uses `--force`). This will remove the worktree even if it has uncommitted changes.

### worktree_list
Lists all worktrees in machine-readable porcelain format. Useful for discovering existing worktrees before creating or removing them.

### worktree_prune
Cleans up stale worktree references where the worktree directory no longer exists on disk.

## Push Behavior

- `push: true` is only allowed with the `rebase` operation
- Uses `--force-with-lease` (safe force push — refuses if remote was updated by someone else)
- Refuses to push to `main` or `master` branches as a safety check
- Rejected on all other operations

## Pre-flight Checks

The tool automatically verifies before each operation:
- The path exists and is a directory
- The directory is a git repository
- The working tree is clean (for rebase, merge, and pull — no uncommitted changes)
- No rebase or merge is already in progress (for rebase, merge, and pull)

## Important

- Use `git_ops` instead of `run_shell` for git operations — it provides structured results, env var scrubbing, and audit trails
- For git operations NOT covered by this tool (committing, cherry-pick, stash, branch creation/deletion), use `run_shell`
- The `base` parameter defaults to `origin/main` — check the user's default branch if unsure
- If rebase conflicts, suggest the user resolve conflicts manually or try a different approach
- Use `worktree_list` before `worktree_add` to check if a worktree already exists
- The `path` parameter for worktree operations must be an absolute path
