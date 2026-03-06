---
status: pending
priority: p3
issue_id: "487"
tags: [code-review, security, quality]
dependencies: []
---

# run_team Callback Forwards Unsanitized Error Strings to MessageSender

## Problem Statement

The `run_team` tool's `TeamEventCallback` includes `TeamEvent::AgentFailed { agent, error }`
in user-visible messages as `format!("[Team] Agent '{}' failed: {}", agent, error)`. The
`error` field is an unsanitized `anyhow::Error` chain that may include file paths, internal
state, SQLite errors, or other diagnostic information not intended for end users. This
contradicts the project's existing pattern of opaque error messages at user-facing boundaries
(e.g., `Settings` manual Debug impl, `ClaudeApiError` redaction).

## Findings

- **Source**: security-sentinel review
- **Location**: `crates/mika-agent/src/tools/run_team.rs:94–96`
- Existing pattern for redacting internal errors: `Settings` Debug impl, `ClaudeApiError`

## Proposed Solutions

### Option A: Truncate error string and log detail internally (Recommended)
```rust
TeamEvent::AgentFailed { agent, error } => {
    warn!(agent = %agent, error = %error, "agent failed during team run");
    Some(format!("[Team] Agent '{}' encountered an error", agent))
}
```
- **Effort**: Tiny | **Risk**: None

### Option B: Include truncated error (max 200 chars)
```rust
let short_err = error.chars().take(200).collect::<String>();
Some(format!("[Team] Agent '{}' failed: {}", agent, short_err))
```
- **Effort**: Tiny | **Risk**: Low

## Acceptance Criteria

- [ ] `AgentFailed` event does not forward raw error chains to the user-facing MessageSender
- [ ] Internal error details are logged via `tracing::warn!` instead
- [ ] User receives a useful but non-diagnostic message

## Work Log

- 2026-03-06: Identified by security-sentinel review of feat/unified-task-engine
