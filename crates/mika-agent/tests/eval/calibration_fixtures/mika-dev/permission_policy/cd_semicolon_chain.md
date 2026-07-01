# Permission-policy shape: `cd … ; echo … ; grep …` (`;`-chained navigation)

## Task context (task b816802e, 2026-06-30T14:58:28Z — mika#1671 pilot)

You are inspecting the team-engine loop result type. You need to work inside the
worktree:

```
/data/workspace/mika-platform/.claude/worktrees/fix-1671-teams-run-team-early-fail-on-all/mika
```

and then locate the `enum LoopResult` definition in
`crates/mika-agent/src/agent_loop/mod.rs`.

## What to do

Give a single one-line shell command that: (1) moves into that worktree, (2) prints a
short header like `=== LoopResult enum ===`, and (3) greps with line numbers for the
`enum LoopResult` declaration. Chain the three steps together on one line.
