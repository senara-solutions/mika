---
title: "Cross-repo issue scope drift after upstream merge"
category: prompt-engineering
date: 2026-04-08
severity: medium
module: skill-review
tags: [cross-repo, issue-scope, builtin-handlers, required-tools, dev-loop]
pr: senara-solutions/mika#485
relocated_from: senara-solutions/mika-skills#102
---

> **Relocation note (2026-04-08):** This solution doc originally shipped on `mika-skills` via PR #102 together with the plan doc, with zero source changes — the code landed separately in `mika#485`. The split was itself an instance of the very drift this document describes. Relocated to the `mika` repo so the learning lives with the implementation.

## Problem

Issue `mika-skills#99` specified switching `skill-review` from `write_agent_file` to `write_skill_variant` and adding the latter to `required_tools`. By the time mika-dev started implementing it, upstream `mika#477` had already merged `write_skill_variant` back into `review_skill` as a unified tool. The target tool name no longer existed as a registered builtin in `KNOWN_BUILTINS`, making the downstream issue spec incorrect.

Worse, the skill-review **source files do not live in `mika-skills`** — they live in `mika/crates/mika-agent/templates/skills/skill-review/` as bundled skill templates. The ticket was filed against the wrong repo entirely. mika-dev discovered this mid-implementation when a `write_agent_file` call was sandbox-rejected for writing outside the current worktree.

The result was a botched cross-repo split: the real code rename shipped in `mika#485` (with a failing `Pipeline Artifacts` check and no plan/solution docs), while the plan and solution docs orphaned themselves in a separate `mika-skills#102` PR that contained zero source changes. Two PRs, two repos, neither of them self-consistent.

## Root Cause

Two independent drifts stacked:

1. **Companion-issue staleness.** `mika#469` originally planned `write_skill_variant` as a separate builtin, and `mika-skills#99` was filed as its downstream companion. `mika#477` then merged `write_skill_variant` into `review_skill`, but `mika-skills#99` was never updated to reflect the new reality.
2. **Repo mis-routing.** `skill-review` is a bundled skill template — it lives in `mika` core, not in the `mika-skills` marketplace repo. The issue was filed against the wrong repo by reflex ("it's a skill → mika-skills"). The keyword inference rules in `.claude/commands/mika.md` encode the same mistake: anything with "skill" in the title routes to `mika-skills` by default, even though bundled skill templates live in `mika`.

When mika-dev hit the sandbox rejection mid-run, she correctly identified the right repo but made the wrong structural choice: rather than re-routing all artifacts (plan + solution + code) into a single PR on `mika`, she split them — shipping code in `mika#485` and docs in `mika-skills#102`. This left both PRs invalid as pipeline outputs and triggered a second failure when `mika#485` merged with a failing `Pipeline Artifacts` check.

## Solution

### Immediate fix (what shipped)

The only real remaining code change was a one-line fix to the stale `write_agent_file` description in `tools.json` plus a redundant prohibition line in `system_prompt.md`, both under `crates/mika-agent/templates/skills/skill-review/`. Details in the companion plan doc: `docs/plans/2026-04-08-002-fix-skill-review-write-agent-file-stale-ref-plan.md`. Landed via `mika#485`.

### Process fix (how to avoid this next time)

**Before implementing a companion issue**, always verify the upstream state:

1. Check whether the upstream PR or issue is still open or was superseded. `gh pr list --search "<feature>" --state merged` on the upstream repo.
2. Verify any tool names referenced in the issue exist in `KNOWN_BUILTINS` — grep `crates/mika-agent/src/skills/builtin_handlers.rs` for the name.
3. If the upstream scope changed, re-scope the downstream issue *before* coding. Leave a comment on the downstream issue linking the upstream merge.

**When the ticket is filed on the wrong repo**, the agent must re-route *all* artifacts, not split them:

1. Stop work in the current worktree.
2. Open a new worktree on the correct repo with the same branch name.
3. Move the plan doc, solution doc, **and** the source change into a **single** PR on the correct repo.
4. Do **not** open a separate docs-only PR on the original repo as a consolation prize.
5. Comment on the original issue explaining the re-routing before closing or refiling.

This rule is being added to the `self-dev` skill prompt as part of `mika-platform#17`.

### Structural fix (make it impossible to re-occur)

1. **`verify-pipeline.sh`** should reject PRs containing only docs (under `docs/plans/` or `docs/solutions/`) with no source changes — and reject PRs containing only source changes with no corresponding plan/solution docs. Explicit opt-out via a commit trailer for the small number of legitimately doc-only or code-only changes. Tracked in `mika-platform#17` section C.
2. **Branch protection** must require the `Pipeline Artifacts` status check on all four repos so a PR with the wrong file set cannot merge even if the agent calls `gh pr merge`. Tracked in `mika-platform#17` section A.
3. **mika-dev's merge logic** must read the CI status rollup before calling merge. On any required-check `FAILURE`, block and transition the task. On `PENDING`, use `gh pr merge --auto` so GitHub completes the merge when checks pass. Tracked in `mika-platform#17` section B.

## Prevention

When filing cross-repo companion issues, add a pre-flight check to the issue body:

> Before starting, verify that `<tool_name>` exists in `KNOWN_BUILTINS` at `crates/mika-agent/src/skills/builtin_handlers.rs` and that the skill source actually lives in the repo where this issue is filed. If either has changed, re-scope or re-file before coding.

And before filing: **if the "skill" is a bundled skill template (shipped inside `mika`), file the ticket on `mika`, not `mika-skills`.** The marketplace repo is for community/external skills, not bundled ones.

## References

- Landed PR: `senara-solutions/mika#485`
- Original wrong-repo docs PR (relocated): `senara-solutions/mika-skills#102`
- Superseding merge: `senara-solutions/mika#477`
- Companion plan: `docs/plans/2026-04-08-002-fix-skill-review-write-agent-file-stale-ref-plan.md`
- Umbrella postmortem: `senara-solutions/mika-platform#17`
- Related feedback memory: `feedback_qa_advisory_ci_gate_on_dev` — CI gate lives on mika-dev's merge logic, not on QA
