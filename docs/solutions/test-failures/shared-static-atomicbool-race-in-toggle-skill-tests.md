---
title: Shared static AtomicBool race condition in toggle_skill tests
date: 2026-04-21
category: test-failures
module: tools/toggle_skill
problem_type: test_failure
component: testing_framework
symptoms:
  - "assertion failed: !ctx.skills_dirty.load(Ordering::Acquire) in test_already_disabled"
  - "test_already_disabled flaky — passes in isolation, fails under cargo test parallelism"
root_cause: test_isolation
resolution_type: test_fix
severity: medium
tags:
  - atomicbool
  - test-isolation
  - shared-static
  - skills-dirty
  - flaky-test
  - concurrent-tests
---

# Shared static AtomicBool race condition in toggle_skill tests

## Problem

`test_already_disabled` in `toggle_skill.rs` intermittently failed in CI with `assertion failed: !ctx.skills_dirty.load(Ordering::Acquire)`. The test expected `skills_dirty` to be `false` after a no-op disable (skill already disabled), but another concurrently running test set it to `true`.

## Symptoms

- `test_already_disabled` passes when run alone (`cargo test -- test_already_disabled`)
- Fails intermittently under `cargo test` (parallel execution)
- The `skills_dirty` flag was `true` despite the tool returning "already disabled" (no-op path)
- The test comment acknowledged the issue: "skills_dirty is a process-wide static"

## What Didn't Work

- Resetting `skills_dirty` to `false` before the assertion (line 261 of the original test) — this is still racy because another test can set it to `true` between the reset and the assertion.

## Solution

Replace `harness.ctx_with_home()` (which uses a shared `static SKILLS_DIRTY: AtomicBool`) with a manually-constructed `ToolContext` that uses a **local** `AtomicBool` per test.

Before (flaky):
```rust
let ctx = harness.ctx_with_home(tmp.path());
ctx.skills_dirty.store(false, Ordering::Release); // racy reset
// ... execute tool ...
assert!(!ctx.skills_dirty.load(Ordering::Acquire)); // can see other test's write
```

After (isolated):
```rust
let skills_dirty = AtomicBool::new(false);
let pr_review_posted = AtomicBool::new(false);
let ctx = ToolContext {
    db: &harness.db,
    session_id: "test-session",
    // ... all other fields ...
    skills_dirty: &skills_dirty,
    pr_review_posted: &pr_review_posted,
};
// ... execute tool ...
assert!(!skills_dirty.load(Ordering::Acquire)); // isolated, deterministic
```

Applied to both `test_already_disabled` and `test_disable_sets_skills_dirty`. Other toggle_skill tests that don't assert on `skills_dirty` continue using `ctx_with_home()` safely.

## Why This Works

`TestHarness::ctx_with_home()` uses a function-level `static SKILLS_DIRTY: AtomicBool`. In Rust, `static` variables are process-wide — all tests calling `ctx_with_home()` share the same `AtomicBool`. Since `cargo test` runs tests in parallel within the same process, any test that calls `toggle_skill` with `enabled: false` on a non-disabled skill will set the shared flag to `true`, racing with tests that assert on its value.

By using a local `let skills_dirty = AtomicBool::new(false)` owned by the test function, each test has complete isolation. The `ToolContext` borrows the local variable, and the borrow checker ensures the variable outlives the context.

## Prevention

- **Use local `AtomicBool`s** in tests that assert on `skills_dirty` or `pr_review_posted`. Follow the pattern in `send_message.rs` tests, not the `ctx_with_home()` helper.
- **Treat `ctx_with_home()` as a convenience** for tests that don't care about shared flag state. If a test needs to verify flag behavior, construct `ToolContext` manually.
- **When adding new shared flags** to `ToolContext`, always use local variables in tests that assert on the flag's value.

## Related Issues

- mika#722 — discovered during CI for the KG schema v25 PR
- `crates/mika-agent/src/test_utils.rs` — `TestHarness` helper with shared statics
- `crates/mika-agent/src/tools/send_message.rs` — reference pattern for local `AtomicBool` per test
