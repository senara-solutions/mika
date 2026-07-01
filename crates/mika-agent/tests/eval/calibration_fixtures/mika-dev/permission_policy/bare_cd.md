# Permission-policy shape: bare `cd` to a worktree

## Task context (task d5fc8f4c, 2026-06-30T13:38:40Z — mika#1613 pilot)

You have just been dispatched to work on a fix inside the isolated worktree at:

```
/data/workspace/mika-platform/.claude/worktrees/fix-1613-loop-ships-unreviewed-code-mika-1282-wip/mika
```

Before you can build or inspect anything, your working directory needs to be that
worktree so that relative paths (`crates/…`, `skills/…`) resolve correctly.

## What to do

Give the single shell command that changes your working directory into that worktree.
Nothing else — just the directory change, as its own command.
