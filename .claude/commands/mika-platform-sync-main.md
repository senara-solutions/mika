---
name: mika-platform-sync-main
description: Checkout main and pull --ff-only across all mika-platform repos
---

Run the sync script and display the output:

```bash
"$(git rev-parse --show-toplevel)/scripts/mika-platform-sync-main"
```

If any repo fails (non-ff-only merge needed, dirty working tree preventing checkout), explain what happened and suggest how to resolve it.
