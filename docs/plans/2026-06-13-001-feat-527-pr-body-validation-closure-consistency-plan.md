---
ticket: mika#527
branch: feat/527/pr-body-validation-closure-consistency
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/527
execution: code
---

# Plan: PR body validation — closure-consistency + follow-up tracker (mika#527)

## Problem frame

Two recent incidents (mika#517 and mika#512) closed parent issues prematurely:
- mika#517 closed by PR #523 while sub-issue #516 was still OPEN (PR body openly said `Refs #516`).
- mika#512 closed by PR #522 while a marketplace requirement deferred to "a follow-up PR" was never created.

Both produced orphan work — sub-issues without an owning parent, and follow-up commitments without a tracker. The fix is two structural CI gates on PR bodies:

**Check A (closure-consistency).** When PR body says `Closes #N`, walk #N's formal sub-issue list (GitHub GraphQL `trackedIssues`); if any are OPEN, hard-fail unless the PR body explicitly acknowledges with `Tracked in: <ref>`.

**Check B (follow-up tracker).** When PR body contains a follow-up-deferral trigger phrase ("follow-up PR", "will be fixed in a follow-up", "deferred to a separate PR", "tracked in a follow-up"), require a `Tracked in: <ref>` line naming the tracking issue/PR.

## Resolution of first-pass findings

**F1 — sub-issue discovery mechanism (BLOCKING):** Plan commits to **GitHub GraphQL `trackedIssues`** (architect's option a). The heuristic grep of `#N` references would false-positive on the issue body's "Related" sections (e.g., the body of this very plan references multiple `#N`s that are not sub-issues). `gh api graphql` is a first-class `gh` CLI capability with the same auth token used for the PR-state checks; no new infrastructure needed.

**F2 — cross-repo scope (sharpening):** Cross-repo sub-issue checking is **out of scope** for v1. Rationale:
- The `trackedIssues` GraphQL field only returns same-repo formal sub-issues. Cross-repo references in issue bodies are textual mentions, not formal parent-child links, so the GraphQL approach naturally scopes to same-repo.
- Both cited incidents had distinct concerns: mika#517/#516 was same-repo (in scope); mika#512/#504 was cross-repo (mika→mika-skills) and is handled by Check B's follow-up-tracker requirement (the PR body's "marketplace skills in `mika-skills/` will be fixed in a follow-up" would trigger Check B).
- Cross-repo formal parent-child support can ship as a follow-up if GitHub adds the GraphQL surface or a use case justifies a textual-mention parser.

The plan documents this scope choice in §Scope boundaries and the workflow file's commit message.

## Scope boundaries

- New script: `scripts/check-pr-body-consistency.sh` (Bash) — both checks.
- New CI workflow job: `.github/workflows/pr-body-validation.yml` (or extension of an existing workflow that fires on `pull_request` events).
- Same-repo formal sub-issue check only (GraphQL `trackedIssues`).
- Hard gate (`exit 1`) on violation. Escape hatch via `Tracked in: <ref>` line in PR body.
- **Out of scope:** cross-repo formal sub-issue parent-child detection, retroactive scan of closed PRs, mika-dev integration to auto-author the `Tracked in:` line, advisory mode.

## Implementation Units

### U1 — Sub-issue discovery via GraphQL

**Goal:** A shell function `get_open_sub_issues(parent_number)` that returns the list of OPEN sub-issues for a parent issue, scoped to the same repo.

**Files:**
- Create: `scripts/check-pr-body-consistency.sh`

**Approach:** Use `gh api graphql` with a query like:

```graphql
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      trackedIssues(first: 50, states: OPEN) {
        nodes { number }
      }
    }
  }
}
```

Shell wrapper extracts numbers via `jq`. Authenticated by the existing `GITHUB_TOKEN` env var that CI workflows already have access to. 50-entry cap matches GitHub's typical sub-issue tree sizes — mika issues are rarely deeper.

**Test scenarios:**
- **Issue with no sub-issues:** returns empty list.
- **Issue with all closed sub-issues:** returns empty list.
- **Issue with mixed open/closed sub-issues:** returns only the open ones.
- **Issue with > 50 sub-issues:** returns the first 50 with a stderr warning (defer pagination until a real case hits it).
- **Non-existent issue number:** GraphQL returns null repository.issue; script reports "issue not found" and exits non-zero (do not silently pass).

**Verification:** unit-test the function with mocked `gh api graphql` outputs (cat fixtures piped to a stubbed `gh` for offline testing).

### U2 — Check A: closure-consistency

**Goal:** Parse `Closes #N` from PR body; for each `#N`, call U1; if any open sub-issues remain, hard-fail unless explicitly acknowledged.

**Files:**
- Modify: `scripts/check-pr-body-consistency.sh` (extend with Check A)

**Approach:**

1. Read PR body from `gh pr view ${PR_NUMBER} --json body --jq .body` (PR_NUMBER from `GITHUB_REF` or workflow input).
2. Extract `Closes #N` references via regex `Closes #([0-9]+)` (case-insensitive). Also support `Fixes #N`, `Resolves #N` (GitHub's standard close keywords).
3. For each parent issue number, call `get_open_sub_issues(N)` from U1.
4. For each OPEN sub-issue, check whether the PR body contains `Tracked in: senara-solutions/<repo>#<sub-issue-number>` OR `Tracked in: #<sub-issue-number>`.
5. If any open sub-issue lacks a `Tracked in:` acknowledgment, `exit 1` with a structured error message listing the offending sub-issues.

**Error message shape:**
```
ERROR: PR closes #517, but the following sub-issues are still OPEN and not acknowledged via `Tracked in: <ref>`:
  - #516

Fix one of:
  1. Close the open sub-issue(s) in this PR (add `Closes #516` to the body).
  2. Acknowledge tracking in this PR's body: add a line `Tracked in: #<new-followup-number>` for each.
  3. Re-scope: don't close the parent (#517) if the sub-issues will be done in follow-ups.
```

**Test scenarios:**
- **Happy path 1 — no sub-issues:** PR with `Closes #N` where #N has no sub-issues → pass.
- **Happy path 2 — all sub-issues closed:** `Closes #N` where #N's sub-issues are all closed → pass.
- **Hard-fail — open sub-issue not acknowledged:** `Closes #N` where #N has open sub-issue #M, and PR body has no `Tracked in:` line → exit 1 with error listing #M.
- **Acknowledged path — `Tracked in:` present:** Same as above but PR body contains `Tracked in: #M` → pass.
- **Multiple `Closes`:** PR body with `Closes #N1\nCloses #N2` → both checked, both reported on failure.
- **Mixed keywords:** PR body with `Closes #N\nFixes #M\nResolves #P` → all three recognized.

**Verification:** scripted tests with fixture PR bodies + mocked `gh api graphql` responses; manual smoke against a real PR.

### U3 — Check B: follow-up tracker

**Goal:** Detect deferral trigger phrases in PR body; require a `Tracked in:` line.

**Files:**
- Modify: `scripts/check-pr-body-consistency.sh` (extend with Check B)

**Approach:**

Define trigger phrase set (case-insensitive regex, word-boundary-anchored):
- `follow-up PR`
- `will be fixed in a follow-up`
- `deferred to a separate PR`
- `tracked in a follow-up`
- `addressed in a follow-up`
- `will be (handled|done|fixed) in a (separate|follow-up|follow up) PR`

If any phrase matches AND no `Tracked in:` line is present in the body, `exit 1`. The `Tracked in:` escape hatch must include a valid GitHub ref pattern (`#N` or `<owner>/<repo>#N`).

**Error message shape:**
```
ERROR: PR body indicates deferred work ("will be fixed in a follow-up") but no `Tracked in: <ref>` line is present.

Add a line to the PR body in the form:
  Tracked in: senara-solutions/<repo>#<issue-or-PR-number>

The trigger phrase that fired:
  > will be fixed in a follow-up PR
```

**Test scenarios:**
- **Happy path 1 — no trigger phrase:** PR body with no deferral language → pass.
- **Happy path 2 — phrase + tracker:** trigger phrase present, `Tracked in: senara-solutions/mika-skills#42` present → pass.
- **Hard-fail — phrase, no tracker:** trigger phrase present, no `Tracked in:` line → exit 1.
- **Multiple triggers, one tracker:** body has 2 deferral phrases + 1 `Tracked in:` line → pass (single tracker covers the deferral).
- **Bare-`#N` tracker:** `Tracked in: #42` (same-repo) → pass.
- **Cross-repo tracker:** `Tracked in: senara-solutions/other-repo#99` → pass.
- **Malformed tracker:** `Tracked in: see other PR` (no `#N`) → exit 1 with "tracker reference does not match `#N` or `<owner>/<repo>#N` form".

**Verification:** same shape as U2 — scripted fixtures, manual smoke.

### U4 — CI workflow wiring

**Goal:** Run the script on every `pull_request` event.

**Files:**
- Create: `.github/workflows/pr-body-validation.yml`

**Approach:** Single-job workflow triggered on `pull_request` (opened, edited, synchronize). Checkout the repo (for the script), run `bash scripts/check-pr-body-consistency.sh ${{ github.event.pull_request.number }}`. The PR body is fetched at run time (latest body, so edits during review are re-validated). `GITHUB_TOKEN` is auto-injected.

```yaml
name: PR Body Validation
on:
  pull_request:
    types: [opened, edited, synchronize]
permissions:
  contents: read
  pull-requests: read
  issues: read
jobs:
  validate:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@<pinned-sha>
      - name: Validate PR body
        run: bash scripts/check-pr-body-consistency.sh "${{ github.event.pull_request.number }}"
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

Pin the `actions/checkout` SHA per the project convention. `GH_TOKEN` (not `GITHUB_TOKEN`) is the env var `gh` CLI expects.

**Test scenarios:**
- **Workflow runs on `opened`:** new PR triggers the job.
- **Workflow runs on `edited`:** body edits re-validate.
- **Workflow runs on `synchronize`:** new commits don't re-check the body (body unchanged), but harmless re-run.

**Verification:** smoke-test by opening a deliberately-failing PR (e.g., `Closes #517` with sub-issue still open) post-merge; expect the gate to fire.

### U5 — Docs update

**Goal:** Document the gate in `crates/mika-cli/CLAUDE.md` (CI/CD section) so contributors know about the new requirement.

**Files:**
- Modify: `CLAUDE.md` (CI/CD subsection or contributor guidance section)

**Approach:** Add a one-paragraph note under the CI/CD § listing the new workflow:

> **PR Body Validation (mika#527):** Every PR is checked for (a) closure-consistency — if `Closes #N` is declared, #N's open sub-issues must either be closed in the same PR or acknowledged via a `Tracked in: <ref>` line in the body; (b) follow-up-tracker — if the body says deferred work will be addressed in a follow-up, a `Tracked in: <ref>` line must name the tracker. Both checks hard-fail. Escape hatch: add `Tracked in: senara-solutions/<repo>#<number>` lines to the PR body.

**Verification:** manual read.

## Dependencies / sequencing

- U1 → U2 (U2 calls U1's GraphQL function)
- U1 → U3 has no dependency on U1 (Check B doesn't need sub-issue lookup), but lives in same script so it ships together
- U4 wires U1+U2+U3 into CI
- U5 docs ship in the same PR

## Patterns to follow (cross-cutting)

- `scripts/check-byte-slices.sh`, `scripts/check-loop-select.sh` — existing CI-script patterns (Bash, single-purpose, structured output)
- `.github/workflows/ci.yml` — existing workflow shape, pinned action SHAs
- `gh api graphql` — used in `crates/mika-agent/src/github_graphql.rs` (for `blockedBy` checks), same auth model

## Verification (top-level)

- All test scenarios pass via mocked fixtures
- `shellcheck scripts/check-pr-body-consistency.sh` clean (matches project convention for shell scripts)
- Manual smoke: deliberately-failing PR opened post-merge → gate fires; corrected PR → gate passes

## Risk / known unknowns

- **GraphQL `trackedIssues` API surface stability.** GitHub considers sub-issues GA; the field is stable. If GitHub later renames/restructures, the script's GraphQL query needs updating in one place.
- **`gh` CLI version drift.** Workflows use ubuntu-22.04 + the system `gh`. The `api graphql` subcommand has been stable for years.
- **Body-edit retroactive enforcement.** The workflow re-runs on `edited` events, so removing the `Tracked in:` line after the initial validation will be caught on the next sync.
- **PR template integration.** Not blocking, but a follow-up could add `## Tracked in:` to `.github/pull_request_template.md` to make the escape hatch discoverable.

## Out-of-scope (explicit)

- Cross-repo formal sub-issue parent-child detection (see §Resolution of first-pass findings F2).
- Retroactive scan of historical closed PRs.
- mika-dev auto-authoring of `Tracked in:` lines.
- Advisory mode (warn-but-don't-fail) — hard gate is intentional per first-pass guidance.
- PR template editing — separate concern.
