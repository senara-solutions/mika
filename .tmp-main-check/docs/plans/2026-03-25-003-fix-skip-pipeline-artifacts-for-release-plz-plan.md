---
title: "fix: Skip Pipeline Artifacts check for release-plz PRs"
type: fix
status: completed
date: 2026-03-25
---

# fix: Skip Pipeline Artifacts check for release-plz PRs

## Overview

The `pipeline-artifacts` CI job in `.github/workflows/ci.yml` enforces that every PR includes a plan doc (`docs/plans/*.md`) and source code changes — verifying the `/mika` development pipeline was followed. However, automated `release-plz` PRs (version bumps + changelogs) are machine-generated and don't go through the `/mika` pipeline. This causes the job to fail on every release PR, blocking CI.

## Problem Statement

PR #181 (`chore: release v0.2.0`) on branch `release-plz-2026-03-16T17-09-56Z` failed the Pipeline Artifacts job because:
1. No `docs/plans/*.md` file exists in the diff (release-plz doesn't create plans)
2. The changes are version bumps in `Cargo.toml` and `CHANGELOG.md` — not `/mika` pipeline output

All other CI jobs (Check, Dashboard, Docs Site, Security) passed. Only the pipeline verification gate is inappropriate for this PR type.

Reference: https://github.com/senara-solutions/mika/actions/runs/23553171591/job/68572586355?pr=181

## Proposed Solution

Add a branch-name exclusion to the `pipeline-artifacts` job's `if` condition in `.github/workflows/ci.yml`:

```yaml
# Before
if: github.event_name == 'pull_request'

# After
if: github.event_name == 'pull_request' && !startsWith(github.head_ref, 'release-plz-')
```

This is the minimal, targeted fix. The `release-plz` branch naming convention (`release-plz-<timestamp>`) is stable — it's set by the release-plz tool and documented in `release-plz.toml`.

## Acceptance Criteria

- [x] `pipeline-artifacts` job is skipped for PRs from branches starting with `release-plz-`
- [x] `pipeline-artifacts` job still runs for all other PRs
- [x] No other CI jobs are affected

## Context

- `.github/workflows/ci.yml:80-89` — the `pipeline-artifacts` job definition
- `scripts/verify-pipeline.sh` — the script that checks for plan docs and source changes (no changes needed here)
- `.github/workflows/release-plz.yml` — creates PRs on `release-plz-*` branches
- `release-plz.toml` — release-plz configuration

## MVP

### `.github/workflows/ci.yml` (line 83)

```yaml
if: github.event_name == 'pull_request' && !startsWith(github.head_ref, 'release-plz-')
```

Single line change. No other files affected.
