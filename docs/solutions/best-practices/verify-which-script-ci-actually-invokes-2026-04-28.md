---
title: "Verify which script CI actually invokes, not which script you found by grep"
date: 2026-04-28
category: best-practices
module: ci, cross-repo, debugging
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - Debugging a CI failure where the script being invoked is one of multiple same-named copies across repos in a workspace
  - Reading a script's source to predict CI behavior, when the workspace has a meta-repo + sub-repos that each carry their own copy
  - Proposing a fix to a CI script after reading "the" script, when in fact you read one copy and CI runs another
  - Adding a new escape hatch / mechanism to one repo's script and assuming the same mechanism exists in sibling repos
  - Multi-repo workspaces where utility scripts are vendored rather than referenced from a canonical location
tags:
  - cross-repo
  - script-drift
  - ci-investigation
  - verify-pipeline
  - assumption-check
  - dry-violation-detection
  - mika-platform
---

# Verify which script CI actually invokes, not which script you found by grep

## Context

On 2026-04-28, while debugging a CI failure on `senara-solutions/mika#860` (a docs-only PR shipping two compound docs), I claimed the failure could be unblocked by adding a `Pipeline-Exempt: docs-only — <reason>` commit trailer. I had read `verify-pipeline.sh`'s source, found the bucket-comparison logic and the trailer-parsing block at lines 99-104, and confidently asserted the trailer would land it green.

The trailer commit pushed. CI re-ran. It failed again with the same error: `MISSING: No plan doc in docs/plans/. Run /ce:plan.`

The string `"MISSING: No plan doc in docs/plans/. Run /ce:plan."` is from an *unconditional* check at line 34. The bucket logic and trailer parsing I had read were not in that check. They were not in the same file at all.

Direct cause: there are two scripts named `verify-pipeline.sh`. One lives at `mika-platform/scripts/verify-pipeline.sh`. The other lives at `mika/scripts/verify-pipeline.sh`. They are independent files. They do not share content. They drifted.

`mika/.github/workflows/ci.yml:130` runs `bash scripts/verify-pipeline.sh origin/main`. The path is relative to the repo's root, so the *mika* copy is what CI executes — not the mika-platform copy I had been reading.

The mika-platform copy had been upgraded per `mika-platform#18` (CLOSED 2026-04-08) to use bucket-comparison logic with a `Pipeline-Exempt:` trailer escape hatch. The mika copy — last touched at commit `b4aaed3f` — still had the original three unconditional checks (plan doc presence, source change presence, compound doc presence), with no escape hatch.

I had read the wrong file. The trailer commit was decoration. The fix that actually unblocked the PR was porting the mika-platform script verbatim onto mika (PR #860 commit `e7fe3ee7`).

## The Pattern

In a workspace with multiple related repos that each vendor their own copy of a "shared" utility script, **the file you find by grep is not necessarily the file CI executes**. Verifying which script CI actually invokes is a discipline, not a default.

Three concrete failure modes follow from this:

1. **Reading-the-wrong-copy.** As above. `grep -rln <pattern>` returns the first match in the workspace; the first match is rarely the canonical location, especially in a meta-repo + sub-repo layout where the meta-repo's copy is often the *most-edited* but not the *most-invoked*.

2. **Patching-the-wrong-copy.** Even when the mistake is recognized at code-edit time, the muscle memory of "I read script X, I'll edit script X" propagates the read-the-wrong-copy error into the diff. The fix lands on the file that CI doesn't run.

3. **Documenting-the-wrong-copy.** Compound docs and tickets cite "the script" by relative path or function name. When the workspace has two copies, the citation is ambiguous unless the doc explicitly names the repo. mika#861 (the label-inheritance design ticket) initially cited `scripts/verify-pipeline.sh` without specifying which repo's copy — a recurrence of the same ambiguity that caused the original bug.

## The Discipline

Before claiming any CI behavior:

1. **Find the workflow file CI uses.** `gh run view <run_id> --log-failed` shows the workflow path and step name. Trace from there to the workflow YAML. Read the actual `run:` line — it runs commands against the *checked-out repo's filesystem*, not against any other repo in the workspace.

2. **Resolve script paths against the workflow's repo, not your search path.** When `ci.yml` says `bash scripts/verify-pipeline.sh`, "scripts/" is relative to the repo root that the workflow checked out (per `actions/checkout@vN`). If you're debugging from a meta-repo workspace, that repo root is *the sub-repo*, not the meta-repo.

3. **Diff sibling copies before assuming behavior parity.** If a workspace has multiple repos that carry the same utility script, running `diff <repo-A>/scripts/X.sh <repo-B>/scripts/X.sh` is a 5-second check that surfaces drift before it surprises you. Make this a reflex when working across repos.

4. **Cite the script by full repo-prefixed path in docs and tickets.** `mika/scripts/verify-pipeline.sh` and `mika-platform/scripts/verify-pipeline.sh` are different files; `scripts/verify-pipeline.sh` alone is ambiguous in a multi-repo workspace. Tickets that propose changes should always specify which repo's copy they target.

## Why This Matters Beyond CI Scripts

The pattern generalizes to any utility that's vendored across repos: pre-commit hooks, dispatcher scripts, validation tooling, compound-doc index generators. The shape is always the same — a "shared" utility that isn't actually shared, just copied. The DRY violation is real but often invisible until one copy diverges and CI starts behaving differently between repos.

Two structural fixes worth considering when this drift class becomes load-bearing:

- **Single canonical location with vendoring discipline.** Keep one copy in the meta-repo or in a designated "shared scripts" location; sub-repos either symlink to it or vendor it via a CI step that re-syncs from canonical. Drift becomes detectable as a CI lint failure rather than a silent runtime divergence.

- **CI lint that diffs sibling copies.** A meta-repo CI job that runs `diff` between sibling utility scripts and fails when they drift. Catches regressions at PR time rather than at next-debugging-cycle time. Cheap; high signal.

Neither is in scope for this doc. The doc's job is to name the discipline; the structural fixes are downstream.

## Application

When debugging CI:

- First action: `gh run view <run_id> --log-failed` to see the actual error string and the workflow step that produced it
- Trace the workflow step to the `run:` line; trace the `run:` line to the script; resolve the script path against the *checked-out repo's root*, not your IDE's search path
- If you have multiple repos in the workspace, diff sibling copies of any script you're about to read or edit
- When proposing a fix, name the script by repo-prefixed path in the PR/ticket description

When authoring docs and tickets:

- Cite scripts with their full repo prefix: `mika/scripts/verify-pipeline.sh`, not `scripts/verify-pipeline.sh`
- When a ticket proposes a behavior that depends on a specific script's logic, link to a permalink (commit-pinned URL) of that specific script, not just the path

When reviewing PRs:

- If the PR claims to fix a CI behavior by editing a script, verify the edited script is the same file CI invokes
- If the PR cites "the verify-pipeline.sh logic" or similar, ask which repo's copy

## Citations

- `senara-solutions/mika#860` — the docs-only PR whose CI failure surfaced the drift. Initial trailer commit (`bdb15156`) was decoration; actual fix was the script port (`e7fe3ee7`).
- `senara-solutions/mika-platform#18` (CLOSED 2026-04-08) — the canonical ticket that built bucket-comparison + trailer logic on the mika-platform copy. Title: "verify-pipeline.sh: reject docs-only and code-only PRs."
- `senara-solutions/mika#861` (OPEN) — label-inheritance design ticket that initially cited the script ambiguously; updated post-PR-#860 to clarify which repo's copy is in scope.
- `mika/.github/workflows/ci.yml:117-130` — Pipeline Artifacts job that runs `bash scripts/verify-pipeline.sh origin/main` against the *mika* repo's copy.
- `feedback_evidence_before_diagnosis.md` (mika-platform memory) — meta-rule: "When an agent's response looks wrong, query session/DB before proposing fixes; symptom→options is an anti-pattern." This doc is a specific instance of the same pattern applied to script source.
- `feedback_compound_infra_fixes.md` (mika-platform memory) — infra fixes evaporate fast; compound them. This doc compounds today's specific drift incident.
