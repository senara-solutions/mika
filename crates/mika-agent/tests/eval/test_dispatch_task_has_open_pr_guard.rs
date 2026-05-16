//! Integration test surface for the dispatch_task_has_open_pr guard (mika#920).
//!
//! The guard lives inside `validate_dispatch_readiness()` and operates on a
//! `db::Task` + `tool_input` JSON. Direct unit tests of the guard live in
//! `crates/mika-agent/src/skills/executor.rs` `mod tests` — they cover all
//! five plan scenarios (rejection, iteration_context bypass, no pr_url bypass,
//! ready-label webhook bypass, deferred-dispatch sentinel bypass) and the
//! `register_deferred_callback` sentinel-injection round-trip.
//!
//! This file pins the public predicates the guard depends on: PR URL parsing
//! and verdict extraction from a reviews-list response. These are the building
//! blocks that ride alongside the guard's DB-level decision and stay stable
//! across implementation refactors.
//!
//! The full validate_dispatch_readiness behavior is exercised in:
//!   - `executor.rs::tests::test_open_pr_guard_rejects_re_dispatch_without_context`
//!   - `executor.rs::tests::test_open_pr_guard_bypasses_with_iteration_context`
//!   - `executor.rs::tests::test_open_pr_guard_bypasses_when_no_pr_url`
//!   - `executor.rs::tests::test_open_pr_guard_bypasses_ready_label_webhook`
//!   - `executor.rs::tests::test_open_pr_guard_bypasses_deferred_dispatch_sentinel`
//!   - `executor.rs::tests::test_open_pr_guard_bypasses_dev_groom_skill`
//!   - `executor.rs::tests::test_register_deferred_callback_injects_sentinel`
//!
//! Eval-harness based tests are not added here — the harness uses stub tools
//! that bypass the real executor's long-running path (no `LongRunningContext`,
//! no `validate_dispatch_readiness`), so a stub-based eval cannot exercise the
//! tool-boundary pre-hoc gate. The same constraint is documented in
//! `test_unauthorized_webhook_dispatch_tool_boundary.rs`.

// No tests here — the guard is exercised in `executor.rs` unit tests.
// This file is a contract/discovery surface so engineers grepping the eval
// directory can find the open-PR guard's coverage without missing the unit
// tests that actually exercise it.
