---
status: pending
priority: p3
issue_id: 714
tags: [code-review, performance]
dependencies: []
---

# JSON metadata reconstruction uses Value::clone

## Problem Statement
In `a2a_get_messages`, message metadata is recovered by parsing to `serde_json::Value`, then `.clone()` on the `a2a_parts` value before deserializing. This deep-clones the entire JSON tree unnecessarily.

## Findings
- `crates/mika-agent/src/a2a_db.rs` lines 235-278
- `v.clone()` on line 244 creates a deep copy of the parts array JSON

## Proposed Solutions
Define a typed struct `A2aMessageMeta { a2a_message_id: String, a2a_parts: Vec<Part>, a2a_metadata: Option<Value> }` and deserialize directly, eliminating the intermediate Value and clone.

## Acceptance Criteria
- [ ] Message metadata deserialized without intermediate Value::clone
