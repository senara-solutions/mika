---
status: pending
priority: p3
issue_id: 697
tags: [code-review, quality]
dependencies: []
---

# Update Stale MIKA_ANTHROPIC_API_KEY References in Active Docs/Todos

## Problem Statement

30+ files under `docs/`, `todos/`, and `docs/solutions/` still reference `MIKA_ANTHROPIC_API_KEY`. Completed todos and historical plans are fine (they record point-in-time decisions), but pending todos and active solution docs used as reference material should be updated.

## Key Files to Update

- `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md`
- `docs/solutions/integration-issues/custom-skill-silent-loading-failure.md`
- `docs/solutions/integration-issues/shell-exec-jq-json-parsing.md`
- `todos/607-pending-p3-setup-tty-guard.md`
- `todos/613-pending-p3-non-interactive-setup-flags.md`

## Acceptance Criteria

- [ ] Active/pending docs reference `MIKA_LLM_API_KEY`
- [ ] Historical/completed docs left as-is

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-17 | Created from code review of PR #193 | |
