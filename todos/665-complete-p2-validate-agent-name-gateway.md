---
status: pending
priority: p2
issue_id: 665
tags: [code-review, security, gateway, telegram]
dependencies: []
---

# Validate agent_name in Gateway /send Handler

## Problem Statement

The `agent_name` field in `SendPayload` is used without validation in `format!("[{name}] ...")` and stored in the `outbound_messages` table. While the field comes from authenticated internal containers, there is no defense-in-depth validation at the gateway boundary. A malformed agent name (containing `]`, newlines, or excessively long) could produce misleading Telegram output.

## Findings

- `crates/mika-gateway/src/routes.rs:613` — `format!("[{name}] {}", payload.text)` with no sanitization
- `crates/mika-gateway/src/routes.rs:628-635` — stored in DB as unbounded TEXT
- `crates/mika-agent/src/tools/delegate_task.rs:65` — existing `validate_agent_name` function validates agent names elsewhere

Identified by: security-sentinel

## Proposed Solutions

### Option A: Validate in handle_send
Add length (max 64) and character validation (alphanumeric + `-` + `_`) in the `/send` handler before formatting. Return 400 on invalid names.

- **Pros**: Defense-in-depth at trust boundary
- **Cons**: Slight code addition
- **Effort**: Small
- **Risk**: Low

## Acceptance Criteria

- [ ] `agent_name` longer than 64 chars rejected with 400
- [ ] `agent_name` with special characters (`]`, newlines, etc.) rejected with 400
- [ ] Valid agent names still work

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-14 | Identified during security review | agent_name crosses trust boundary |
