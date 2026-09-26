---
title: "qa-review skill deployed on mika-dev blocks dev operations"
date: 2026-04-17
category: integration-issues
module: skills
problem_type: integration_issue
component: tooling
symptoms:
  - "mika-dev run_gh calls fail with 'issue list not in qa-review allowlist'"
  - "Agent falls back to run_shell and raw curl, all blocked"
  - "9 LLM steps, ~335K input tokens, 219s latency, zero useful work"
  - "Agent never replies to user — turn abandoned after restart"
root_cause: config_error
resolution_type: config_change
severity: high
tags: [skills, qa-review, mika-dev, run-gh, allowlist, always-on, bundled-skills]
---

# qa-review skill deployed on mika-dev blocks dev operations

## Problem

mika-dev could not list milestone issues or perform any `run_gh` operation beyond PR review commands. Every attempt was blocked by the qa-review skill's restricted allowlist, causing the agent to burn tokens on futile fallback attempts without ever replying to the user.

## Symptoms

- `run_gh issue list --milestone 6` → "Command 'issue list' not in qa-review allowlist. run_gh is restricted to: pr review, pr diff, pr list, issue view"
- `run_gh api repos/.../milestones/6/issues` → "Direct API calls not permitted"
- `run_shell gh issue list ...` → "Use the dedicated run_gh skill instead of run_shell for security"
- Agent cycled through 9 LLM steps trying alternatives (curl with bad credentials, delegation attempts, web search), never produced a response
- Turn was orphaned when mika-dev restarted mid-turn at step 9

## What Didn't Work

- **Vincent's workaround** — sent "set your skill qa-review to always on off". mika-dev correctly called `update_skill(name="qa-review", always_on=false)` and it persisted to `skill_overrides`. But the agent simultaneously resumed the blocked milestone task in the same turn. The tool output says "Changes take effect immediately on the next turn" — so qa-review was still active for the remainder of this turn, and every subsequent step repeated the same allowlist failures.
- **The always_on override** — even with `always_on=false` in `skill_overrides`, the skill remains installed on the wrong agent. It can still be keyword-triggered by messages containing "review", "pr", "qa", or "pull request" and re-impose its restricted `run_gh` allowlist.

## Solution

Remove qa-review from mika-dev entirely — it belongs only on mika-qa:

```bash
mika skills --agent mika-dev remove qa-review
mika skills --agent mika-dev remove qa-review-build-callback
```

Verify it's not re-installed by `make deploy` or `seed_bundled_skills()`. The bundled skill seeding (`seed_bundled_skills` in `crates/mika-agent/src/skills/bundled.rs`) installs skills into whichever agent's skills directory it targets — it does not enforce per-agent scope. If `make deploy` runs `mika skills --agent mika-dev update`, it will re-seed all bundled skills including qa-review.

## Why This Works

The `qa-review` skill declares `[constraints] required_tools = ["qa_pr_view", "run_gh"]` and its handler restricts `run_gh` to a PR-review-only allowlist. When always-on (or keyword-triggered), this restriction applies to ALL `run_gh` calls in the turn — not just those initiated by qa-review. This is by design for mika-qa (which should only do PR operations), but catastrophic for mika-dev (which needs full `run_gh` access for issue listing, milestone queries, etc.).

The root issue is a deployment-time misconfiguration, not an engine bug. The skills system has no concept of "this skill is only valid on agent X" — any skill installed on an agent will load and enforce its constraints.

## Prevention

- Track mika#620 for the immediate removal fix
- Consider adding an optional `[skill] agents = ["mika-qa"]` field to `skill.toml` that `seed_bundled_skills` and `mika skills install` can validate against, preventing installation on non-matching agents
- When auditing agent turns that show repeated `run_gh` failures, check which skills are loaded — the `skill_name` column in `tool_calls` shows which skill's allowlist is being applied
- The `mika skills --agent <name> list` CLI command shows all installed skills per agent — use it to verify correct deployment

## Related Issues

- mika#620 — fix: remove qa-review skill from mika-dev agent
- mika#619 — fix: demote 'skills loaded' log from INFO to DEBUG (found in same audit session)
- mika#576 — skill-review fires on PRs that discuss skill-review (keyword false positive) — similar pattern of skill keywords matching meta-discussion
