---
ticket: mika issue#1102
type: fix
branch: fix/1102/self-dev-intent-guard-mika-910-over
date: 2026-05-14
---

# Plan: Tighten INTENT_GUARD #910 trigger predicate (mika#1102)

## Phase 0 Pin — Verbatim Source at `8c01567c` (main)

### Site 1: Import line (`agent.rs:4845`)

```rust
use crate::webhook_dispatch::READY_LABEL_DISPATCH_MARKER;
```

### Site 2: Doc comment + trigger function (`agent.rs:4872-4887`)

```rust
/// #910 — Triggers on `[GitHub]` webhook turns that are NOT ready-label
/// dispatch events.  Inverse of `ready_label_dispatch_trigger` on the
/// `[GitHub]` domain.  Uses `READY_LABEL_DISPATCH_MARKER` (imported from
/// `webhook_dispatch` module, mika#933) for consistency with the positive-case
/// guard.
///
/// NOTE: This predicate is intentionally **over-broad** compared to the
/// tool-boundary `is_unauthorized_webhook_dispatch()` in `webhook_dispatch.rs`.
/// It matches PR and check-suite events too (qa/ci skill territory), which
/// generates confusing post-hoc corrections but does not break those flows
/// (the dispatch already happened by EndTurn).  Tightening this to match the
/// tool-boundary allowlist is a follow-up ticket (see mika#933 plan §
/// "Predicate sharing § Important").
fn webhook_no_unauthorized_dispatch_trigger(msg: &str) -> bool {
    msg.starts_with("[GitHub]") && !msg.starts_with(READY_LABEL_DISPATCH_MARKER)
}
```

### Site 3: Test — PR review (`agent.rs:8513-8518`)

```rust
#[test]
fn no_unauthorized_dispatch_trigger_matches_pr_review() {
    assert!(webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] PR review (approved) on senara-solutions/mika#694 by reviewer"
    ));
}
```

### Site 4: Test — Check suite (`agent.rs:8520-8525`)

```rust
#[test]
fn no_unauthorized_dispatch_trigger_matches_check_suite() {
    assert!(webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Check suite failure on branch fix/foo"
    ));
}
```

### Site 5: Shared predicate — delegation target (`webhook_dispatch.rs:32-49`)

```rust
pub(crate) fn is_unauthorized_webhook_dispatch(msg: &str) -> bool {
    if !msg.starts_with("[GitHub]") {
        return false;
    }
    if msg.starts_with(READY_LABEL_DISPATCH_MARKER) {
        return false;
    }
    // qa skill territory (Phase 0 prefix surface rows E, F).
    if msg.starts_with("[GitHub] PR ") {
        return false;
    }
    // ci skill territory (Phase 0 prefix surface row G).
    if msg.starts_with("[GitHub] Check suite ") {
        return false;
    }
    // Everything else in [GitHub] domain (rows B, C, D, H) is fallthrough.
    true
}
```

---

## Problem

`webhook_no_unauthorized_dispatch_trigger()` in `agent.rs` (line 4885) uses an over-broad predicate:

```rust
fn webhook_no_unauthorized_dispatch_trigger(msg: &str) -> bool {
    msg.starts_with("[GitHub]") && !msg.starts_with(READY_LABEL_DISPATCH_MARKER)
}
```

This fires on **all** `[GitHub]` events except ready-label, including:
- `[GitHub] PR review (approved) on ...` — legitimate qa skill territory
- `[GitHub] Check suite failure on ...` — legitimate ci skill territory

When the guard fires on these turns, the agent receives a confusing "intent-precondition guard fired — re-prompting" correction that doesn't apply to its actual workflow.

## Root Cause

The trigger was written before mika#933 introduced the positive-allowlist predicate. The doc comment on the function explicitly acknowledges the over-broadness and names the follow-up (this ticket).

## Fix

The correct predicate already exists: `is_unauthorized_webhook_dispatch()` in `crate::webhook_dispatch` (lines 32-49 of `webhook_dispatch.rs`). It uses the same positive-allowlist shape as the tool-boundary guard — excludes ready-label, PR events, and check-suite events.

### Changes

**1. `crates/mika-agent/src/agent.rs`**

- Update the import at line 4845 to include `is_unauthorized_webhook_dispatch`:
  ```rust
  use crate::webhook_dispatch::{is_unauthorized_webhook_dispatch, READY_LABEL_DISPATCH_MARKER};
  ```

- Replace the `webhook_no_unauthorized_dispatch_trigger` function body (line 4885-4887) to delegate:
  ```rust
  fn webhook_no_unauthorized_dispatch_trigger(msg: &str) -> bool {
      is_unauthorized_webhook_dispatch(msg)
  }
  ```

- Update the doc comment to remove the "intentionally over-broad" NOTE and reference this fix.

- **Test updates** — two tests flip from `assert!` to `assert!(!...)`:
  - `no_unauthorized_dispatch_trigger_matches_pr_review` → rename to `no_unauthorized_dispatch_trigger_skips_pr_review` and assert `!` (PR events are qa skill territory, must NOT trigger).
  - `no_unauthorized_dispatch_trigger_matches_check_suite` → rename to `no_unauthorized_dispatch_trigger_skips_check_suite` and assert `!` (check suite events are ci skill territory, must NOT trigger).

- All other existing tests remain unchanged (comment events, non-ready labels, direct prompts, empty, callback — all behave identically under the tightened predicate).

**2. `crates/mika-agent/CLAUDE.md`**

- Update the `webhook_no_unauthorized_dispatch` entry in the Post-Conditions § Intent-precondition registry (guard 6b) to remove the "intentionally over-broad" note and state the predicate now shares the `is_unauthorized_webhook_dispatch()` allowlist.

### No changes needed

- `webhook_dispatch.rs` — the shared predicate is already correct and tested (8-row matrix).
- `executor.rs` — the tool-boundary guard already uses `is_unauthorized_webhook_dispatch()`.
- `webhook_no_unauthorized_dispatch_satisfied()` — the satisfied predicate is unchanged (checks for successful `run_claude_pilot` calls).
- The INTENT_GUARDS registry entry — only the trigger function pointer changes; correction message, label, and ordering are all preserved.

## Verification

1. `cargo test -p mika-agent` — all existing tests pass with the updated assertions.
2. The 8-row matrix from `webhook_dispatch.rs::tests::test_is_unauthorized_webhook_dispatch_predicate` already covers the full surface. The agent.rs trigger tests become a subset of that matrix, confirming the delegation works correctly.
3. `cargo clippy` — no warnings.

## Risk Assessment

**Low risk.** The tightened predicate is strictly more permissive on qa/ci turns (removes false-positive corrections) and identical on all other surfaces. The tool-boundary guard in `executor.rs` was the load-bearing prevention all along; this post-hoc EndTurn guard is defense-in-depth. Tightening the defense-in-depth layer to match the primary layer is a pure improvement.

## Acceptance Criteria Mapping

- [x] INTENT_GUARD fires only on ready-label webhook turns → ensured by delegating to `is_unauthorized_webhook_dispatch()` which allowlists ready-label, PR, and check-suite events.
- [x] Same 8-row gateway-prefix-surface test matrix from mika#933 → the matrix already exists in `webhook_dispatch.rs`; the trigger now delegates to it.
- [x] No "intent-precondition guard fired" on qa/ci turns → PR and check-suite events no longer match the trigger.
- [x] Existing ready-label dispatch enforcement preserved → ready-label events are excluded by `READY_LABEL_DISPATCH_MARKER` check in `is_unauthorized_webhook_dispatch()`, same as before.
