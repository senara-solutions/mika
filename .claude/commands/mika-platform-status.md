---
name: mika-platform-status
description: Show git status, branches, and worktrees across all mika-platform repos
argument-hint: "[--verbose]"
---

Run the status script and display the output:

```bash
"$(git rev-parse --show-toplevel)/scripts/mika-platform-status" $ARGUMENTS
```

If the output reveals anything notable (dirty repos, unexpected branches, stale worktrees marked "prunable"), briefly highlight it after showing the output.
