---
status: pending
priority: p2
issue_id: 711
tags: [code-review, quality]
dependencies: []
---

# Dead code: unused A2aClient methods, error variants, params

## Problem Statement
Multiple items in the mika-a2a crate are defined but never called from production code (~280 LOC total). This is premature API surface.

## Findings
Dead code inventory:
- `A2aClient::with_http_client()` (client.rs:29-39, 11 LOC)
- `A2aClient::get_agent_card()` (client.rs:42-49, 8 LOC)
- `A2aClient::send_message_streaming()` + `parse_sse_stream()` (client.rs:72-94 + 182-234, ~80 LOC)
- `A2aClient::get_task()` (client.rs:97-122, 26 LOC)
- `A2aClient::cancel_task()` (client.rs:125-143, 19 LOC)
- `A2aError::TaskNotFound`, `TaskNotCancelable`, `UnsupportedOperation` (error.rs, 9 LOC)
- `A2aMethod::as_str()` (jsonrpc.rs:148-160, 13 LOC)
- `JsonRpcId::Null` (jsonrpc.rs:24, unreachable via deserialization)
- `PushNotificationConfigParams` (params.rs:48-56, 10 LOC)
- `a2a_update_task_metadata()` (a2a_db.rs:135-147, 13 LOC)
- `TaskStateMachine::transition()` (state_machine.rs:42-48, unused outside own tests)

## Proposed Solutions
Remove all unused items. Only keep `A2aClient::new()`, `send_message()`, `build_jsonrpc_request()`, `post_jsonrpc()`.

## Acceptance Criteria
- [ ] No dead code in mika-a2a crate or a2a_db.rs
- [ ] `cargo build` and `cargo test` pass after removal
