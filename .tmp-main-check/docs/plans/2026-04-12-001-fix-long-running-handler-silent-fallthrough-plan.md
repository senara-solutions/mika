---
title: "fix: execute_skill_tool silently falls through long-running handler to sync exec"
type: fix
status: completed
date: 2026-04-12
issue: "#537"
---

# fix: execute_skill_tool silently falls through long-running handler to sync exec

## Overview

When a skill tool declares `handler.long_running = true` but `long_running_ctx` is `None` (callback turns, silent mode, CLI test), `execute_skill_tool` silently falls through to the synchronous `execute_exec` path. The sync path does not inject `__mika_task_id` / `__mika_agent`, causing handlers that expect long-running metadata to exit 1 with a cryptic error that looks like a handler bug, not an engine bug.

## Problem Statement

The `if let` guard at `executor.rs:112-128` matches on **both** `long_running: true` AND `Some(ctx) = long_running_ctx`. When the context is `None`, the entire block is skipped and execution falls through to the timeout-wrapped `execute_inner`, which calls `execute_exec` — a sync path that doesn't provide the long-running metadata the handler expects.

This was introduced intentionally in commit `04ae084c` to block recursion during callback turns, but the implementation chose "silently fall through" instead of "refuse with a clear error".

## Proposed Solution

Add an explicit guard after the long-running dispatch block: if the handler is `ToolHandler::Exec { long_running: true, .. }` and `long_running_ctx` is `None`, return `ToolOutput::error(...)` with a descriptive message naming the tool and explaining the context restriction.

### Implementation

**File: `crates/mika-agent/src/skills/executor.rs`** (`execute_skill_tool` function, ~line 128)

After the existing `if let` block (lines 112-128), add:

```rust
// Refuse long-running tools when no long-running context is available
// (callback turns, silent mode, CLI test). The sync exec path does not
// inject __mika_task_id/__mika_agent, so the handler would crash with
// a cryptic error. Return an explicit error instead.
if matches!(
    &skill_tool.handler,
    ToolHandler::Exec { long_running: true, .. }
) && long_running_ctx.is_none()
{
    warn!(
        tool = %skill_tool.definition.name,
        "long-running tool invoked without long_running_ctx"
    );
    return ToolOutput::error(format!(
        "Tool '{}' is declared long_running but cannot run in the current context \
         (callback turn, silent mode, or CLI test). Long-running tools require a \
         conversation-mode turn with an active task engine.",
        skill_tool.definition.name
    ));
}
```

**File: `crates/mika-cli/src/commands/skills.rs`** (~line 639-645)

No code change needed here — the CLI already passes `None` for `long_running_ctx`, so after the executor fix, `mika skills test <skill> <tool>` for a long-running tool will naturally receive the new error message instead of silently falling through. The existing error display at line 648-649 will print it correctly.

### Unit Test

**File: `crates/mika-agent/src/skills/executor.rs`** (in `mod tests`)

```rust
#[tokio::test]
async fn test_long_running_tool_without_context_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = ResolvedSkillTool {
        definition: ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: "Long-running test tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        },
        handler: ToolHandler::Exec {
            command: "./handlers/run.sh".to_string(),
            long_running: true,
            estimated_duration_secs: Some(300),
        },
        skill_dir: tmp.path().to_path_buf(),
    };

    // Pass None for long_running_ctx — simulates callback turn / silent mode / CLI test
    let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;

    assert!(output.is_error, "expected error, got: {}", output.content);
    assert!(
        output.content.contains("run_claude_pilot"),
        "error should name the tool: {}",
        output.content
    );
    assert!(
        output.content.contains("long_running"),
        "error should mention long_running: {}",
        output.content
    );
}
```

## Acceptance Criteria

- [x] `execute_skill_tool` returns `ToolOutput::error` when handler is `Exec { long_running: true }` and `long_running_ctx` is `None`
- [x] Error message names the tool and explains the context restriction
- [x] Unit test: `long_running: true` + `long_running_ctx = None` → `is_error == true` with expected message
- [x] `mika skills test <skill> <tool>` for a long-running tool produces the same clear error (via the executor fix, no separate CLI change needed)

## Context

- **Key file:** `crates/mika-agent/src/skills/executor.rs:104-153` — `execute_skill_tool` function
- **CLI caller:** `crates/mika-cli/src/commands/skills.rs:639-645` — passes `None` for `long_running_ctx`
- **ToolHandler enum:** `crates/mika-agent/src/skills/manifest.rs:154-170`
- **LongRunningContext:** `crates/mika-agent/src/skills/executor.rs:92-97`
- **Callback turn lr_ctx=None:** `crates/mika-agent/src/agent.rs:1395-1404`
- **Original commit:** `04ae084c` — introduced `lr_ctx = None` for callback turns

## Sources

- GitHub issue: #537
- Related: #520 (GH_TOKEN injection that triggered the discovery)
