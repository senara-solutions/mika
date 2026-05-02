---
title: PR-writer must cite plan-doc paths in PR body for CI hook compliance
date: 2026-05-02
category: workflow-issues
module: self-dev, mika-platform
problem_type: workflow_issue
component: development_workflow
severity: medium
applies_when:
  - Adding a plan-doc-check CI hook that validates PR bodies contain literal docs/plans/*.md paths
  - The /mika pipeline produces plan docs but the PR-writer step does not cite them
  - Any prompt-driven PR body composition where a CI hook expects specific path strings
tags:
  - plan-doc-check
  - pr-body
  - ci-hook
  - prompt-instruction
  - pipeline-compliance
---

# PR-writer must cite plan-doc paths in PR body for CI hook compliance

## Context

The `/mika` pipeline produces plan docs at step 1 (`/ce:plan`) and creates PRs at step 8, but the step-8 instructions only said "include `Closes #<number>`" -- nothing about citing the plan-doc path. The `plan-doc-check` CI hook (mika-platform#64) uses `grep -oE 'docs/plans/[^[:space:]]+\.md'` to find plan-doc citations in the PR body and commit bodies. Because the agent summarized plans in prose rather than citing literal paths, the hook rejected every `/mika` PR that added a plan doc.

Evidence: mika-platform#72 -- PR body described the plan in prose ("Sub-repo verification", "Review findings addressed"), never cited `docs/plans/2026-05-02-001-feat-workspace-readme-plan.md`. Hook fired `[plan-doc-check: none] REJECT`. Merged via bypass actor, defeating the gate's intent.

## Guidance

When a CI hook checks for literal file paths in PR bodies, the prompt instruction that composes the PR body must:

1. **Detect** which files match the hook's expected pattern (e.g., `git diff --name-only main...HEAD | grep '^docs/plans/.*\.md$'`)
2. **Cite** each matching path literally in the PR body (e.g., `Plan: docs/plans/<file>.md`)
3. **Prohibit prose substitution** explicitly -- agents default to summarizing rather than citing paths

The instruction was added as a `**Plan-doc citation (MANDATORY):**` paragraph in `.claude/commands/mika.md`, following the same imperative style as the existing `Closes #<number>` instruction.

## Why This Matters

Structural enforcement (CI hooks) is the correct layer for pipeline compliance -- but the enforcement only works if the writer knows what to emit. A prompt-instruction gap between what the hook checks and what the writer produces creates a silent bypass: the pipeline does the right thing (plan exists in diff), the hook does the right thing (checks for the path), but the writer omits the citation and the hook fires.

This is the complementary half of the structural enforcement documented in `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`. That doc established that CI hooks are the right enforcement layer; this doc establishes that the writer must be taught to satisfy the hook.

## When to Apply

- Adding a new CI hook that validates PR body content against file paths in the diff
- Modifying an existing CI hook's grep pattern -- the corresponding prompt instruction must be updated in the same PR
- Propagating a CI hook to new repos -- ensure the writer instruction exists in each repo's `/mika` command before enabling the hook

## Examples

**Before (broken):** PR body contains prose like "Added implementation plan for the feature" -- `grep -E 'docs/plans/[^[:space:]]+\.md'` finds nothing, hook rejects.

**After (fixed):** PR body contains `Plan: docs/plans/2026-05-02-007-fix-pr-writer-plan-doc-citation-plan.md` -- grep matches, hook passes.

The instruction in `.claude/commands/mika.md`:
```
**Plan-doc citation (MANDATORY):** Before composing the PR body, run
`git diff --name-only main...HEAD | grep '^docs/plans/.*\.md$'`. For each
matching path, include a line in the PR body citing the literal path.
```

## Related

- senara-solutions/mika#931 -- originating issue
- senara-solutions/mika-platform#72 -- failing example (bypass-merged)
- senara-solutions/mika-platform#62 / mika-platform#64 -- hook creation
- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` -- structural enforcement as the correct layer
- `docs/solutions/ci-cd/compound-doc-enforcement-in-pipeline-verification.md` -- same pattern for compound-doc enforcement
