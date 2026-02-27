---
status: complete
priority: p2
issue_id: "312"
tags: [code-review, agent-native]
dependencies: []
---

# Surface input_summary in the history tool summary block

## Problem Statement

`format_tool_summary_block` renders `[Tools used: tmux_send_command → Command sent]` but omits the `input_summary` field. For the primary motivating use case ("what command did you just send?"), the answer is in the input (e.g., `cargo test --lib`), not the output (e.g., "Command sent"). The input is stored in the metadata JSON but discarded at display time.

Identified by: agent-native-reviewer

## Findings

- `ToolCallSummary.input_summary` is captured and persisted to DB but never surfaced to the agent
- The original bug report was specifically about Mika not being able to say what tmux command it sent
- Current format: `tmux_send_command → Command sent`
- Needed format: `tmux_send_command("cargo test --lib") → Command sent`

## Proposed Solutions

### Option A: Include truncated input in the summary block (Recommended)
```rust
let short_input = truncate_summary(input, 60);
let short_output = truncate_summary(output, 60);
parts.push(format!("{name}({short_input}){status} → {short_output}"));
```
- Pros: Directly addresses the motivating use case
- Cons: Slightly longer history blocks
- Effort: Small

## Technical Details

- **Affected file:** `crates/mika-agent/src/agent.rs:157-179` (format_tool_summary_block)
- **JSON field available:** `input_summary` in the `tool_calls` array entries

## Acceptance Criteria

- [ ] History block includes truncated input for each tool call
- [ ] Agent can answer "what command did you send?" from the history context
- [ ] Existing tests updated to reflect new format

## Work Log

- 2026-02-27: Identified during code review of commit 573596b
