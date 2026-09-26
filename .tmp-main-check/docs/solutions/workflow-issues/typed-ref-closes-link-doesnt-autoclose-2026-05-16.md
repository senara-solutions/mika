---
title: "`Closes mika#N` typed-ref doesn't auto-close issues on GitHub"
module: workflow
date: 2026-05-16
problem_type: workflow_issue
component: github_integration
severity: medium
tags:
  - github
  - pr-template
  - typed-ref
  - autoclose
  - squash-merge
related_components:
  - mika-platform-pr-template
  - dispatch-lib
  - dev-pilot
applies_when:
  - "A PR's body or squash-merge subject references its closing issue using the typed-ref convention (e.g., `mika#920`)"
  - "Operator expects GitHub to auto-close the referenced issue on merge"
---

# `Closes mika#N` typed-ref doesn't auto-close issues on GitHub

## Symptom

A PR uses one of this workspace's typed-ref conventions to name its closing issue — `Closes mika#920` in the body, or `(mika issue#920)` in the squash-merge subject. After merge, the referenced issue **stays OPEN**. The expected auto-close does not fire. Operator notices days later that "completed" issues are still in the open queue and manually closes them.

## Mechanism

GitHub's [auto-close keyword grammar](https://docs.github.com/en/issues/tracking-your-work-with-issues/linking-a-pull-request-to-an-issue) recognizes exactly three issue-reference forms after a closing keyword:

| Form | Recognized? |
|------|-------------|
| `#N` | Yes (same-repo) |
| `org/repo#N` | Yes (cross-repo) |
| `GH-N` | Yes (legacy alias) |
| `mika#N` | **No** — treated as plain text |
| `(mika issue#N)` | **No** — treated as plain text |

The workspace-internal "typed-ref" convention (`mika#N`, `mika-cloud#N`, `mika-skills#N`) was adopted to disambiguate cross-repo references in conversation and ticket bodies. It is **not** a recognized GitHub syntax. When a PR body says `Closes mika#920`, GitHub's parser sees a closing keyword followed by something it does not match as an issue reference, and emits no auto-close link.

The same failure mode applies in squash-merge subjects: `fix(dispatch): … (mika issue#920) (#1143)` is not auto-close grammar. The `(#1143)` at the end is the PR self-reference auto-appended by the squash strategy; it does not point at issue #920.

## Canonical instances (2026-05-16)

**Instance 1 — mika#920 / PR #1143**

PR #1143 squash subject: `fix(dispatch): isolate verify-pipeline-test from CI parent-PR env vars (mika issue#920) (#1143)`. Body: contained "Closes mika#920" line. After merge: issue #920 remained OPEN. Closed manually by orchestrator-Claude.

**Instance 2 — mika#899 / PR #1145**

PR #1145 body: `Closes mika#899`. After merge: issue #899 remained OPEN. Closed manually by orchestrator-Claude.

N=2 in a single day from independent dispatch paths. Pattern is mechanical, not random — every PR using the typed-ref convention will produce this failure.

## Why the convention exists (and what to do about it)

The typed-ref convention is **correct for human-readable references** in chat, ticket bodies, and handsoff logs, where the repo qualifier prevents ambiguity ("`#920`" is ambiguous in a multi-repo workspace; "`mika#920`" is not). It became a problem when it propagated into **machine-parsed locations** — PR auto-close grammar and squash subjects.

The fix is a separation: typed-ref for prose, plain GitHub grammar for auto-close mechanics.

## Fix options

**Option A — PR template guidance (lowest cost).**

Update the PR template across mika-platform repos to make the autoclose line use plain `Closes #N` and keep typed-refs only for cross-repo companion references:

```
## Closes
Closes #N   # same-repo, plain form, auto-closes

## Companion (cross-repo, optional)
Companion PR: senara-solutions/mika-cloud#42   # cross-repo, doesn't auto-close
```

Cheapest move. Relies on author discipline (or LLM dispatch) to follow the template. No tooling required.

**Option B — Post-merge close automation.**

A GitHub Action on `pull_request: closed` that scans the squash-merge subject and body for typed-ref patterns (`mika#\d+`, `mika-cloud#\d+`, etc.) and closes matching issues via the API. Insurance against template drift. Higher surface (workflow file, secret scope) but mechanical.

**Option C — Stop using typed-refs in PR bodies entirely.**

The strictest position: forbid typed-refs in machine-parsed locations, accept the ambiguity that "Closes #N" creates in a multi-repo workspace. Acceptable because PRs are repo-scoped — the closing issue is almost always same-repo, and the rare cross-repo case is already handled by the `org/repo#N` syntax that GitHub natively understands.

**Recommended: A + C, in that order.** Update templates to use plain `Closes #N`; reserve typed-refs for prose-only references. Option B is overkill for the volume.

## Detection (until fix lands)

After any merge, before closing the session loop, run:

```sh
gh pr view <pr-num> --json body,title -q '.body, .title' | rg -o 'mika[a-z-]*#\d+'
```

If matches return, the referenced issues did NOT auto-close. Either run `gh issue close <num>` manually, or wait for the post-merge automation if Option B ships.

## Related

- PR template: `.github/PULL_REQUEST_TEMPLATE.md` in each mika-platform repo.
- Memory: `feedback_task_reference_format` (typed-ref convention for prose contexts — still correct, just doesn't extend to PR auto-close).
