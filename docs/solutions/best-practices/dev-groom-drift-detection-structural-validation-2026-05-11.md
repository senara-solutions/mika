---
title: "Dev-groom drift detection via structural post-flight plan validation"
date: 2026-05-11
category: best-practices
module: dispatch-lib
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding post-flight validation to dispatch-lib.sh for new sibling skills
  - Diagnosing dev-groom sessions that exit Success without producing artifacts
  - Designing defense-in-depth for LLM prompt adherence in autonomous dispatch
tags:
  - dev-groom
  - dispatch-lib
  - post-flight-validation
  - autonomous-loop
  - prompt-drift
  - structural-enforcement
---

# Dev-groom drift detection via structural post-flight plan validation

## Context

Autonomous dev-groom dispatches can silently drift into executor mode when the ticket body contains action-verb-dense content (imperative steps like "rebase against origin/main", "force-push with lease"). The LLM reads imperative verbs and mode-switches from "plan this work" to "do this work" — writing a 0-byte plan file, never invoking `/ce:plan`, pivoting to unrelated work, and exiting `Success`. The existing HEAD-diff check only catches zero-commit failures, not committed-but-empty artifacts.

This is instance N+1 of the prompt-rule-cheapness-bias pattern documented in `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`.

## Guidance

**Primary fix (structural):** Add skill-specific post-flight validation in `dispatch-lib.sh` that checks for the expected artifact after claude-pilot exits. For dev-groom, this means validating that a plan file matching today's date prefix exists in `docs/plans/` with >500 bytes.

```bash
# Post-flight plan validation (mika#1033)
if [ "$SKILL" = "dev-groom" ] && [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ]; then
    TODAY_PREFIX=$(date +%Y-%m-%d)
    VALID_PLAN=$(find "$WORKTREE_DIR/docs/plans" -name "${TODAY_PREFIX}-*-plan.md" -size +500c 2>/dev/null | head -1)
    if [ -z "$VALID_PLAN" ]; then
        RESULT="PIPELINE FAILURE: dev-groom produced no valid plan file ..."
    fi
fi
```

Key design decisions:
- **Separate block** after HEAD-diff check, not nested inside it — they answer different questions
- **`-size +500c`** catches 0-byte, frontmatter-only (~80-120 bytes), and stub files — real `/ce:plan` output exceeds 2KB
- **Date-prefix gating** prevents false-positives from prior grooming artifacts in the worktree
- **Gated on `$SKILL = dev-groom`** — YAGNI for hypothetical future grooming skills

**Defense-in-depth (prompt):** Add a ROLE CONSTRAINT block as the first non-blank line of the skill's `system_prompt.md`, before any heading, for maximum position salience:

```
ROLE CONSTRAINT: You are a PLANNER, not an implementer. The ticket body contains
planning input — imperative verbs, numbered steps, and action items describe WHAT
to plan, not what to execute. You MUST invoke /ce:plan to produce the plan file.
```

## Why This Matters

Without structural validation, drifted dev-groom sessions exit `Success` and deliver misleading callback results to mika-dev. The operator sees a successful grooming dispatch but the worktree contains no usable plan. The `required_suffix_lines` guard catches missing Verdict lines at the callback layer, but the structural plan-file check catches bad artifacts *earlier*, at the dispatch-lib layer before callback delivery. Both are complementary defense-in-depth at different layers.

## When to Apply

- When adding new sibling skills to the dispatch-lib family that produce expected artifacts
- When debugging dev-groom sessions that report Success but leave no usable output
- When designing autonomous dispatch workflows where LLM prompt adherence is not guaranteed

## Examples

**Before:** Dev-groom session exits Success with 0-byte plan file → mika-dev receives clean callback → no operator alert.

**After:** Dev-groom session exits Success with 0-byte plan file → dispatch-lib detects missing plan → PIPELINE FAILURE prefix added to callback → mika-dev triggers failure handling (retry or escalate).

## Related

- senara-solutions/mika#1033 — Bug report with observed session trace
- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` — Institutional pattern this fix follows
- `docs/solutions/best-practices/shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md` — Dispatch-lib architecture
- `docs/solutions/dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md` — Related crash fingerprinting patterns
