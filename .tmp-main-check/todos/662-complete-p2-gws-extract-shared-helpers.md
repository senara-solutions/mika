---
status: pending
priority: p2
issue_id: 662
tags: [code-review, architecture, quality]
---

# Extract shared CLI validation and subprocess helpers

## Problem Statement

`validate_gws_input` and `validate_gh_input` share ~30 lines of identical validation logic (string rejection, array parsing, empty check, length limit). The spawn-read-wait blocks in `run_gws` and `run_gh` are ~35 lines of identical code. This duplication will compound with each new CLI handler.

## Findings

- **File**: `crates/mika-agent/src/skills/builtin_handlers.rs`
- **Agents**: Simplicity reviewer, Architecture strategist, Pattern recognition specialist
- **Estimated duplication**: ~65 lines

## Proposed Solutions

### Option A: Extract two shared helpers (Recommended)

1. `parse_command_array(input, example_hint) -> Result<Vec<String>, ToolOutput>` — handles steps 1-4 of validation
2. `spawn_and_collect(cmd: tokio::process::Command) -> ToolOutput` — handles spawn, bounded reads, wait, exit-code formatting

Each handler becomes ~20 lines (allowlist check + blocked flags + build Command + call helper).

- Effort: Medium
- Risk: Low

## Acceptance Criteria

- [ ] Shared validation helper extracted
- [ ] Shared subprocess helper extracted
- [ ] Both `run_gh` and `run_gws` use the shared helpers
- [ ] No behavior change (all existing tests pass)
- [ ] Test duplication reduced
