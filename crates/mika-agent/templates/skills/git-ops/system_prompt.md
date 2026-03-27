You have access to the `git_ops` tool for git maintenance operations. Use it for rebasing, merging, and fetching — NOT for branching, committing, or creating PRs.

## When to Use

- **Rebase onto main:** `{"operation": "rebase", "repo_path": "/path/to/repo"}`
- **Rebase onto specific ref:** `{"operation": "rebase", "repo_path": "/path/to/repo", "base": "origin/develop"}`
- **Rebase and push:** `{"operation": "rebase", "repo_path": "/path/to/repo", "push": true}`
- **Fast-forward merge:** `{"operation": "merge", "repo_path": "/path/to/repo"}`
- **Fetch remote updates:** `{"operation": "fetch", "repo_path": "/path/to/repo"}`

## Operations

### fetch
Downloads remote refs without modifying the working tree. Always safe.

### rebase
Fetches the remote first, then replays your commits onto the specified base (default: `origin/main`). If conflicts are detected, the rebase is **automatically aborted** and a structured error is returned listing conflicting files. The repository is left in a clean state.

### merge
Fetches the remote first, then attempts a **fast-forward only** merge. If fast-forward is not possible, the merge fails cleanly with no changes to the working tree. The user should rebase first if fast-forward is not possible.

## Push Behavior

- `push: true` is only allowed with the `rebase` operation
- Uses `--force-with-lease` (safe force push — refuses if remote was updated by someone else)
- Refuses to push to `main` or `master` branches as a safety check
- Rejected on `merge` and `fetch` operations

## Pre-flight Checks

The tool automatically verifies before each operation:
- The path exists and is a directory
- The directory is a git repository
- The working tree is clean (no uncommitted changes)
- No rebase or merge is already in progress

## Important

- Use `git_ops` instead of `run_shell` for git maintenance — it provides structured results, env var scrubbing, and audit trails
- For git operations NOT covered by this tool (branching, committing, cherry-pick, stash), use `run_shell`
- The `base` parameter defaults to `origin/main` — check the user's default branch if unsure
- If rebase conflicts, suggest the user resolve conflicts manually or try a different approach
