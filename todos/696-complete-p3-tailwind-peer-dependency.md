---
status: pending
priority: p3
issue_id: 696
tags: [code-review, quality]
dependencies: []
---

# Add tailwindcss as Peer Dependency to UI Package

## Problem Statement

Extracted UI components use Tailwind utility classes and custom theme tokens (`bg`, `bg-card`, `accent`, etc.) but `packages/ui/package.json` does not list `tailwindcss` as a peer dependency. This creates an implicit, undocumented contract that any consumer must have Tailwind CSS v4 configured with matching theme tokens.

## Proposed Solutions

### Option A: Add tailwindcss to peerDependencies
- Add `"tailwindcss": "^4.0.0"` to peerDependencies
- Document required theme tokens in README or package.json description
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] `tailwindcss` listed as peer dependency
- [ ] Required theme tokens documented

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-17 | Created from code review of PR #193 | |
