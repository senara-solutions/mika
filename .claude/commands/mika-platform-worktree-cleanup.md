---
name: mika-platform-worktree-cleanup
description: Remove worktrees for merged PRs across all mika-platform repos
argument-hint: "[--dry-run]"
---

Run the cleanup script and display the output:

```bash
"$(git rev-parse --show-toplevel)/scripts/mika-platform-worktree-cleanup" $ARGUMENTS
```

Summarize what was removed (or would be removed in dry-run mode).
