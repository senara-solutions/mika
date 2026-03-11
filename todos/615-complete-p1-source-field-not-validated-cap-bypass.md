---
status: complete
priority: p1
issue_id: 615
tags: [code-review, security, input-validation]
dependencies: []
---

# source field not validated, allowing per-session cap bypass

## Problem Statement

The `source` parameter in `create_work_item` is free-form text with no allowlist validation. Guard 5 exempts `source == "user_request"` from the 5-item-per-session cap. Since the LLM chooses the `source` value, a confused or jailbroken agent can set `source: "user_request"` on every call, completely bypassing the cap.

The tool schema description lists valid values (`user_request`, `github_issue`, `team_run`, `self_dev`) but the code never enforces them.

## Findings

- **Source**: Security review agent, Pattern review agent, Agent-native review agent
- **Location**: `crates/mika-agent/src/tools/create_work_item.rs` lines 60-67, 122-130
- **Evidence**: No `VALID_SOURCES` allowlist check exists

## Proposed Solutions

### Option A: Add VALID_SOURCES allowlist (Recommended)
```rust
const VALID_SOURCES: &[&str] = &["user_request", "github_issue", "team_run", "self_dev"];
if let Some(src) = source && !VALID_SOURCES.contains(&src) {
    return Ok(ToolOutput::error(...));
}
```
Also add `"enum"` to the JSON schema for `source`.

- **Pros**: Closes bypass, consistent with `update_task_status` VALID_STATUSES pattern
- **Cons**: None
- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] `source` validated against allowlist
- [ ] Invalid source returns error
- [ ] JSON schema includes `enum` constraint
- [ ] Test for invalid source value
