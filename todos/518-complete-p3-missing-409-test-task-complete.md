---
status: complete
priority: p3
issue_id: "518"
tags: [code-review, testing, patterns]
dependencies: []
---

# Missing 409 Conflict Test for `POST /tasks/{id}/complete`

## Problem Statement

The test suite for `POST /tasks/{id}/complete` covers 200 OK, 400 Bad Request, 401 Unauthorized, and 404 Not Found. There is no test for the 409 Conflict path (attempting to complete an already-completed or cancelled task). The handler has this branch; it is not exercised by any test.

## Findings

- **Source**: patterns-reviewer (F-4 Minor)
- **Location**: `crates/mika-agent/src/server/mod.rs` test section (lines 1015-1189)

The handler at `handlers.rs:363-370` validates task status and would return 409 if the task is already completed/cancelled/expired/failed. But once the TOCTOU fix from todo 498 is applied (which adds `AND status IN (...)` to the SQL), the 409 path becomes even more important — it is the primary idempotency signal for duplicate callers.

All other status codes are covered. This gap in test coverage leaves the idempotency behavior unverified.

## Proposed Solutions

### Option A: Add `test_task_complete_already_completed_returns_409` (Recommended)

```rust
#[tokio::test]
async fn test_task_complete_already_completed_returns_409() {
    let (state, _dir) = test_state().await;
    let task_id = create_callback_task(&state).await;

    // Complete it once
    state.agents["default"].db.update_task_completed(&task_id, Some("first")).await.unwrap();

    let app = test_app(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/tasks/{task_id}/complete"))
                .header("Authorization", "Bearer test-token")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"result":"second","agent":""}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}
```

- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] Test `test_task_complete_already_completed_returns_409` exists and passes
- [ ] Test verifies 409 status code when task is already in completed/cancelled/expired/failed state

## Work Log

- 2026-03-06: Identified by patterns-reviewer of feat/unified-task-engine
