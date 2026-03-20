---
status: pending
priority: p2
issue_id: 710
tags: [code-review, architecture]
dependencies: []
---

# Push notification subsystem without delivery (YAGNI)

## Problem Statement
Full push notification config CRUD is implemented (~250 LOC across DB/async/server) but no code actually sends push notifications. The agent card misleadingly advertises `push_notifications: Some(true)`. This is premature code with no functional purpose.

## Findings
- DB layer: `a2a_set_push_config`, `a2a_get_push_config`, `a2a_list_push_configs`, `a2a_delete_push_config` (~90 LOC)
- AsyncDB wrappers (~56 LOC)
- Server handlers: 4 handlers (`handle_push_config_set/get/list/delete`) (~130 LOC)
- SQLite table: `a2a_push_notification_configs`
- Agent card: `push_notifications: Some(true)` is misleading

## Proposed Solutions
**Option A (Recommended):** Remove the entire push notification subsystem (~250 LOC) and set `push_notifications: Some(false)`. Add back when delivery is built.
**Option B:** Keep but set `push_notifications: Some(false)` until delivery is implemented.

## Acceptance Criteria
- [ ] Agent card accurately reflects push notification capability
- [ ] No dead CRUD code for unimplemented features (if Option A)
