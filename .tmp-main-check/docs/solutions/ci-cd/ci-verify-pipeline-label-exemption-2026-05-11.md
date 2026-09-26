---
title: "CI verify-pipeline.sh docs-only exemption requires both label AND trailer"
date: 2026-05-11
category: ci-cd
module: scripts
problem_type: inconsistency
component: tooling
symptoms:
  - "Docs-only PR with pipeline-exempt label passes qa-review but fails CI Pipeline Artifacts check"
  - "Operator must add both pipeline-exempt label (for qa-review) AND Pipeline-Exempt trailer (for CI)"
  - "Two inconsistent escape mechanisms for the same docs-only exemption intent"
tags: [ci, pipeline-exempt, docs-only, verify-pipeline, orthogonality]
related_issues: [1067, 1064, 1065, 1062]
---

## Problem

After mika#1065 shipped label-based exemption in the qa-review skill, docs-only PRs still needed a separate `Pipeline-Exempt: docs-only — <reason>` commit trailer to pass the CI `verify-pipeline.sh` check. Two inconsistent mechanisms for the same intent — an orthogonality violation.

Empirically confirmed on mika#1062 (2026-05-10): PR passed qa-review via label but blocked at CI until a trailer commit was manually added.

## Root cause

`scripts/verify-pipeline.sh` only checked commit trailers for exemption. The `pipeline-exempt` label (just shipped in #1065 for qa-review) had no effect on the CI script.

## Solution

Extended `verify-pipeline.sh` to read PR labels from the GitHub Actions event payload (`GITHUB_EVENT_PATH` env var) via `jq`. The `pipeline-exempt` label now bypasses the docs-only rejection path alongside the existing trailer mechanism.

Key design decisions:
- **`GITHUB_EVENT_PATH` + `jq` over `gh pr view`**: The event payload JSON is always present in GitHub Actions `pull_request` events — no `gh` CLI, `GITHUB_TOKEN`, or network call needed
- **Docs-only path only**: The label bypasses only the docs-only rejection, not the code-only path — consistent with #1064's source-change guard
- **Trailer preserved as fallback**: For local runs (no `GITHUB_EVENT_PATH`) and backward compatibility

## Key insight

When extending a CI check's exemption mechanism, prefer reading from `GITHUB_EVENT_PATH` (the event payload file always present in GitHub Actions) over `gh` API calls. Zero new dependencies, zero network, and graceful fallback when the file doesn't exist (local runs).

## Files changed

- `scripts/verify-pipeline.sh` — added label check from event payload, updated docs-only conditional and error message
