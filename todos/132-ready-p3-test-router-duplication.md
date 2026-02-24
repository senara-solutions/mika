---
status: ready
priority: p3
issue_id: "132"
tags: [code-review, quality, testing]
dependencies: []
---

# Test Router Duplicated from Production Router

## Problem Statement

The test helper `test_app()` in `server/mod.rs:170-180` manually reconstructs the same router that `run_server` builds. If routes or middleware are added to production, the test router must be updated separately — easy to forget, causing tests to pass on stale routing.

## Findings

- **Source:** architecture-strategist
- **Location:** `crates/mika-agent/src/server/mod.rs:170-180`

## Proposed Solutions

### Option 1: Extract router construction to a shared function used by both
- **Pros**: Single source of truth
- **Cons**: Production startup may need slight restructuring
- **Effort**: Small
- **Risk**: Low

## Acceptance Criteria

- [ ] Router construction shared between production and tests
- [ ] All tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review

## Resources

- PR #5: Phase 2 Container HTTP Server
