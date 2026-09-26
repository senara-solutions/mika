---
status: complete
priority: p1
issue_id: 700
tags: [code-review, architecture, security]
dependencies: []
---

# Missing Agent Lock in A2A Handlers

## Problem Statement

`handle_message_send` and `handle_message_stream` in `server/a2a.rs` do NOT acquire `agent_state.agent_lock` before calling `run_a2a_agent`. The existing `handle_message` handler acquires the lock with `try_lock_owned()` and returns 429 if the agent is busy. Without the lock, A2A requests can run concurrently with Telegram requests, violating the serialization guarantee that prevents data corruption and LLM rate limit exhaustion.

## Findings

- Location: `crates/mika-agent/src/server/a2a.rs` lines 152-298 (message_send) and 301-470 (message_stream)
- `handle_message` correctly acquires the lock, but `handle_message_send` and `handle_message_stream` skip it entirely
- This allows concurrent execution of agent logic from multiple entry points (A2A + Telegram)
- Consequences include potential data corruption in SQLite and LLM rate limit exhaustion

## Proposed Solutions

Acquire `agent_state.agent_lock.clone().try_lock_owned()` at the start of both handlers, returning a JSON-RPC error (INTERNAL_ERROR with "agent busy") if it fails, mirroring the 429 pattern in `handle_message`.

## Acceptance Criteria

- [ ] Both A2A handlers (`handle_message_send` and `handle_message_stream`) acquire `agent_lock` before calling `run_a2a_agent`
- [ ] If the lock cannot be acquired, return a JSON-RPC INTERNAL_ERROR with "agent busy" message
- [ ] Concurrent A2A + Telegram requests are serialized through the same lock
