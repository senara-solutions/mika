---
module: skills/bundled/dev-groom
tags: [branch-derivation, canonical-script, slug-drift, single-source-of-truth]
problem_type: workflow-issues
category: skill-prompt-maintenance
date: 2026-05-02
issue: 929
---

# Skill prompts must invoke canonical scripts, not re-implement derivation recipes

## Problem

`skills/bundled/dev-groom/system_prompt.md` re-derived branch slugs in-prompt using its own recipe (`<type>/<n>/<sanitized-title>`) instead of invoking `scripts/derive-branch-name`. This caused slug drift between dev-groom-groomed tickets and operator-side dispatch paths (`/mika`, `/mika-groom-ticket`, dev-pilot) for the same ticket.

Concrete example: mika#927 produced a ~95-char slug via dev-groom's recipe vs a 40-char-bounded slug from the canonical script.

## Root Cause

The dev-groom skill was authored before the meta-repo mandate ("Every dispatch path that needs to derive a branch name MUST invoke the canonical script") was fully enforced. The in-prompt derivation lacked the script's truncation (40 chars), priority ordering, and sanitization edge-case handling.

## Fix

Replaced the in-prompt derivation block with explicit invocation of `scripts/derive-branch-name`, preserving:
- Body-callout-takes-priority semantics (script is NOT invoked when callout matches)
- Slug-immutability after worktree creation

## Lesson

When a skill prompt needs to compute a load-bearing value that other dispatch paths also compute, the prompt must invoke the canonical script — not re-implement the recipe inline. This eliminates drift when the recipe evolves (truncation tuning, new label mappings, priority order changes).

**Detection pattern:** `grep -rn` for recipe fragments (label→type mappings, sanitization regex, truncation logic) in `skills/bundled/*/system_prompt.md`. If found outside a script invocation, it's a drift risk.

## Verification

After deploy: dispatch the same ticket via dev-groom AND `/mika-groom-ticket` operator-side. Both must produce identical branch slugs.
