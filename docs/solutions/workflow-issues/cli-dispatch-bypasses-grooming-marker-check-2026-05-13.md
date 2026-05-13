---
title: "Operator-CLI dispatch paths bypass grooming-marker check — engine-level guard at validate_dispatch_readiness"
date: 2026-05-13
category: workflow-issues
module: skills/executor
problem_type: workflow_issue
component: development_workflow
severity: high
applies_when:
  - Adding new dispatch surfaces for `run_claude_pilot` (CLI, webhook, sprint, free-text)
  - Modifying the grooming-marker callout shape in `/mika-groom-ticket`
  - Extending `validate_dispatch_readiness()` with new checks
  - Changing the `dev-pilot` or `dev-groom` skill contracts
tags:
  - dispatch
  - grooming
  - engine-guard
  - validate-dispatch-readiness
  - dev-pilot
  - cli-dispatch
  - run-claude-pilot
  - defense-in-depth
---

# Operator-CLI dispatch paths bypass grooming-marker check

## Context

mika#907 shipped a grooming-marker check to prevent ungroomed tickets from reaching `claude-pilot`. However, the check was **prompt-level** — wired into the self-dev skill's webhook ready-label handler only. The CLI dispatch path (`mika ask --agent mika-dev "implement..."`) is a parallel surface that produces the same effect (claude-pilot starts on a ticket) but never traversed the grooming-marker check.

The mika#908 sprint on 2026-05-01 demonstrated the gap: four tickets shipped in a single CLI-sprint with zero grooming on any of them.

## Guidance

**Structural enforcement (engine-level) trumps prompt-level rules.** Per `feedback_prompt_enforcement_fragile.md`, prompt-level checks drift under LLM load. The engine-level guard at `validate_dispatch_readiness()` fires uniformly for all dispatch paths.

The fix (mika#919) adds a grooming-marker check as the 5th gate in `validate_dispatch_readiness()`, positioned after per-class dispatch slot checks and before the blocked-by check. The guard:

1. Fetches the issue body via `fetch_issue_body()` (REST API, same auth shape as existing helpers)
2. Checks three canonical grooming-marker substrings:
   - `> - **Branch:**` — branch callout
   - `docs/plans/` — plan path prefix
   - `second-pass (GROOMED)` — architect verdict
3. Rejects with `dispatch_no_grooming_marker` error code if any are missing

**Bypass predicates** (check skips entirely):
- Skill is not `dev-pilot` (dev-groom is the marker producer)
- Task type is not `issue` (milestones/projects don't carry plans)
- `reference_url` doesn't parse as a GitHub issue
- `MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1` env var (WARN-logged emergency override)

**Failure policy** mirrors mika#713 (blocked-by check):
- No GitHub token → fail-open with WARN
- Token present but API error → fail-closed

## Why This Matters

The entire point of mika#907 was to make ungroomed dispatch "structurally impossible." With only the webhook path gated, any operator CLI dispatch silently bypassed the architect review requirement. The engine-level guard closes this asymmetry — a single gate, single failure mode, no skill drift.

## When to Apply

- **Coupled pair:** The prompt-level check at `skills/bundled/self-dev/system_prompt.md:253` is defense-in-depth. Both the engine guard and the prompt check must update together if the canonical `/mika-groom-ticket` Phase 5 callout shape changes.
- **Predicate drift:** If the three load-bearing substrings (`> - **Branch:**`, `docs/plans/`, `second-pass (GROOMED)`) ever change in `/mika-groom-ticket`, update both `check_grooming_markers()` in `executor.rs` and the prompt-level check.

## Examples

### Rejection (ungroomed ticket)

```json
{
  "error": "dispatch_no_grooming_marker",
  "task_id": "26431ba6-...",
  "issue": "senara-solutions/mika#908",
  "missing_signals": ["branch_callout", "plan_callout", "groomed_verdict"],
  "recovery": "Run /mika-groom-ticket <ref> or set MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1"
}
```

### Success (groomed ticket)

Issue body contains all three signals → guard passes, dispatch proceeds.

## Related

- mika#907 — original prompt-level grooming check (webhook path only)
- mika#910 — `webhook_no_unauthorized_dispatch` engine guard (sibling pattern)
- mika#996 — auto-groom-on-dispatch (interacts at prompt level)
- `docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — related: branch callout requirement for plan resumption
