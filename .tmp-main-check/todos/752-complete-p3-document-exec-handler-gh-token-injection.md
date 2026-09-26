---
status: complete
priority: p3
issue_id: 752
tags:
  - code-review
  - documentation
  - exec-handlers
dependencies:
  - "#515"
---

# Document exec handler GH_TOKEN injection in configuration.md

## Problem Statement

`docs/configuration.md` already documents that the builtin `run_gh` handler scrubs and re-injects `GH_TOKEN` for platform identity separation. After commit `859c2dd` (#515), the same scrub-then-inject behavior applies to **all exec handler skills** — not just `run_gh`.

A skill author reading `docs/configuration.md` today would not know that exec handlers automatically receive the agent's `MIKA_GITHUB_TOKEN` as `GH_TOKEN`. They might still think they need to write their own token plumbing.

## Findings

- **Source:** `compound-engineering:review:agent-native-reviewer` review of commit `859c2dd`
- **Current docs:** `docs/configuration.md:217` describes `run_gh` token injection only
- **Gap:** No mention of exec handler token injection
- **Affected audience:** Skill authors writing `gh`-using exec handlers

## Proposed Solution

Add a one-line addendum at `docs/configuration.md:217` (the existing `run_gh` injection note) extending it to cover exec handlers:

> The same scrub-then-inject pattern is applied to all exec handler subprocesses spawned by skills — `MIKA_GITHUB_TOKEN` is re-injected as `GH_TOKEN` after the env scrub, so any `gh` CLI invocation inside a skill handler runs as the agent's configured GitHub identity (not the host user).

Optionally, also note this in `docs/skills.md` where exec handlers are documented.

## Recommended Action

(Triage)

## Technical Details

- **Affected files:** `docs/configuration.md`, optionally `docs/skills.md`
- **Effort:** Small (1-2 sentences)
- **Risk:** None

## Acceptance Criteria

- [ ] `docs/configuration.md` mentions exec handler `GH_TOKEN` injection
- [ ] Optionally: `docs/skills.md` mentions the agent identity guarantee for `gh` calls in exec handlers

## Work Log

- 2026-04-11: Created from `/ce:review` of commit `859c2dd` (mika#515)
- 2026-04-11: Completed — extended the existing `run_gh` injection note in `docs/configuration.md` (lines 217-221) with a 3-sentence addendum covering all exec handler subprocesses.

## Resources

- Commit: `859c2dd`
- Related issues: mika#515, mika#517
- ADR: `docs/adr/008-github-identity-separation.md`
