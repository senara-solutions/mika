---
status: complete
priority: p1
issue_id: "040"
tags: [code-review, testing, rust-v2]
dependencies: []
---

# Home Directory Tests Missing #[serial] Annotation

## Problem Statement

Two tests in `home.rs` manipulate the `MIKA_HOME` environment variable without `#[serial]` annotations, creating a race condition with other tests that also use environment variables. This will cause intermittent CI failures.

**Location:** `crates/mika-common/src/home.rs` - `test_resolve_home_dir_with_mika_home` and `test_resolve_home_dir_default`

**Reported by:** pattern-recognition-specialist

## Findings

- `test_resolve_home_dir_with_mika_home` sets and removes `MIKA_HOME` env var
- `test_resolve_home_dir_default` removes `MIKA_HOME` env var
- Config tests in `config.rs` already use `#[serial]` for the same reason
- `serial_test` crate is already a workspace dependency

## Proposed Solutions

### Option A: Add #[serial] to both tests (Recommended)
Add `use serial_test::serial;` and `#[serial]` attribute to both env-var-mutating tests.
- **Pros:** Consistent with existing config.rs pattern, simple fix
- **Cons:** Slightly slower test execution
- **Effort:** Small (5 minutes)
- **Risk:** None

## Acceptance Criteria

- [ ] Both home.rs tests that touch env vars have `#[serial]` annotation
- [ ] `serial_test` added to mika-common dev-dependencies (if not already)
- [ ] All tests pass reliably in parallel

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | Same pattern as config.rs fix from Phase 1 |
