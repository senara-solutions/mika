---
status: pending
priority: p2
issue_id: "472"
tags: [code-review, testing, task-engine]
dependencies: []
---

# 472 · `dispatcher.rs` has zero tests — critical error paths unverified

## Problem Statement

`TaskDispatcher::dispatch` and all its sub-dispatchers have no tests at all.
The unknown action type error path, the missing-`text` error for `send_message`,
and the no-op `inject_context` path are all untested. A typo in an action type
string would silently route every task to the "unknown action_type" error branch.

## Findings

- **Location:** `crates/mika-agent/src/task_engine/dispatcher.rs`
- No `#[cfg(test)]` block exists in the file
- `dispatch_send_message` has a required `config["text"].as_str()` extraction — untested
- `dispatch_inject_context` is a no-op — untested
- Unknown action type returns `Err` — untested

## Proposed Solutions

### Option A — Add unit tests to dispatcher.rs (recommended)
Key tests needed:
- `test_dispatch_unknown_action_type_returns_error`: `action_type = "bogus"` → `is_err()`
- `test_dispatch_send_message_missing_text_returns_error`: `action_config = "{}"` → `is_err()`
- `test_dispatch_inject_context_is_noop`: returns `Ok(())`
- `test_dispatch_send_message_succeeds`: valid `action_config` with a `NoopSender` → `is_ok()`

**Effort:** Medium | **Risk:** Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/dispatcher.rs`
- A `MockMessageSender` or `NoopSender` is needed for the `send_message` tests

## Acceptance Criteria

- [ ] At least 4 test cases covering the paths listed above
- [ ] Tests run with `cargo test` without additional setup
- [ ] No test uses real DB or network (use in-memory SQLite + noop sender)

## Work Log

- 2026-03-06: Identified by test coverage review agent (TEST-7)
