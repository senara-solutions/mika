---
name: mika-platform-refresh
description: Sync main and clean up merged-PR worktrees across all mika-platform repos
argument-hint: "[--dry-run]"
---

Run the refresh script and display the output:

```bash
"$(git rev-parse --show-toplevel)/scripts/mika-platform-refresh" $ARGUMENTS
```

Summarize what was synced and what worktrees were removed (or would be removed in dry-run mode).
