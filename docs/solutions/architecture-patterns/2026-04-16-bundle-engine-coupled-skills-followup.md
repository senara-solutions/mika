---
title: Engine-coupled skill migration — follow-up pass to catch missed skills
date: 2026-04-16
category: architecture-patterns
status: applied
---

# Engine-coupled skill migration — follow-up pass to catch missed skills

## What

Second migration batch after [the initial bundle-engine-coupled-skills PR](2026-04-16-bundle-engine-coupled-skills.md). The first pass enumerated 11 skills; a careful re-read of `mika-skills/` turned up 3 more that also pass the engine-coupled test but were initially overlooked:

- `qa-review-build-callback`
- `self-dev-iterate`
- `address-pr-comments`

## The compoundable lesson

When applying "the boundary should encode atomic-change requirements" to a real codebase, **one pass is rarely enough**. The ambiguous cases reveal themselves only after the easy migrations land and you can see which leftover skills still cause cross-repo drift pain.

Practical rule for future similar migrations: **after the first pass, re-apply the boundary test to each remaining item in the source repo**. The easy cases were migrated because they were obvious; the ambiguous cases remain because their engine-coupling was subtle. That's exactly where the next drift incident will come from.

In this case the three missed skills share a specific pattern: they all *launch or respond to* a claude-pilot session on behalf of self-dev. None of them directly consume a Rust-side tool contract the way e.g. self-dev does, but they depend on the same dispatch shape that self-dev codifies. When that dispatch contract changes (as it did for mika#595 → mika#601), these three need to update in lockstep — exactly the drift class bundling eliminates.

## Procedural note

The earlier compound doc ([2026-04-16-bundle-engine-coupled-skills.md](2026-04-16-bundle-engine-coupled-skills.md)) is the canonical statement of the pattern. This doc exists to capture the specific procedural learning: **always do a follow-up pass** after the initial boundary migration. Budget it in the original plan rather than discovering the need for it later.
