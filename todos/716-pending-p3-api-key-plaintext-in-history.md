---
status: pending
priority: p3
issue_id: 716
tags: [code-review, security]
dependencies: []
---

# a2a_call api_key appears in plaintext in conversation history

## Problem Statement
The `a2a_call` tool accepts `api_key` as a plain text parameter in tool input. This means API keys appear in LLM conversation history, tool call logs, and persisted message metadata. The codebase convention is to redact secrets.

## Findings
- `crates/mika-agent/src/tools/a2a_call.rs` lines 39-41: `api_key` in tool schema
- Tool call summaries in `messages.metadata` would contain the key

## Proposed Solutions
**Option A:** Store remote agent credentials in agent config (e.g., `a2a_remotes.json`) and reference by name.
**Option B:** At minimum, redact `api_key` from tool call summaries in message metadata.

## Acceptance Criteria
- [ ] API keys for remote agents are not stored in plaintext in conversation history
