# Plan: bug(engine) dispatch_arg_match matches branch-name substring on non-dispatch webhook events

**Ticket:** mika issue#1324
**Type:** bug fix
**Date:** 2026-05-28

## Problem Statement

The engine fires `run_claude_pilot_groom` on CI webhook turns (`check_suite.completed`) when the PR branch name contains dispatch-skill substrings like `dev-groom`. This happens because:

1. `self-dev` is always-on and declares `dev-groom` as a dependency
2. When a CI webhook arrives, the skill matcher loads `self-dev-webhook-ci` (keyword match) + `self-dev` (always-on) + `dev-groom` (dependency of self-dev)
3. `dev-groom` registers `run_claude_pilot_groom` into the turn's tool map
4. The LLM sees the branch name contains `dev-groom` and calls `run_claude_pilot_groom` instead of the CI-appropriate `run_claude_pilot`
5. No structural guard blocks `run_claude_pilot_groom` on non-ready-label turns:
   - `dispatch_arg_match` guard (mika#1313): only fires when `ready_label_dispatch_trigger` is true — CI events don't match
   - `webhook_no_unauthorized_dispatch` guard (mika#910): explicitly excludes `[GitHub] Check suite` events (CI skill territory)
   - `validate_dispatch_readiness` in `executor.rs`: checks `is_unauthorized_webhook_dispatch` but CI events are excluded from that predicate

**Observed:** 4 false-positive `run_claude_pilot_groom` dispatches on branch `fix/1318/dev-groom-no-force-push` in 5 minutes. All caught by mika-dev's LLM judgment, not structural guards.

## Root Cause

There is no structural gate that restricts `run_claude_pilot_groom` to ready-label dispatch turns. The CI webhook path legitimately carries `run_claude_pilot` (for CI fix iterations via self-dev-webhook-ci) but `run_claude_pilot_groom` enters the tool map as a side-effect of the self-dev → dev-groom dependency chain, and nothing prevents the LLM from calling it.

## Fix Strategy

Two-layer structural defense: a tool-boundary pre-hoc guard in `executor.rs` (prevents the tool call from executing) and a post-hoc intent guard in `agent.rs` (prevents EndTurn when the tool was called on the wrong event type). Both layers exist for the analogous `run_claude_pilot` case; this extends them to `run_claude_pilot_groom`.

### Layer 1: Tool-boundary guard in `executor.rs` — `validate_dispatch_readiness`

**File:** `crates/mika-agent/src/skills/executor.rs`

Add a new check at the top of `validate_dispatch_readiness` (after the existing `is_unauthorized_webhook_dispatch` check, before the DB checks):

```
// #1324 — Groom-tool webhook-origin guard. run_claude_pilot_groom is only
// legitimate on ready-label dispatch turns (where the issue lacks a grooming
// marker) or operator-direct turns (no [GitHub] prefix). CI and PR webhook
// turns must never trigger grooming.
if is_groom_tool
    && originating_message starts with "[GitHub]"
    && originating_message is NOT a ready_label_dispatch_marker
{
    reject with "groom_tool_wrong_webhook_origin"
}
```

**Implementation detail:**
- Extract the tool name from `tool_input` (the existing `extract_skill_from_input` returns the skill name, but we also need the tool name). The tool name is available from the calling context — either pass it as a parameter or check the skill field: if `skill == "dev-groom"`, the tool is `run_claude_pilot_groom`.
- Actually simpler: add a new parameter `tool_name: &str` to `validate_dispatch_readiness` (the caller in `execute_tool_call` already has it). Check `tool_name == "run_claude_pilot_groom"` directly.

Wait — `validate_dispatch_readiness` is called from the `run_claude_pilot` and `run_claude_pilot_groom` handlers. Let me check the call sites.

**Revised approach:** The function already receives `originating_message: Option<&str>`. The simplest, most surgical fix:

1. Add a `tool_name: &str` parameter to `validate_dispatch_readiness`.
2. At the top of the function (right after the existing `is_unauthorized_webhook_dispatch` check), add:

```rust
// #1324 — Groom-tool origin guard. run_claude_pilot_groom is only
// legitimate on:
// (a) ready-label dispatch turns (issue lacks grooming marker → auto-groom)
// (b) operator-direct turns (no [GitHub] prefix → /mika-groom-ticket)
// (c) callback turns (no [GitHub] prefix)
// CI and PR webhook turns must never trigger grooming — the branch name
// may contain "dev-groom" as a topic slug, not a dispatch signal.
if tool_name == "run_claude_pilot_groom" {
    if let Some(msg) = originating_message {
        if msg.starts_with("[GitHub]")
            && !crate::webhook_dispatch::is_ready_label_dispatch_marker(msg)
        {
            let rejection = serde_json::json!({
                "error": "groom_tool_wrong_webhook_origin",
                "task_id": task_id,
                "reason": "run_claude_pilot_groom is only permitted on ready-label \
                           dispatch turns or operator-direct turns. This turn was \
                           initiated by a non-ready-label [GitHub] webhook event. \
                           The branch name may contain skill-related substrings \
                           (e.g., 'dev-groom') as a topic slug, not a dispatch \
                           signal (mika#1324)."
            });
            record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
            return Err(rejection.to_string());
        }
    }
}
```

3. Update all call sites of `validate_dispatch_readiness` to pass `tool_name`.

### Layer 2: Post-hoc intent guard in `agent.rs` — extend `webhook_no_unauthorized_dispatch`

**File:** `crates/mika-agent/src/agent.rs`

The existing `webhook_no_unauthorized_dispatch` guard prevents EndTurn when `run_claude_pilot` was successfully called on fallthrough webhook turns. But it explicitly excludes CI events via `is_unauthorized_webhook_dispatch` (which returns false for `[GitHub] Check suite` events).

We need a new or extended guard specifically for `run_claude_pilot_groom` on non-ready-label `[GitHub]` turns. Two options:

**Option A (preferred): New intent guard `groom_tool_webhook_origin_guard`**

Add a new `IntentPrecondition` entry in the `INTENT_GUARDS` array:

```rust
IntentPrecondition {
    label: "groom_tool_webhook_origin",
    trigger: groom_tool_webhook_origin_trigger,
    satisfied: groom_tool_webhook_origin_satisfied,
    correction_message: "[mika-engine] run_claude_pilot_groom was called on a \
         non-ready-label [GitHub] webhook turn. This tool is only permitted on \
         [GitHub] Issue labeled ready turns (for auto-groom of ungroomed tickets) \
         or operator-direct turns. CI and PR webhook turns must not trigger \
         grooming — the branch name containing 'dev-groom' is a topic slug, \
         not a dispatch signal (mika#1324). Remove the run_claude_pilot_groom \
         call and handle this turn according to the self-dev-webhook-ci skill \
         instructions instead.",
}
```

Trigger function:
```rust
/// Triggers on [GitHub] webhook turns that are NOT ready-label dispatch
/// AND where run_claude_pilot_groom was attempted.
fn groom_tool_webhook_origin_trigger(msg: &str) -> bool {
    msg.starts_with("[GitHub]")
        && !crate::webhook_dispatch::is_ready_label_dispatch_marker(msg)
}

fn groom_tool_webhook_origin_satisfied(summaries: &[ToolCallSummary]) -> bool {
    // Satisfied (i.e., turn is allowed) when run_claude_pilot_groom was
    // NOT called successfully on this turn. A failed attempt is fine
    // (executor.rs already blocked it structurally).
    !summaries
        .iter()
        .any(|s| s.name == "run_claude_pilot_groom" && s.success)
}
```

**Why Option A over extending the existing guard:** The existing `webhook_no_unauthorized_dispatch` guard has a carefully scoped domain (Webhook Fallthrough — issue events, comments, unknown catchall). CI and PR events are explicitly excluded because those skills legitimately use `run_claude_pilot`. A new guard with a broader trigger (all non-ready-label `[GitHub]` events) but narrower tool check (only `run_claude_pilot_groom`) avoids changing the semantics of the existing guard.

**Option B (rejected): Extend `is_unauthorized_webhook_dispatch` to include CI events when the tool is `run_claude_pilot_groom`**

This would require the predicate to know which tool was called, which breaks its current pure-string-prefix design. Rejected for coupling reasons.

### Layer 3: Regression tests

**File:** `crates/mika-agent/src/agent.rs` (unit test section)

Add tests for the new intent guard:

```rust
#[test]
fn groom_tool_origin_trigger_fires_on_ci_webhook() {
    assert!(groom_tool_webhook_origin_trigger(
        "[GitHub] Check suite completed on fix/1318/dev-groom-no-force-push"
    ));
}

#[test]
fn groom_tool_origin_trigger_fires_on_pr_webhook() {
    assert!(groom_tool_webhook_origin_trigger(
        "[GitHub] PR review submitted on senara-solutions/mika#1318"
    ));
}

#[test]
fn groom_tool_origin_trigger_does_not_fire_on_ready_label() {
    assert!(!groom_tool_webhook_origin_trigger(
        "[GitHub] Issue labeled ready on mika#1324"
    ));
}

#[test]
fn groom_tool_origin_trigger_does_not_fire_on_direct_prompt() {
    assert!(!groom_tool_webhook_origin_trigger(
        "groom mika issue#1324"
    ));
}

#[test]
fn groom_tool_origin_satisfied_when_no_groom_call() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_claude_pilot".to_string(),
        input_summary: "dev-pilot mika#1318".to_string(),
        success: true,
    }];
    assert!(groom_tool_webhook_origin_satisfied(&summaries));
}

#[test]
fn groom_tool_origin_not_satisfied_when_groom_succeeded() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_claude_pilot_groom".to_string(),
        input_summary: "dev-groom mika#1324".to_string(),
        success: true,
    }];
    assert!(!groom_tool_webhook_origin_satisfied(&summaries));
}

#[test]
fn groom_tool_origin_satisfied_when_groom_failed() {
    // Failed attempts are fine — executor.rs blocked it structurally.
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_claude_pilot_groom".to_string(),
        input_summary: "dev-groom mika#1324".to_string(),
        success: false,
    }];
    assert!(groom_tool_webhook_origin_satisfied(&summaries));
}
```

**File:** `crates/mika-agent/src/skills/executor.rs` (test section)

Add a test for the tool-boundary guard:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_groom_tool_rejected_on_ci_webhook_origin() {
    // Simulate a CI webhook turn with run_claude_pilot_groom
    let (db, _dir) = test_async_db().await;
    let task_id = create_test_task(&db).await;
    let result = validate_dispatch_readiness(
        &db,
        &task_id,
        None, // no github token
        None, // no tool input
        Some("[GitHub] Check suite completed on fix/1318/dev-groom-no-force-push"),
        "run_claude_pilot_groom",
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("groom_tool_wrong_webhook_origin"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_groom_tool_allowed_on_ready_label_origin() {
    let (db, _dir) = test_async_db().await;
    let task_id = create_test_task(&db).await;
    // Should not be rejected by the groom-origin guard (may fail on later
    // checks like grooming-marker, but that's expected)
    let result = validate_dispatch_readiness(
        &db,
        &task_id,
        None,
        None,
        Some("[GitHub] Issue labeled ready on mika#1324"),
        "run_claude_pilot_groom",
    )
    .await;
    // If it fails, it should NOT be groom_tool_wrong_webhook_origin
    if let Err(e) = result {
        assert!(!e.contains("groom_tool_wrong_webhook_origin"));
    }
}
```

**File:** `crates/mika-agent/src/webhook_dispatch.rs` (test section)

Add a test documenting the mutual-exclusion between the two guard domains:

```rust
#[test]
fn ci_events_excluded_from_unauthorized_dispatch_but_caught_by_groom_origin_guard() {
    let ci_msg = "[GitHub] Check suite completed on fix/1318/dev-groom-no-force-push";
    // CI events are NOT unauthorized dispatch (ci skill territory)
    assert!(!is_unauthorized_webhook_dispatch(ci_msg));
    // CI events are NOT ready-label dispatch markers
    assert!(!is_ready_label_dispatch_marker(ci_msg));
    // Therefore the groom-origin guard should fire on CI events
    // (the guard fires on [GitHub] && !ready_label_dispatch_marker)
}
```

## Change Summary

| File | Change | Lines (est.) |
|------|--------|-------------|
| `crates/mika-agent/src/skills/executor.rs` | Add `tool_name` param to `validate_dispatch_readiness`; add groom-origin guard; update call sites | ~25 |
| `crates/mika-agent/src/agent.rs` | Add `groom_tool_webhook_origin` intent guard (trigger + satisfied + IntentPrecondition entry) | ~40 |
| `crates/mika-agent/src/agent.rs` (tests) | Regression tests for intent guard | ~50 |
| `crates/mika-agent/src/skills/executor.rs` (tests) | Regression tests for tool-boundary guard | ~40 |
| `crates/mika-agent/src/webhook_dispatch.rs` (tests) | Documentation test for guard domain coverage | ~10 |

**Total:** ~165 lines changed across 3 files.

## Risks and Mitigations

1. **False negative on legitimate auto-groom:** The groom-origin guard allows `run_claude_pilot_groom` on ready-label turns and non-`[GitHub]` turns. These are the only two legitimate paths. Callback turns don't start with `[GitHub]` so they pass through.

2. **`validate_dispatch_readiness` signature change:** Adding `tool_name: &str` is additive and backward-compatible at the call sites. The function is internal (`pub(crate)` or private). All call sites are in the same file.

3. **Intent guard ordering:** The new `groom_tool_webhook_origin` guard is independent of existing guards. It can be placed anywhere in the `INTENT_GUARDS` array. Place it adjacent to `webhook_no_unauthorized_dispatch` for readability.

## Out of Scope

- Removing `dev-groom` from the skill loader on CI webhook turns. This would work but is a bigger change to the skill dependency resolution system and is not necessary given the two structural guards.
- Respecting the "Do NOT auto-dispatch" footer. This is an issue-body-level check that would require a GitHub API call in the guard. The structural fix (blocking `run_claude_pilot_groom` on non-ready-label turns) already prevents the false positive without the API call.
