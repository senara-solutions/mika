---
title: "fix(ci): add release label so release-pr workflow succeeds"
type: fix
status: complete
date: 2026-05-07
---

# fix(ci): add release label so release-pr workflow succeeds

## Overview

The `release-pr` workflow's `gh pr create --label "release"` call fails because no `release` label exists in the repo. This was unmasked by mika#1003 (Class C resolution) — previously the workflow always failed earlier on the non-fast-forward push.

## Problem Frame

After mika#1003 merged, the force-push succeeds but `gh pr create` exits non-zero because `--label "release"` is an atomic part of the create call and the label doesn't exist. No release PR is created. This is Class D (packaging/build/identity) per `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`.

## Requirements Trace

- R1. The `release-pr` workflow must create a release PR without error after merges to main
- R2. The `release` label must exist in the repo label taxonomy
- R3. The chronic-drift doc must note this Class D fix and reset the validation gate clock

## Scope Boundaries

- Only adding the label to `.github/labels.yml` — no workflow logic changes
- Not changing any other label or workflow behavior
- Out of scope: tool switch, Class A/B dormant issues, post-merge orphan-branch GC

## Context & Research

### Relevant Code and Patterns

- `.github/labels.yml` — canonical label definitions, synced via `EndBug/label-sync`
- `.github/workflows/release-pr.yml` line 137 — the `--label "release"` flag on `gh pr create`
- Labels are grouped by category with section comments (Type, Priority, Component, State, Sprint Hooks)

### Institutional Learnings

- `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` — Class D guidance: one-off fixes, each distinct, don't need root-cause analysis but do need compound-doc entries

## Key Technical Decisions

- **Approach A (add label) over Approach B (remove flag):** Preserves the original authored intent to tag release PRs. The label adds filter value on the GitHub Actions tab and in PR queries (`label:release`). Cost is identical — one line in labels.yml vs one line removed from workflow. The issue author leans A; no reason to override.
- **Label placement:** Under a new `# ── Automation ──` section, after Sprint Hooks. Release labels are workflow-generated, not human-applied — they don't fit Type, Component, or State.

## Implementation Units

- [x] **Unit 1: Add `release` label to `.github/labels.yml`**

**Goal:** Make `gh pr create --label "release"` succeed by ensuring the label exists.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `.github/labels.yml`

**Approach:**
- Add a new `# ── Automation ──` section after Sprint Hooks
- Add `release` label with a descriptive color and description indicating it's applied by CI

**Patterns to follow:**
- Existing section comment format: `# ── Category ──` with box-drawing chars
- Existing label format: `name`, `color`, `description` fields

**Test scenarios:**
- Happy path: `gh label list --repo senara-solutions/mika` shows `release` label after label-sync runs
- Integration: next merge to main triggers `release-pr` job and `gh pr create --label "release"` succeeds

**Verification:**
- Label-sync workflow applies the label to the repo
- Release-pr workflow no longer fails on the `gh pr create` step

- [x] **Unit 2: Update chronic-drift doc with Class D entry**

**Goal:** Document this fix in the institutional memory per the compound-doc discipline rule.

**Requirements:** R3

**Dependencies:** Unit 1

**Files:**
- Modify: `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`

**Approach:**
- Add a row to the Class D historical fixes table noting this fix
- Add a note under Stage 3 that the validation gate clock resets at this fix's first clean post-merge run (per AC)

**Test expectation:** none — documentation-only change

**Verification:**
- The chronic-drift doc mentions mika#1006 and the `release` label fix

## System-Wide Impact

- **Interaction graph:** `EndBug/label-sync` reads `.github/labels.yml` and creates/updates labels on GitHub. The `release-pr` job's `gh pr create --label "release"` then succeeds.
- **Unchanged invariants:** No workflow logic changes. The `--label "release"` flag in `release-pr.yml` stays as-is.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Label-sync hasn't run yet when release-pr runs | Label-sync runs on its own schedule; the label can also be created manually once via `gh label create release` |

## Sources & References

- Related issues: #1006, #1003, #775
- Existing doc: `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`
