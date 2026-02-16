---
status: complete
priority: p3
issue_id: 289
tags: [code-review, agent-native, follow-up]
dependencies: []
---

# Add agent tools for config read/write (follow-up PR)

## Problem Statement

The `/config set` and `/config` commands are TUI-only. The agent has no tool to read or set customer config values. This is an agent-native parity gap.

## Findings

- **Agent-Native Reviewer:** Orphan feature — `/config set` and `/config` read have no agent tool equivalent. DB plumbing is ready (set_customer_config, get_customer_config, list_customer_config async wrappers exist). Only the tool layer and prompt update are missing.

## Proposed Solutions

### Solution A: Add set_config and get_config tools (follow-up PR)

1. Add `SetConfigTool` — accepts key/value, validates against shared allowlist
2. Add `GetConfigTool` — returns all customer config entries
3. Move `SETTABLE_CONFIG_KEYS` to shared location in mika-agent
4. Update system prompt to mention config tools

- Effort: Medium
- Risk: Low

## Notes

This is out of scope for the current bug-fix PR. Track as follow-up work.
