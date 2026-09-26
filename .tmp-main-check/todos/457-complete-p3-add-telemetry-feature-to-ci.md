---
status: complete
priority: p3
issue_id: "457"
tags: [code-review, ci, telemetry]
dependencies: ["451"]
---

# Add --features telemetry Test Step to CI

## Problem Statement

CI (`ci.yml`) runs `cargo test` without `--features telemetry`. The telemetry-gated code paths are never tested in CI, so regressions could slip through.

## Findings

- **Source**: Architecture strategist
- **Location**: `.github/workflows/ci.yml`

## Proposed Fix

Add a matrix entry or separate step: `cargo test --workspace --features telemetry`. Depends on fixing #451 (environment-dependent test) first.

## Acceptance Criteria

- [ ] CI tests both `cargo test` and `cargo test --features telemetry`
- [ ] No environment-dependent test failures in CI
