---
status: pending
priority: p2
issue_id: 692
tags: [code-review, quality]
dependencies: []
---

# Remove Orphaned react-markdown/remark-gfm from Dashboard Dependencies

## Problem Statement

Both `dashboard/package.json` and `packages/ui/package.json` list `react-markdown` and `remark-gfm` as direct dependencies. Since `MarkdownContent` was moved to the UI package, these dependencies in the dashboard are now orphaned — no dashboard code directly imports them. The duplication is misleading about where the dependency is actually used.

## Findings

- `dashboard/package.json` lists `react-markdown` and `remark-gfm`
- `packages/ui/package.json` also lists them (correctly, since `MarkdownContent` lives there)
- No dashboard file directly imports from `react-markdown` or `remark-gfm` after the extraction
- npm workspaces hoisting may cause no build issues, but the explicit listing is incorrect

## Proposed Solutions

### Option A: Remove from dashboard/package.json
- Remove `react-markdown` and `remark-gfm` from `dashboard/package.json`
- Run `npm install` to update lockfile
- **Pros:** Correct dependency declarations
- **Cons:** None
- **Effort:** Small
- **Risk:** None

## Recommended Action

*(To be filled during triage)*

## Technical Details

- **Affected files:** `dashboard/package.json`

## Acceptance Criteria

- [ ] `react-markdown` and `remark-gfm` only listed in `packages/ui/package.json`
- [ ] Dashboard build still succeeds

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-17 | Created from code review of PR #193 | |

## Resources

- [PR #193](https://github.com/senara-solutions/mika/pull/193)
