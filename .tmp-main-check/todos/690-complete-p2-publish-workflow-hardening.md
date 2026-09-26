---
status: pending
priority: p2
issue_id: 690
tags: [code-review, security, ci]
dependencies: []
---

# Harden publish-ui.yml Workflow

## Problem Statement

The `publish-ui.yml` workflow has two issues: (1) it runs `npm publish` on every push to main touching `packages/ui/**` without checking for a version bump — npm registries reject duplicate versions, causing CI failures, and (2) GitHub Actions are referenced by tag (`@v4`) instead of commit SHA, violating the project's established convention from `ci.yml`.

## Findings

- `ci.yml` pins all actions to commit SHAs (project convention documented in CLAUDE.md)
- `publish-ui.yml` uses `actions/checkout@v4` and `actions/setup-node@v4` (tag refs)
- No version-bump check before `npm publish` — will fail if version unchanged
- Permissions are appropriately scoped (`contents: read`, `packages: write`)
- `publishConfig.registry` correctly points to GitHub Packages (no accidental public publish)

## Proposed Solutions

### Option A: Pin actions + add version-change check
- Pin actions to commit SHAs with version comments
- Add a step that compares `package.json` version against the published registry version
- Skip publish if version unchanged
- **Pros:** Robust, follows conventions, prevents CI failures
- **Cons:** Slightly more complex workflow
- **Effort:** Small
- **Risk:** Low

### Option B: Pin actions only, defer version automation
- Pin actions to SHAs
- Accept that publish will fail on duplicate versions (developers must bump manually)
- **Pros:** Minimal change
- **Cons:** CI failures on unchanged versions
- **Effort:** Small
- **Risk:** Low

## Recommended Action

*(To be filled during triage)*

## Technical Details

- **Affected files:** `.github/workflows/publish-ui.yml`
- Current SHA pins for reference:
  - `actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683` (v4.2.2)
  - `actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020` (v4.4.0)

## Acceptance Criteria

- [ ] Actions pinned to commit SHAs with version comments
- [ ] Workflow handles unchanged versions gracefully (skip or check)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-17 | Created from code review of PR #193 | |

## Resources

- [PR #193](https://github.com/senara-solutions/mika/pull/193)
- `.github/workflows/ci.yml` (reference for SHA pinning convention)
