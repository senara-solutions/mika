---
status: pending
priority: p2
issue_id: "435"
tags: [code-review, quality, logging]
dependencies: []
---

# Double Logging in emit_event

## Problem Statement

In `crates/mika-agent/src/teams/engine.rs`, `emit_event()` logs inside match arms AND at the bottom:

```rust
fn emit_event(&self, event: TeamEvent) {
    let msg = match &event {
        TeamEvent::AgentCompleted { agent, .. } => {
            info!(agent, "agent completed");  // logs here
            "agent completed"
        }
        TeamEvent::AgentFailed { agent, error } => {
            warn!(agent, error, "agent failed");  // logs here
            "agent failed"
        }
        // ...
    };
    info!(team = %self.run.team_name, "{msg}");  // also logs here
}
```

`AgentCompleted`, `AgentFailed`, and `RunFailed` produce two log entries each.

## Findings

- Code simplicity reviewer identified this as redundant logging
- Produces noisy logs that make it harder to filter team events

## Proposed Solutions

### Option A: Remove inner match arm logging (Recommended)

Remove the `info!`/`warn!` calls from inside the match arms. The bottom `info!` already covers all events. Add the `agent`/`error` fields to the bottom log line instead.

- **Pros:** Single log entry per event, cleaner logs
- **Cons:** None
- **Effort:** Small
- **Risk:** None

## Technical Details

- **File:** `crates/mika-agent/src/teams/engine.rs`, lines ~718-737
- **Components:** Team event system

## Acceptance Criteria

- [ ] Each `emit_event` call produces exactly one log line
- [ ] Agent name and error details still appear in log output
