---
title: dispatch-lib plan-on-branch entry command override
date: 2026-05-11
category: best-practices
module: skills
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - dispatch-lib.sh dispatches a claude-pilot session for a groomed ticket with a plan callout in the issue body
  - Debugging narrate-then-exit failures where the model recognizes it should call /ce:work but narrates instead
  - Extending the plan-on-branch detection pattern to new dispatch skills
tags:
  - dispatch-lib
  - claude-pilot
  - plan-on-branch
  - entry-command
  - narrate-then-exit
  - dev-pilot
  - groomed-plans
---

# dispatch-lib plan-on-branch entry command override

## Context

When the grooming pipeline writes a plan callout into an issue body (`> - **Plan:** \`docs/plans/<file>.md\``), the autonomous dispatch flow previously always passed `--command "/mika"` to claude-pilot. Inside the Claude Code session, the `/mika` slash command detected the plan callout and skipped to `/ce:work`. However, this detection happened *inside* the model session — the model had to parse the issue, recognize the plan, and invoke `/ce:work`. This is where the narrate-then-exit failure class occurred: the model would narrate "Proceeding to /ce:work" and call `end_turn` instead of actually invoking the slash command.

10+ prior prompt-enforcement attempts failed to eliminate this behavior reliably. The structural insight: if the model doesn't need to make the decision, it can't narrate instead of acting.

## Guidance

Move plan-on-branch detection upstream to `dispatch-lib.sh` (the shell script that launches claude-pilot). When the issue body contains the plan callout, pass `--command "/ce:work <PLAN_PATH>"` directly instead of `--command "/mika"`.

The detection is implemented as `_detect_plan_on_branch()` — an internal helper called after `_set_up_worktree()` (which populates `$ISSUE_BODY` and `$WORKTREE_DIR`) and before `_handle_dry_run()`. It:

1. Guards on `SKILL=dev-pilot` only (dev-groom has its own entry command)
2. Extracts the plan path using `grep -oP` with a PCRE pattern requiring the `docs/plans/` prefix
3. Validates the file exists in the worktree with `[ -f ]`
4. Overrides `ENTRY_COMMAND` only when all checks pass; falls back silently otherwise

The case switch (mika#932 contract) remains unchanged — it sets the default, and the plan detection conditionally overrides it.

## Why This Matters

Prompt-level enforcement is unreliable for preventing model narration failures. When a model can "decide" to take an action, it can also decide to narrate about taking the action instead. The only structural fix is to eliminate the decision point by passing the correct entry command upstream. This pattern applies to any dispatch flow where the model's first action is deterministic based on available metadata.

## When to Apply

- When adding new dispatch skills that have deterministic entry commands based on issue metadata
- When debugging narrate-then-exit failures in any claude-pilot dispatch path
- When extending plan-on-branch detection to other callout patterns (e.g., branch callouts, label-based routing)

## Examples

Before (narrate-then-exit prone):
```bash
# dispatch-lib.sh always passes /mika
case "$SKILL" in
  dev-pilot)  ENTRY_COMMAND="/mika" ;;
esac
# Model inside /mika must decide to invoke /ce:work
```

After (structural fix):
```bash
# Case switch sets default
case "$SKILL" in
  dev-pilot)  ENTRY_COMMAND="/mika" ;;
esac

# After worktree setup, detect plan callout and override
_detect_plan_on_branch  # Sets ENTRY_COMMAND="/ce:work <path>" if plan found

# claude-pilot receives the correct entry command directly
claude-pilot --command "$ENTRY_COMMAND" ...
```

## Related

- [Shared dispatch library for claude-pilot skills](shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md) — the dispatch-lib architecture this builds on
- [Auto-groom on dispatch](auto-groom-on-dispatch-2026-05-06.md) — the grooming pipeline that produces plan callouts
- mika#1074 — structural fix issue
- mika#1072 — prompt-level mitigation (interim fix)
