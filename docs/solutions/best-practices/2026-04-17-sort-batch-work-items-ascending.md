---
title: Sort batch work-item fetches ascending by issue number
date: 2026-04-17
category: best-practices
module: self-dev
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Fetching a batch of GitHub issues for sequential processing (milestone or project workflows)
  - Any workflow step that retrieves tasks and processes them in order
  - Adding new gh CLI or GraphQL queries that list issues for dispatch
tags:
  - milestone-workflow
  - project-workflow
  - self-dev
  - issue-ordering
  - jq
  - deterministic-ordering
---

# Sort batch work-item fetches ascending by issue number

## Context

The self-dev skill's milestone workflow (Step M2) and project workflow (Step P2) fetch batches of GitHub issues and process them sequentially. `gh issue list` returns issues in **descending** created order by default (newest first). GitHub's GraphQL ProjectV2 API returns items in project-board order, which is non-deterministic.

When child issues have dependency ordering — earlier issues are prerequisites for later ones — processing newest-first breaks the dependency chain. This was caught when milestone #12 was about to be dispatched: issue #630 depends on #629, but the default ordering would queue #630 first.

## Guidance

Always add explicit ascending sort by issue number when fetching batches of tasks for sequential processing.

**Milestone workflow (gh CLI):**

```bash
# Before — relies on gh's default descending order
run_gh issue list --milestone <n> --repo senara-solutions/<repo> --state open --json number,title --jq '.[].number'

# After — explicit ascending sort
run_gh issue list --milestone <n> --repo senara-solutions/<repo> --state open --json number,title --jq 'sort_by(.number) | .[].number'
```

**Project workflow (GraphQL):**

```bash
# Before — relies on non-deterministic project-board order
--jq '.data.organization.projectV2.items.nodes[].content | select(.state == "OPEN") | "\(.repository.name)#\(.number)"'

# After — collect into array, sort, then emit
--jq '[.data.organization.projectV2.items.nodes[].content | select(.state == "OPEN")] | sort_by(.number) | .[] | "\(.repository.name)#\(.number)"'
```

## Why This Matters

Issue numbers are monotonically increasing and match creation order. Sorting ascending by number provides:

1. **Dependency safety** — earlier issues (prerequisites) run before later ones that depend on them
2. **Determinism** — the same milestone always produces the same processing order regardless of API pagination or server-side ordering changes
3. **Predictability** — operators can reason about execution order by looking at issue numbers

Without explicit sorting, the workflow silently relies on an API default that can change between gh CLI versions or GitHub API updates.

## When to Apply

- Adding any new `gh issue list` or GraphQL query that fetches multiple issues for sequential processing
- Any batch work-item dispatch step in self-dev, qa-review, or other orchestration skills
- Whenever the processing order of tasks matters for correctness

## Examples

The fix applied to `skills/bundled/self-dev/system_prompt.md` adds `sort_by(.number)` to both the milestone (Step M2) and project (Step P2) jq filters. The output format is unchanged — only the ordering is corrected.

## Related

- GitHub issue: #632
- Related incident: #630 depends on #629, milestone #12 would have broken with default ordering
- Related milestone/project workflow: `skills/bundled/self-dev/system_prompt.md` Steps M1–M5, P1–P5
