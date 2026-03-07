---
status: complete
priority: p3
issue_id: "561"
tags: [code-review, duplication]
dependencies: []
---

# Callback Result Framing Duplicated Between CLI and Server Paths

## Problem Statement

The `<callback_result trust="untrusted">` wrapping logic exists in two places:
1. CLI path: `crates/mika-cli/src/commands/chat.rs:253-259` (inline `format!`)
2. Server path: `crates/mika-agent/src/task_engine/dispatcher.rs` (via `SilentTrigger::Callback`)

If the framing format changes, both must be updated.

## Proposed Solutions

Extract a shared `format_callback_result(label, task_id, result) -> String` helper in `mika-agent`.

**Effort:** Small
