---
status: complete
priority: p3
issue_id: 641
tags: [code-review, agent-native, observability]
dependencies: []
---

# Tool history loses non-zero exit signal after #100

## Problem Statement

After the fix for #100, `ToolCallSummary.success` is derived from `!output.is_error`. Since non-zero exits now return `ToolOutput::success`, the tool history context (`<context type="tool_history">`) and compaction summaries no longer distinguish between "exited 0" and "exited non-zero". Cross-turn introspection loses this signal.

## Findings

- `agent.rs:949`: `success: !output.is_error` — now always true for completed processes
- `agent.rs:219`: `[FAILED]` tag only appended when `success == false`
- Compaction `extract_tool_names` appends `(err)` the same way
- Current-turn agent sees the exit code in tool result text; cross-turn history does not

## Proposed Solutions

### Option A: Heuristic content check
Check if output content starts with `Exit code:` or `Killed by signal:` to set a `non_zero_exit` flag on `ToolCallSummary`.
- Pros: No struct changes, simple
- Cons: Fragile string matching
- Effort: Small

### Option B: Add exit_code to ToolOutput
Add an `exit_code: Option<i32>` field to `ToolOutput` and propagate through to `ToolCallSummary`.
- Pros: Clean, typed, extensible
- Cons: Touches more code (ToolOutput, agent.rs, compaction)
- Effort: Medium

## Acceptance Criteria

- [ ] Tool history context shows non-zero exit status for shell commands
- [ ] Compaction summaries distinguish successful vs non-zero-exit tool calls
