---
title: "Release automation chronic drift — failure classes that outlive tool choice"
date: 2026-04-23
category: ci-cd
problem_type: operational-pattern
severity: medium
resolved: false
tags:
  - release-automation
  - ci-cd
  - chronic-drift
  - institutional-memory
  - rust-workspace
modules:
  - .github/workflows/release-pr.yml
  - .github/workflows/release.yml
related:
  - docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md
---

## Problem statement

Release automation for this Rust workspace has been chronically brittle across **three different tools** (semantic-release → release-plz → git-cliff) over ~7 weeks, producing **14+ fix commits with zero durable institutional memory** until this doc. Every tool landed with a working initial commit, accumulated 5–10 fixes, then either stabilized precariously or was replaced. Each fix was muscle-memory at the time and evaporated within a week.

As of 2026-04-23, the current tool (git-cliff, since 2026-04-03) fails on every merge to `main` with:

```
! [rejected]        release/v0.6.0 -> release/v0.6.0 (non-fast-forward)
error: failed to push some refs to 'https://github.com/senara-solutions/mika'
hint: Updates were rejected because the tip of your current branch is behind
      its remote counterpart.
```

Zero impact on the CI gate (`CI` workflow is green on the same commits). The `Release` workflow fails silently in parallel; real impact is noise plus the release PR never auto-updating. But the symptom isn't what this doc is about. **This doc is about why the same failure class keeps resurfacing under different tools**, so the next fix can address a class rather than an instance.

## Failure classes that survive tool choice

The 14+ historical fix commits cluster into **four root causes**. Each has outlasted at least one tool migration. Name these explicitly when diagnosing the next failure:

### Class A — Workspace dep resolution with mixed publish-status crates

This workspace has 4 crates: `mika-common` (publishes to crates.io), `mika-agent` / `mika-cli` / `mika-gateway` (`publish = false`). Tools that run `cargo package` or similar verification on all workspace members can't resolve inter-crate deps: the dependent crates were never published to crates.io, so `cargo package`'s metadata lookup fails.

This is the class that **killed release-plz**. From commit `4825e7ae` (migration to git-cliff, 2026-04-03):

> *"release-plz is fundamentally incompatible with publish = false workspace crates that have inter-dependencies: it always runs cargo package on all workspace members, and cargo package can't resolve workspace dep versions from crates.io (where they were never published). This caused 7+ failed fix attempts over April 1-3."*

Historical fixes against Class A (all release-plz era):

| Commit | Fix |
|---|---|
| `04c65428` | remove version from workspace dep specs for publish = false crates |
| `3361bf53` | set `release = false` on all crates except mika-common |
| `db341b6a` | declare only mika-common in `release-plz.toml` |
| `3ae64459` | restore `publish = false` to match Cargo.toml declarations |
| `8800d8e8` | exclude mika-agent from release-plz packaging |
| `076150bd` | skip cargo package verification in release-plz |
| `58e3dd8a` | disable cargo package verification in release-plz |

Cumulative trajectory of this fix cluster: **each fix narrowed release-plz's responsibility until the tool was doing almost nothing**. The underlying mismatch was never resolved; the tool was scoped down until it stopped hitting it, then replaced.

**For git-cliff and any successor:** verify the tool doesn't `cargo package` all workspace members by default, or configure it to skip. If it does and there's no opt-out, that's a hard stop for this workspace shape — do not spend fix commits narrowing scope; either find a different tool or change the `publish` strategy.

### Class B — Comparison mode / changelog source of truth

The tool needs to know "what's unreleased?" Answer sources: crates.io metadata (doesn't exist for our non-published crates), git tags, commit history, or a persisted changelog file. Tools default to the wrong one; fixes flip flags.

Historical fixes against Class B (release-plz era):

| Commit | Fix |
|---|---|
| `8b1e1f3f` | switch release-plz to `git_only = true` mode for tag-based comparison |
| `621af062` | revert release-plz to crates.io comparison mode |
| `04b66e7c` | add `git_only = true` to fix release PR creation |

Cluster trajectory: **one flag flipped back and forth**, indicating neither mode worked cleanly. Root cause likely same as Class A — crates.io mode fails because crates aren't published; git-only mode fails because of some other mismatch.

**For git-cliff:** the equivalent decision is "conventional commits since last tag" — tag is the source of truth. Confirm the last-tag resolution is correct before diagnosing any "nothing to release" or "everything looks unreleased" failure.

### Class C — Release-branch state management

The tool opens/updates a PR on a long-lived `release/vX.Y.Z` branch. Branch state divergence between runs (manual commits, failed prior runs, concurrent pushes) produces non-fast-forward rejections.

Historical fixes against Class C:

| Commit | Fix |
|---|---|
| `b3fc1f44` | exclude `release/*` branches from pipeline artifact checks (adjacent fix) |
| **(open)** | **current symptom: `release/v0.6.0` non-fast-forward on every merge** |

Cluster trajectory: **only one historical fix, and it's a scope-exclusion (keep the broken branch from blocking other workflows), not a root-cause fix**. The current failure is the same class: the release branch's state diverges from what the workflow expects to push.

**For git-cliff (current open issue):** the fix needs to make the `release-pr` job's push **idempotent with respect to the branch's current state**. Three candidate approaches, each with different trade-offs:

| Approach | Preserves branch history? | Survives concurrent runs? | Simplicity |
|---|---|---|---|
| Rebase onto origin/release before push | Yes, but rewrites | No — second concurrent run still sees stale local branch | Moderate |
| Force-push-with-lease | Partial — only allows if lease matches | Better — lease check catches concurrent divergence | Moderate |
| Recreate `release/vX.Y.Z` from main every run | No — history thrown away each run | Yes — every run is independent | High |

The release PR branch has **no meaningful history worth preserving** — commits on it are regenerated by the tool every run. That tips toward **option 3 (recreate)**, but this is a judgment call for whoever works the fix ticket (mika#775).

### Class D — Packaging / build / identity

Ancillary failures in the build, packaging, or git identity surrounding the release run.

Historical fixes against Class D:

| Commit | Fix |
|---|---|
| `afd8ca24` | use pinned Rust toolchain in release-plz workflow (#394) |
| `0c649c4f` | commit dashboard dist for embedded serving and release-plz |
| `47374b9b` | fix YAML syntax in release workflow (git-cliff era) |
| `e89dc7a3` | add git identity for release tag creation (git-cliff era) |

Cluster trajectory: **one-off fixes, each distinct**. These are the kind of fixes that legitimately don't need root-cause analysis — the problem space is unbounded (every new tool has its own packaging/identity quirks) and each fix stands alone.

**For future fixes:** anything in Class D is a one-off; anything in A/B/C is chronic-drift and needs compound-doc discipline.

## Current failure (Class C, open)

Symptom on every merge to `main` since at least 2026-04-23: `release/v0.6.0` non-fast-forward. Tracked as **mika#775**. Compound doc will get a "Stage 3 — resolution" section when that ticket lands, naming:

- Which of the three candidate approaches was chosen
- Why it survived 10+ consecutive merges without recurrence (validation gate)
- What it closed and what it didn't (Class A/B vulnerabilities may remain)

## Operational workaround (while Class C remains open)

If a release is actually blocked by the non-fast-forward failure:

```bash
# 1. Inspect what's on release/v0.6.0 that shouldn't be
git fetch origin release/v0.6.0
git log --oneline origin/main..origin/release/v0.6.0

# 2. Delete the remote branch — the tool recreates it on next run
git push origin :release/v0.6.0

# 3. Trigger the Release workflow manually via workflow_dispatch,
#    or wait for the next merge to main
```

Until mika#775 lands, this is the reset.

## Tool evolution (appendix — chronological index)

Failure classes are the primary axis of this doc. Tool chronology is here as secondary, to help locate commits by tool-era when grep hits.

- **Stage 0 — semantic-release** (pre-2026-03). No surviving config; replaced because of Rust-workspace integration issues. Primary failures were Class A.
- **Stage 1 — release-plz** (2026-03-01 → 2026-04-03). Setup captured in [`rust-workspace-release-plz-github-actions.md`](./rust-workspace-release-plz-github-actions.md) (now historical). 10+ fixes, all in Classes A, B, D. Migration driven by Class A.
- **Stage 2 — git-cliff** (2026-04-03 → present). Migration commit `4825e7ae`. Fixes so far in Classes C, D. Current open issue is Class C.

The workflow file is `.github/workflows/release-pr.yml` (renamed from `release-plz.yml` in mika#775); the tool is git-cliff.

## Meta — why release automation drifts chronically

Two feedback-loop hazards make release automation unusually evaporative:

1. **Failures only manifest on next push to main** — not on PR CI, not on local tests. So the fix cycle is "merge a PR, observe the next merge, see if it passed" — ~1 fix per merge cycle, with 15–60 min between iterations.
2. **No local reproduction.** The broken state lives in a GitHub Actions runner environment. Reproducing in-repo requires mocking the runner's auth context, network state, and timing.

Combined effect: it's psychologically easier to apply a point-fix than to understand the class. Fourteen+ commits of point-fixes is the anti-pattern.

**Rule going forward:** every release-automation fix that's more than a typo earns a compound-doc entry in THIS file, even if it's three sentences. The friction of writing is negligible compared to re-deriving context the next time. Rule operationalized in `feedback_compound_infra_fixes.md`.

## Cross-references

- [`rust-workspace-release-plz-github-actions.md`](./rust-workspace-release-plz-github-actions.md) — original release-plz setup (2026-03-01, now historical)
- Commit `4825e7ae` — tool migration (release-plz → git-cliff)
- Ticket mika#775 — fix Class C (`release/v0.6.0` non-fast-forward)
- MEMORY: `feedback_compound_infra_fixes.md` — institutional rule about infra-fix evaporation
