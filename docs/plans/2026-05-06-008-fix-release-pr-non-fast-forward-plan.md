---
title: "fix(ci): make the release-pr push idempotent — Class C resolution"
type: fix
status: active
date: 2026-05-06
---

# fix(ci): make the release-pr push idempotent — Class C resolution

## Overview

The `release-pr` job in `.github/workflows/release-plz.yml` (file retained for backward-compat; tool is git-cliff since `4825e7ae`) has failed on every merge to `main` since at least 2026-04-23 with `! [rejected] release/v0.6.0 -> release/v0.6.0 (non-fast-forward)`. The root cause is that the job creates a new local commit on `release/v0.6.0` from the latest `main` HEAD on every run, but the remote branch already exists from a prior failed run (without an associated PR), so `git push` is rejected as non-fast-forward. The job's "Check for existing release PR" step only probes for an *open PR* — it never sees the orphan branch, so it never recovers.

This plan implements the Class C resolution from `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`: **recreate `release/vX.Y.Z` from `main` every run** (approach 3 in that doc's Class C table), making the push idempotent with respect to whatever state happens to be on the remote branch.

The pre-flight orphan cleanup (`git push origin --delete release/v0.5.0 release/v0.5.1 release/v0.6.0`) was already executed before this ticket was dispatched. The plan does not include orphan cleanup — that was a one-shot operational unblock.

## Problem Frame

**The class:** Release-branch state management (Class C in the chronic-drift taxonomy). The release PR branch's state diverges from what the workflow expects to push, producing non-fast-forward rejections. This class has had only one prior fix (`b3fc1f44` — a scope-exclusion that kept the broken branch from blocking *other* workflows, not a root-cause fix).

**Why it keeps recurring:** the workflow's existing-state check is `gh pr list --head <branch>` (open PRs only). When a prior run pushes the branch but fails before opening the PR (or the PR is closed without merging), the check returns empty, the job tries to recreate the local branch from the new `main` HEAD, and `git push` rejects because the remote branch has a divergent commit from the prior failed run. Every subsequent merge to `main` reproduces the same failure.

**Why approach 3 (recreate-from-main) over the alternatives** (per the chronic-drift doc's Class C table):
- The release branch has **no meaningful history worth preserving** — every commit on it is regenerated from main HEAD + version bump + `cargo update` + `git-cliff` output.
- Approach 1 (rebase before push): doesn't survive concurrent runs — second queued run still has stale local.
- Approach 2 (force-push-with-lease): preserves "history" the branch doesn't have, and can fail spuriously on concurrent merges where two queued runs each force-push.
- Approach 3 (recreate from main + plain `--force`): every run is independent, the branch state is always deterministic from `(main HEAD, NEXT version)`, no lease/rebase machinery needed.

## Requirements Trace

From mika#775's acceptance criteria:

- **R1.** Root cause identified and documented — covered in this plan's Problem Frame and the Stage 3 section added to the chronic-drift compound doc (Unit 3).
- **R2.** Fix makes `release-pr`'s push idempotent — Unit 1 replaces the existing-PR-then-create-branch-then-push flow with always-recreate-then-push-then-conditionally-create-PR.
- **R3.** Survives concurrent merges — `concurrency: { group: release-pr, cancel-in-progress: false }` (already present, lines 17–19) queues runs; under approach 3, the second queued run force-pushes a branch built from the (now newer) main HEAD, replacing the first run's branch state. PR auto-updates as a side effect (acceptable improvement; not mission creep — it's a natural property of the chosen approach).
- **R4.** Validation gate — 10 consecutive clean merges OR 14 days, zero Release-workflow failures. **Observational, post-merge.** Documented in the deferred section; not a code-producing implementation unit.
- **R5.** Rename `.github/workflows/release-plz.yml` → `.github/workflows/release-pr.yml` — Unit 2.
- **R6.** Update `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` with a Stage 3 / Class C resolution section — Unit 3.

## Scope Boundaries

**In scope:**
- Single workflow file change (`.github/workflows/release-plz.yml` → `release-pr.yml` after rename) to make the push idempotent.
- Workflow rename + active reference updates.
- Compound doc Stage 3 section.

**Out of scope (explicit, per the issue body):**
- Tool switch (git-cliff → release-plz/something else).
- Changing the workspace's `publish = false` strategy.
- Retroactive fixes for the already-noisy historical CI failures.
- Auto-cleanup of post-merge orphan branches (e.g., `release/v0.5.0` after v0.5.0 ships) — this is a related but distinct gap, deferred below.

### Deferred to Separate Tasks

- **Post-merge release-branch GC.** When a `chore: release vX.Y.Z` commit is merged, `LATEST_TAG` advances and the next workflow run targets `release/v(X.Y.Z+1)`. The just-merged `release/vX.Y.Z` becomes a permanent orphan on origin, requiring manual cleanup. Today's pre-flight cleanup demonstrated this. Worth a follow-up ticket; not in this PR's scope.
- **PR-never-auto-updates** as a *standalone* concern — but this fix incidentally addresses it (recreate-from-main + force-push means the branch always reflects current main, so the PR shown on GitHub is always up to date). Document as a side-effect, not a separate scope item.

## Context & Research

### Relevant Code and Patterns

- `.github/workflows/release-plz.yml:9–134` — the `release-pr` job. The current "Check for existing release PR" (lines 80–92) and "Create release branch and PR" (lines 94–134) steps are replaced by Unit 1.
- `.github/workflows/release-plz.yml:135–183` — the `release-tag` job. **Unchanged.** Its `if:` condition keys on the commit message `chore: release v…` on `main`, which is independent of how the PR was produced.
- `.github/workflows/release.yml` — separate workflow that fires on `push: tags: v*` to build and upload cross-platform binaries. **Unchanged** (no dependency on the `release-pr` job's mechanics).
- `cliff.toml` (referenced by commit `4825e7ae`) — git-cliff's changelog template. **Unchanged.**
- `release-plz.toml` — **does not exist** (deleted during the 4825e7ae migration; the upstream `release-plz` tool is gone). Mentioned for completeness — earlier external commentary suggested it still exists; verified absent in the worktree.

### Institutional Learnings

- `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` — the failure-class taxonomy. This plan's Unit 3 adds the Stage 3 / Class C resolution section.
- `docs/solutions/best-practices/infra-fix-compounding-practice-2026-04-23.md` — operationalizes the look-back rule that's part of why this ticket exists (the issue author already had three approaches enumerated and a stated lean — the implementer's job is to honor that work, not re-derive).

### External References

None used. Local grounding from the chronic-drift doc + the issue body covers all design questions; the workspace patterns (mixed `publish = false` / Class A history) make every external "use upstream tool X" suggestion already-evaluated and rejected.

## Key Technical Decisions

- **Approach 3 (recreate from main) over approach 1 (rebase) or approach 2 (force-with-lease).** Rationale: the branch's content is purely derivative — `main HEAD + sed-bumped Cargo.toml + cargo update + git-cliff CHANGELOG.md + chore commit`. Throwing it away and rebuilding is the simplest correct invariant.
- **Use plain `git push --force` (not `--force-with-lease`).** Rationale: approach 3 explicitly throws away history each run. `--force-with-lease` would protect against human commits to the release branch — but the contract is that this branch is tool-owned, and the release process documentation will state that explicitly. If a defense-in-depth concern surfaces in review, switch to `--force-with-lease`; both are mechanically valid under approach 3.
- **Reorder existing-PR check from before-push to after-push.** Under approach 3, the branch is always recreated, so the "branch exists?" question is answered by the workflow itself, not by probing remote state. The remaining check is "is there an open PR I should not duplicate?" — done after push, only to decide whether to call `gh pr create`. This is the minimal change that makes the recreate semantics correct without adding new branching logic.
- **Single combined step instead of separate "check" + "create" steps.** Reduces surface area and eliminates the cross-step `if:` guard that today's flow uses.
- **Workflow rename happens in this PR, not a separate one.** The AC bundles them; doing it separately would mean another round-trip through CI and another commit on the chronic-drift class.
- **Compound doc update is part of this PR.** Per the AC and per `feedback_compound_infra_fixes.md` — the Stage 3 section captures the resolution while context is fresh.

## Open Questions

### Resolved During Planning

- *Should the existing-PR check stay before or after the push?* — Resolved: after. Under approach 3, the push is always desired; the PR-create call is the only conditional.
- *Should the rename be in this PR or deferred?* — Resolved: this PR. AC bundles them.
- *Use `--force` or `--force-with-lease`?* — Resolved: `--force`, matching approach 3's "every run independent" framing. Open to switching during review if a concrete concurrency or defense-in-depth concern is raised.

### Deferred to Implementation

- **Whether any historical doc/plan reference to `release-plz.yml` should also be updated.** The implementer should grep at `/ce:work` time and update only *active* references (CLAUDE.md, deployment docs, the chronic-drift doc itself). Historical plans dated 2026-03-* and 2026-04-* describe past state and should not be retroactively rewritten — that erases history and breaks `feedback_compound_infra_fixes` traceability.
- **Whether `actionlint` or another YAML linter is wired into the repo's pre-commit** — if it is, run it; if not, manual `yq '.' .github/workflows/release-pr.yml` is sufficient.
- **Branch protection rule status for `release/*`.** GitHub branch protection settings live outside the repo. If a rule blocks force-push to `release/*`, this fix will fail in production despite passing static review. The implementer should check (or ask the operator) whether such a rule exists. Plan mitigation: if it does, either remove the rule or switch to approach 2 (`--force-with-lease`) which still requires force-push permission. Surface in the PR description so a human gate-keeper can confirm.

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
Current flow (broken):
  has_changes? ─yes─> compute NEXT ─> existing-PR? ─yes─> [skip]
                                              │
                                              no
                                              ↓
                                      create local branch from main
                                      bump version, regen changelog, commit
                                      git push        ← FAILS here when remote
                                                        branch exists w/o PR
                                      gh pr create

Approach 3 flow (this fix):
  has_changes? ─yes─> compute NEXT ─> recreate local branch from main
                                      bump version, regen changelog, commit
                                      git push --force                  ← ALWAYS succeeds
                                      open-PR exists? ─yes─> [done]    ← branch is now updated;
                                                                        PR auto-tracks
                                                      no
                                                      ↓
                                                gh pr create
```

The shape change is: **always do the work**; check for the PR *after*, only to decide whether to open a new one. Under this shape, the branch on origin is always equal to `main HEAD + version bump + changelog`, regardless of whatever orphan state existed before.

## Implementation Units

- [ ] **Unit 1: Make the `release-pr` push idempotent (approach 3 — recreate from main)**

**Goal:** Replace the "Check for existing release PR" + "Create release branch and PR" steps with a single step that always recreates the release branch from main, force-pushes it, and conditionally opens a PR if none is open. Eliminates the non-fast-forward failure mode permanently.

**Requirements:** R1, R2, R3.

**Dependencies:** None. Can land in any order with Units 2 and 3.

**Files:**
- Modify: `.github/workflows/release-plz.yml` (will be `release-pr.yml` after Unit 2; either order works)

**Approach:**
- Delete the "Check for existing release PR" step (current lines 80–92).
- Replace the body of the "Create release branch and PR" step (current lines 94–134) with a flow that:
  1. Sets git identity.
  2. Runs `git checkout -B "release/v${NEXT}" main` (the `-B` recreates locally regardless of prior state).
  3. Bumps `Cargo.toml` workspace version, runs `cargo update --workspace`, regenerates `CHANGELOG.md` via `git-cliff --tag "v${NEXT}" -o CHANGELOG.md`.
  4. Stages the three files, commits as `chore: release v${NEXT}`.
  5. Force-pushes: `git push --force origin "release/v${NEXT}"`.
  6. Probes for an existing open PR with `gh pr list --state open --head "release/v${NEXT}" --json number --jq '.[0].number'`.
  7. If no open PR exists, runs `gh pr create` with the same body template that exists today (extract `CHANGELOG_BODY` via `git-cliff --tag "v${NEXT}" --unreleased --strip header`).
- Step `if:` guard remains `steps.check.outputs.has_changes == 'true'` (preserves the no-unreleased-commits skip).
- All `gh pr list` invocations explicitly pass `--state open` (defensive — current code relies on default, which is open but worth being explicit).

**Patterns to follow:**
- Existing git identity setup (current lines 103–104).
- Existing PR body template (current lines 122–131) — preserved verbatim, only the surrounding orchestration changes.
- The `release-tag` job below remains untouched; its `chore: release v…` commit-message contract still produces the tag and GitHub release downstream.

**Test scenarios:**
- *Static — happy path walkthrough:* given LATEST_TAG=v0.5.0 and 175 unreleased commits on main, the step computes NEXT=v0.6.0, recreates branch, force-pushes (succeeds because `release/v0.6.0` doesn't exist on origin after pre-flight cleanup), probes for open PR (none), opens new PR. Verify by reading the rewritten step end-to-end.
- *Static — concurrent-merges walkthrough:* run R1 finishes (branch exists, PR open). Run R2 starts (queued by `concurrency`), main HEAD has advanced. Recreates branch from new main HEAD, force-pushes (overwrites R1's branch tip), probes for open PR (R1's PR is open), skips create. Verify the existing PR's branch reference auto-tracks the force-push (this is GitHub's default behavior).
- *Static — already-merged walkthrough:* the just-merged commit is `chore: release v0.6.0`. LATEST_TAG advances to v0.6.0 (set by `release-tag` job downstream). Next merge fires this workflow with NEXT=v0.7.0, recreates `release/v0.7.0` (which doesn't exist), opens a new PR. The orphan `release/v0.6.0` persists on origin (deferred to separate ticket).
- *YAML validation:* run `yq '.' .github/workflows/release-pr.yml` (or `actionlint` if available locally) to confirm the file parses.

**Verification:**
- The workflow file is syntactically valid YAML / GitHub Actions schema.
- The "Create or update release branch and PR" step (renamed from "Create release branch and PR") contains exactly one `git push` call, with `--force`.
- No `if: ... steps.existing.outputs.exists == 'false'` guard remains anywhere in the workflow.

---

- [ ] **Unit 2: Rename workflow file `release-plz.yml` → `release-pr.yml`**

**Goal:** Eliminate the misleading filename — the tool has been git-cliff since 4825e7ae, but the workflow filename still references the upstream Rust tool.

**Requirements:** R5.

**Dependencies:** None — but easier to land *after* Unit 1 so the rename diff is just a rename, not a rewrite-plus-rename.

**Files:**
- Rename: `.github/workflows/release-plz.yml` → `.github/workflows/release-pr.yml` (use `git mv`)
- Modify: `CLAUDE.md` (CI/CD section — change `release-plz.yml` → `release-pr.yml`)
- Modify: `docs/deployment.md` (active deployment doc — update any mention of the filename)
- Modify: `crates/mika-agent/docs/deployment.md` (mirrored copy synced via `scripts/sync-agent-docs.sh`; update both)
- Modify: `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` — the line at the bottom of the "Tool evolution" appendix that says *"The release-plz.yml filename is retained for backward compatibility… Renaming to release-pr.yml is in mika#775's AC"* should become *"The workflow file is now `.github/workflows/release-pr.yml` (renamed in mika#775)."*

**Approach:**
- `git mv` the file (preserves rename metadata in git history).
- `git grep -l 'release-plz\.yml'` audit at start of unit; update only paths NOT under `docs/plans/2026-0[3-4]-*` (historical plans), `CHANGELOG.md`, or any commit-message-style content. Specifically *do not* edit:
  - `docs/plans/2026-03-01-feat-automated-release-system-plan.md`
  - `docs/plans/2026-03-25-003-fix-skip-pipeline-artifacts-for-release-plz-plan.md`
  - `docs/plans/2026-04-01-003-fix-release-binary-builds-and-clean-up-release-naming-plan.md`
  - `docs/plans/2026-04-02-001-fix-build-rs-out-dir-dashboard-embedding-plan.md`
  - `docs/plans/2026-04-23-006-feat-institutionalize-infra-fix-compounding-plan.md`
  - `docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md` (explicitly historical)
  - `docs/solutions/ci-cd/release-binary-build-failures-rust-embed.md` (historical context)
- Check `docs/solutions/best-practices/autonomous-agent-operational-discipline-2026-04-23.md` and `docs/solutions/best-practices/infra-fix-compounding-practice-2026-04-23.md` — if their references describe historical state ("the release-plz.yml workflow at the time"), leave them; if they describe the *current* file, update them.
- Check `todos/373-complete-p2-pin-github-actions-to-commit-shas.md` — same heuristic.

**Patterns to follow:**
- Pre-existing precedent for "filename retained for tooling-history reasons" — none in this repo, which is *why* the rename is the right call.

**Test scenarios:**
- *Active-references audit:* `git grep -l 'release-plz\.yml'` after the unit returns only the historical files explicitly preserved above (a known short list). No active surface (`CLAUDE.md`, `docs/deployment.md`, `crates/mika-agent/docs/deployment.md`, the chronic-drift doc) still says `release-plz.yml`.
- *Workflow still triggers:* GitHub Actions matches workflows by their `on:` triggers, not filename. After rename, the next push to main should trigger the renamed workflow exactly as before. (Validated by the next merge after this PR lands.)
- *Job names unchanged:* the job `name:` fields (`Release PR`, `Tag & Release`) are unchanged, so any GitHub branch protection rules referencing those names continue to work.

**Verification:**
- `git ls-files .github/workflows/release-plz.yml` returns nothing.
- `git ls-files .github/workflows/release-pr.yml` returns the file.
- `git grep -l 'release-plz\.yml' -- ':!docs/plans/2026-0[3-4]*' ':!docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md' ':!CHANGELOG.md'` returns no results.

---

- [ ] **Unit 3: Add Stage 3 / Class C resolution section to the chronic-drift compound doc**

**Goal:** Capture the resolution while context is fresh, per `feedback_compound_infra_fixes.md` and per the AC. The doc's "Current failure (Class C, open)" section explicitly says it will be updated when this ticket lands.

**Requirements:** R6.

**Dependencies:** Conceptually depends on Unit 1's design being settled (so the doc can describe it accurately). Does not depend on Unit 1 being merged — the design *is* the plan, and the plan is settled.

**Files:**
- Modify: `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`

**Approach:**
- Add a new section between `## Current failure (Class C, open)` and `## Operational workaround`, titled `## Stage 3 — Class C resolution (mika#775, 2026-05-06)`. Section content:
  - Approach chosen: recreate-from-main with `git push --force`. Cite the rationale (no meaningful history on the branch; deterministic from main HEAD + version + changelog).
  - Why it addresses the *class*, not the instance: every Class C symptom historically arose from "branch state diverged from what the workflow expected." Approach 3 makes the workflow stop having an expectation about prior state — the branch state is recomputed on every run. Future variants of Class C (e.g., a future tool also targeting `release/*` branches) inherit a viable pattern.
  - Class A/B vulnerabilities that remain: Class A (workspace dep resolution) is dormant because git-cliff doesn't `cargo package`. Class B (comparison mode) is dormant because git-cliff uses tag-based comparison and the workflow's `LATEST_TAG = git describe --tags --abbrev=0` is straightforward. Both could resurface under a future tool migration.
  - What this fix *does not* close: the orphan-after-merge issue (deferred), the PR-not-auto-updating issue (incidentally closed as a side effect of approach 3, but worth documenting separately).
- Update the `## Current failure (Class C, open)` section: change "open" to point to the new Stage 3 section ("see Stage 3 below for the resolution").
- Update the line in the "Tool evolution" appendix that mentions the rename (per Unit 2's note above).
- Update frontmatter: `resolved: false` → `resolved: pending-validation` (a mid-state honest about the validation gate). When the gate passes (10 merges or 14 days), a follow-up commit flips this to `resolved: true`.

**Patterns to follow:**
- The doc's existing per-class resolution voice — terse, evidence-grounded, calls out class-vs-instance explicitly.

**Test scenarios:**
- Test expectation: none — documentation update with no behavioral testable change.

**Verification:**
- The new `## Stage 3 — Class C resolution (mika#775, 2026-05-06)` section exists and addresses approach choice, why-class-not-instance, what-remains, and what-this-doesn't-close.
- The `## Current failure (Class C, open)` section now points readers at Stage 3.
- Frontmatter `resolved` field is `pending-validation`.

## System-Wide Impact

- **Interaction graph:** the `release-pr` job's only interaction is the `release-tag` job *downstream*, via the `chore: release vX.Y.Z` commit message contract on `main`. That contract is unchanged — only the *path* by which the commit reaches main is different (PR auto-updates instead of being created once and never updated). `release-tag` does not care.
- **Error propagation:** the workflow's failure mode under the fix is now cleanly bimodal: either `git push --force` succeeds (almost always) or it fails because of a branch protection rule (rare; surfaces as an actionable error). The previous tri-modal failure (push reject / PR create fail / both) collapses.
- **State lifecycle risks:** post-merge orphan branches (`release/v0.6.0` after v0.6.0 ships) are not cleaned up. Acknowledged as a deferred concern; not a regression — the current behavior also leaves orphans.
- **API surface parity:** none. The workflow's external surface is the GitHub PR it creates; the PR title, body, label, and base are unchanged.
- **Integration coverage:** the workflow only fires on `push: branches: main` and `workflow_dispatch`. There is no pre-merge runner that could test the change against a synthetic main-push event; validation is post-merge observational (the AC's 10-merges-or-14-days gate).
- **Unchanged invariants:**
  - `release-tag` job (lines 135–183) — untouched.
  - Concurrency group `release-pr` with `cancel-in-progress: false` — untouched. Approach 3 is correct under this concurrency behavior.
  - `cliff.toml` — untouched.
  - The `release-plz.toml` *absence* (deleted in 4825e7ae) — preserved.
  - `release.yml` (binary builds on tag push) — untouched.

## Risks & Dependencies

| Risk | Mitigation |
|---|---|
| Branch protection rule on `release/*` blocks force-push, fix passes static review but fails in production. | Surface in the PR description; ask operator to confirm no such rule exists, or remove it. Fallback: switch to approach 2 (`--force-with-lease`), which still needs force-push permission but is the minimum acceptable form. |
| Force-push during open PR review disrupts review comments. | Acceptable: this PR is auto-generated, not subject to line-by-line review (it's a version bump + autogenerated changelog). The chronic-drift doc accepted "PR never auto-updating" as the prior status quo; force-update is strictly better for the PR's actual purpose. |
| A future contributor manually pushes a fix to `release/v0.6.0` and the next workflow run silently overwrites it. | The release process documentation (Unit 3's Stage 3 section + the chronic-drift doc generally) explicitly states `release/*` branches are tool-owned. Belt-and-suspenders option: branch protection rule restricting human push to `release/*` (out of scope for this PR; can be added later). |
| Validation gate signal is noisy because main-push frequency is high. | The gate's 10-merges-or-14-days is OR-coupled. At current merge frequency (~10 merges/day visible in `git log`), 10 consecutive clean runs is reachable in ~1 day of normal traffic, well inside the 14-day window. |
| New Class A/B failure surfaces post-deploy. | Out of scope for this ticket. The chronic-drift doc's class structure ensures any new failure gets re-classified, not pattern-matched as "another Class C." |

## Documentation / Operational Notes

- The compound doc update (Unit 3) is the durable artifact. It captures the chosen approach + rationale so the next person reading "release CI is broken again" finds the resolution path immediately.
- The CLAUDE.md and `docs/deployment.md` updates from Unit 2 are correctness fixes (filename now matches reality), not new content.
- The PR description should call out the branch-protection-rule check explicitly so the human gate-keeper can confirm before merge.
- Post-merge: the validation gate (10 clean merges OR 14 days, zero Release-workflow failures) is observational. When it passes, a one-line follow-up commit flips `resolved: pending-validation` → `resolved: true` in the chronic-drift doc frontmatter.

## Sources & References

- **Origin issue:** mika#775 — *fix(ci): release workflow pushes non-fast-forward on every merge — release/v0.6.0 chronic drift*
- **Compound doc:** `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`
- **Migration commit:** `4825e7ae` (release-plz → git-cliff, 2026-04-03)
- **Class C taxonomy:** `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` § "Failure classes that survive tool choice" → Class C
- **Operational workaround already executed:** `git push origin --delete release/v0.5.0 release/v0.5.1 release/v0.6.0` (this session, 2026-05-06, before dispatch)
- **Look-back rule that surfaced this prior work:** memory entry `feedback_compound_infra_fixes.md`; institutionalized via mika#776
