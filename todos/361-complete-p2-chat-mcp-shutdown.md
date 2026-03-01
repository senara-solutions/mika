---
status: complete
priority: p2
issue_id: "361"
tags: [code-review, mcp, resource-management]
dependencies: []
---

# Chat mode does not shut down MCP connections on exit

## Problem Statement

In `ask` mode, MCP connections are gracefully shut down after the agent run. In `chat` mode, `mcp_manager` is moved into the spawned worker task and simply dropped when the worker exits. This means stdio MCP server child processes get `SIGKILL` instead of graceful MCP protocol shutdown, and HTTP sessions are abandoned.

## Findings

- **Source**: pattern-recognition-specialist review
- **File**: `crates/mika-cli/src/commands/chat.rs`
- **Evidence**: `worker_mcp` moved into worker task at line 93, no `shutdown()` call when worker loop ends
- **Comparison**: `ask.rs` correctly calls `mcp.shutdown().await` at lines 67-69

## Proposed Solutions

### Option A: Add shutdown in worker task before exit (Recommended)

After the `while let Some(request) = user_rx.recv().await` loop ends, call `worker_mcp.shutdown().await`:

```rust
// After the main loop
if let Some(mcp) = worker_mcp {
    mcp.shutdown().await;
}
```

- Effort: Small
- Risk: Very low
- Pros: Simple, consistent with ask mode
- Cons: None

## Recommended Action

Option A

## Technical Details

- **Affected files**: `crates/mika-cli/src/commands/chat.rs`

## Acceptance Criteria

- [ ] MCP connections gracefully shut down when chat mode exits
- [ ] Consistent with ask mode shutdown pattern
- [ ] No impact on normal chat operation

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-01 | Created from code review | |

## Resources

- PR branch: feat/mcp-headers-cli-enable
