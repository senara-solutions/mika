---
status: pending
priority: p3
issue_id: 693
tags: [code-review, quality]
dependencies: []
---

# Remove Dual Declaration Generation in UI Package Build

## Problem Statement

`packages/ui/package.json` build script runs both `vite build` (which uses `vite-plugin-dts` for declaration generation) and `tsc --emitDeclarationOnly --declaration --outDir dist`. Both produce `.d.ts` files — running both is redundant. Pick one approach.

## Findings

- `vite.config.ts` includes `vite-plugin-dts` plugin
- `package.json` build script: `vite build && tsc --emitDeclarationOnly --declaration --outDir dist`
- Both produce `.d.ts` files in `dist/`

## Proposed Solutions

### Option A: Keep tsc, remove vite-plugin-dts
- Remove `vite-plugin-dts` from devDeps and vite.config.ts
- Keep the `tsc` step in the build script
- **Pros:** `tsc` is the standard, more predictable
- **Effort:** Small

### Option B: Keep vite-plugin-dts, remove tsc step
- Remove `&& tsc --emitDeclarationOnly ...` from build script
- **Pros:** Simpler build command
- **Effort:** Small

## Acceptance Criteria

- [ ] Only one declaration generation method
- [ ] `dist/` still contains correct `.d.ts` files

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-17 | Created from code review of PR #193 | |
