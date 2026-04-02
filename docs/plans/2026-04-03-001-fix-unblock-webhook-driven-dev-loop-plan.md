---
title: "fix: unblock webhook-driven dev loop"
type: fix
status: active
date: 2026-04-03
---

# fix: unblock webhook-driven dev loop

## Overview

Two fixes bundled into one PR to unblock the GitHub App webhook-driven dev loop (mika-dev → mika-qa coordination via webhooks instead of delegate_task). Both are small, independent code changes with no interaction between them.

- **#403** — Remove dead `app_id` field from `GitHubInstallation` struct (gateway webhook parse failure)
- **#386** — Raise conversation-mode `MAX_TOOL_STEPS` from 10 to 20 (autonomous dispatch step budget)

## Fix 1: Remove `app_id` from `GitHubInstallation` (#403)

### Problem

Gateway logs show `WARN GitHub webhook body parse failed error=missing field 'app_id'` — some GitHub webhook event types send a minimal `installation` object (`{"id": N}`) without `app_id`. Since `app_id: u64` is a required field on `GitHubInstallation`, serde deserialization fails and the webhook returns 400 BAD_REQUEST. The event is lost.

The `app_id` field is dead code — its only consumer (the bot self-event filter) was removed in #401/#402 (commit `f066a58`). No code anywhere reads `installation.app_id`.

### Change

**File:** `crates/mika-gateway/src/github.rs`, line 82

```rust
// BEFORE
pub struct GitHubInstallation {
    pub id: u64,
    pub app_id: u64,  // ← delete this line
}

// AFTER
pub struct GitHubInstallation {
    pub id: u64,
}
```

**Serde behavior note:** After removing `app_id`, payloads that still include `app_id` will parse correctly — serde's default `#[derive(Deserialize)]` ignores unknown fields. No `deny_unknown_fields` attribute is present on this struct.

### Test

Add one integration test with a realistic payload containing `"installation": {"id": 123}` (no `app_id`) to prevent regression. The existing tests all use `installation: None` in their event construction.

```rust
#[test]
fn test_github_webhook_minimal_installation_parses() {
    // Payload with installation object that only has "id" (no app_id)
    // Regression test for #403
    let payload = serde_json::json!({
        "action": "opened",
        "issue": { "number": 1, "title": "test", "body": "test body",
                   "html_url": "https://github.com/org/repo/issues/1",
                   "user": { "login": "testuser" } },
        "repository": { "full_name": "org/repo" },
        "sender": { "login": "testuser" },
        "installation": { "id": 12345 }
    });
    let event: GitHubWebhookEvent = serde_json::from_value(payload).unwrap();
    assert_eq!(event.installation.unwrap().id, 12345);
}
```

## Fix 2: Raise `MAX_TOOL_STEPS` from 10 to 20 (#386)

### Problem

mika-dev hits the 10-step conversation-mode limit before completing the autonomous dispatch sequence (fetch issue → research code → create work item → launch claude-pilot). This requires ~10-12 steps minimum. The agent gets cut off during research and never reaches task creation.

Callback mode already has 20 steps (PR #378). Team mode also has 20. Conversation mode is the outlier at 10.

### Change

**File:** `crates/mika-agent/src/agent.rs`, line 31

```rust
// BEFORE
const MAX_TOOL_STEPS: usize = 10;

// AFTER
const MAX_TOOL_STEPS: usize = 20;
```

### Impact Analysis

| Mode | Before | After | Notes |
|------|--------|-------|-------|
| Conversation | 10 | **20** | Primary fix target |
| Silent Heartbeat | 10 | **20** | Accepted side effect |
| Silent Reflection | 10 | **20** | Accepted side effect |
| Silent SkillRun | 10 | **20** | Accepted side effect |
| Silent Callback | 20 | 20 | No change (uses `MAX_CALLBACK_TOOL_STEPS`) |
| Silent Reminder | 20 | 20 | No change (uses `MAX_CALLBACK_TOOL_STEPS`) |
| Team | 20 | 20 | No change (uses `MAX_TEAM_TOOL_STEPS`) |

**Silent mode budget increase (intentional):** Heartbeat/reflection/skill_run going from 10 to 20 is an accepted side effect. The agent only uses as many steps as it needs — a higher ceiling does not force more steps. A heartbeat that today uses 3 of 10 steps will still use 3 of 20. The step-awareness nudge moves from step 8 to step 18, but this only matters if the agent approaches the limit.

**Total timeout:** `AGENT_TOTAL_TIMEOUT_SECS` stays at 300s. With 20 steps at a 30s per-tool default, the theoretical worst case (600s) exceeds the timeout. In practice, most tool calls complete in 1-5 seconds, so 20 steps typically takes 60-120 seconds. The 5-minute timeout is a safety net for pathological cases. Accept the asymmetry.

**Constant consolidation:** All three constants (`MAX_TOOL_STEPS`, `MAX_CALLBACK_TOOL_STEPS`, `MAX_TEAM_TOOL_STEPS`) are now 20. Keep them separate — they document different modes' step budgets and may diverge again. No consolidation needed.

### Tests

All existing tests use the constant names (not hardcoded `10`), so they pass without modification:
- `test_loop_mode_silent_properties` — asserts `max_steps() == MAX_TOOL_STEPS` ✓
- `test_silent_trigger_non_callback_gets_default_step_limit` — asserts against `MAX_TOOL_STEPS` ✓
- `test_silent_trigger_callback_gets_higher_step_limit` — asserts against `MAX_CALLBACK_TOOL_STEPS` ✓

**Semantic note:** The test name `test_silent_trigger_callback_gets_higher_step_limit` becomes vacuously true (callback and default are both 20). The test still passes and the name documents intent. Update the doc comment on `SilentTrigger::max_steps()` to remove "higher budget" language since all modes now share the same budget.

## Acceptance Criteria

- [x] Gateway parses GitHub webhook events with minimal `installation` objects (no `app_id`)
- [x] Gateway parses GitHub webhook events that still include `app_id` (serde ignores it)
- [x] Conversation-mode agent loop allows up to 20 tool steps
- [x] All existing tests pass (`cargo test`)
- [x] New deserialization test for minimal installation payload

## Sources

- #403 — `GitHubInstallation.app_id` should be optional
- #386 — conversation-mode max_steps=10 too low for autonomous task dispatch
- #405 — umbrella issue
- PR #378 — callback max_steps fix (prior art)
- PR #401/#402 — bot self-event filter removal (made `app_id` dead code)
- `docs/solutions/runtime-errors/silent-callback-max-steps-exhaustion.md` — learnings on step budgets
- `docs/solutions/runtime-errors/reminder-trigger-max-steps-exhaustion.md` — learnings on exhaustive match arms
