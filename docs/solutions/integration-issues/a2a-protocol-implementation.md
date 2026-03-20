---
title: "A2A Protocol v0.3 Implementation for Agent-to-Agent Communication"
category: integration-issues
date: 2026-03-20
tags:
  - a2a
  - protocol
  - agent-to-agent
  - json-rpc
  - streaming
  - sse
  - gateway
  - security
  - sqlite
  - schema-migration
severity: major
component:
  - mika-a2a
  - mika-agent
  - mika-gateway
related_issues:
  - "214"
---

# A2A Protocol v0.3 Implementation

## Problem Context

Mika agents had no standardized way to interoperate with external AI agents built by other teams or vendors. The internal team collaboration model (team runs, `delegate_task`) only worked between agents within the same Mika container. There was no mechanism for an external AI system to discover a Mika agent's capabilities, send it tasks, or receive streaming results — and Mika agents had no way to call out to external agents during their tool loop.

Google's Agent-to-Agent (A2A) protocol v0.3 emerged as a cross-vendor standard for agent interoperability based on JSON-RPC 2.0. Implementing it gave Mika two interoperability directions:

1. **Server role:** External A2A clients can discover Mika agents via Agent Cards, send messages, track task state through a state machine (Submitted → Working → Completed/Failed/Canceled), and receive streaming updates via SSE.

2. **Client role:** Mika agents can call out to any external A2A-compliant agent during their tool loop via a new `a2a_call` builtin tool.

## Solution

### Architecture Overview

```
External caller
    │ x-api-key or Authorization: Bearer <key>
    ▼
mika-gateway  POST /a2a/{customer_id}/{agent_name}
    │ validate API key against a2a_api_keys (Postgres)
    │ check key belongs to customer_id
    │ forward to container with MIKA_INTERNAL_TOKEN
    ▼
mika-server   POST /a2a/{agent_name}   (internal token auth only)
    │ parse JSON-RPC 2.0 envelope
    │ dispatch on method name
    │ acquire agent_lock (same mutex as Telegram messages)
    │ create task + session via a2a_create_task()
    │ run_agent() — full Mika agent loop
    │ persist response, build Task object
    ▼
JSON-RPC 2.0 response or SSE stream
```

### What Was Built

1. **New crate `crates/mika-a2a/`** — standalone protocol library with A2A types, JSON-RPC plumbing, task state machine, SSE streaming types, and HTTP client. No Mika-specific dependencies.

2. **A2A server handlers in `crates/mika-agent/`** — Axum route handlers accepting inbound A2A JSON-RPC requests, running the real agent loop, returning A2A-compliant responses (sync or SSE streaming).

3. **Orthogonal persistence in `crates/mika-agent/src/a2a_db.rs`** — maps A2A tasks onto existing `tasks`, `sessions`, and `messages` tables via a thin `a2a_task_map` bridge table, plus new `a2a_artifacts` and `a2a_push_notification_configs` tables.

4. **Gateway proxy in `crates/mika-gateway/`** — Postgres-backed API key authentication (SHA-256 hashed keys in `a2a_api_keys` table, migration 003) plus transparent JSON-RPC/SSE reverse proxy.

5. **`a2a_call` builtin tool** — outbound tool with SSRF protection (120s timeout) letting agents call remote A2A endpoints.

6. **Agent Card generation in `a2a_card.rs`** — dynamically builds `agent.json` from the running agent's skill registry.

### Key Design Decisions

#### Orthogonal Persistence

A2A tasks piggyback on the existing `tasks` table with `trigger_type='a2a'`; conversations reuse `sessions` + `messages`; only genuinely new data gets new tables:

```sql
CREATE TABLE a2a_task_map (
    a2a_task_id TEXT PRIMARY KEY,
    task_id     TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    context_id  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
```

Bidirectional state mapping translates between internal task statuses and A2A states:

```rust
fn a2a_state_to_task_status(state: &str) -> &'static str {
    match state {
        "submitted" => "pending",
        "working"   => "in_progress",
        "completed" => "completed",
        "failed"    => "failed",
        "canceled"  => "cancelled",
        _           => "pending",
    }
}
```

A2A messages are stored in the unified `messages` table with original `Part[]` arrays preserved in `metadata` JSON as `a2a_parts`.

#### Task State Machine

`TaskStateMachine` encodes the A2A v0.3 state graph as a single exhaustive `matches!` pattern. Transitions are validated before any DB write:

```
Submitted → Working | Canceled | Rejected
Working   → Completed | Failed | Canceled | InputRequired | AuthRequired
InputRequired | AuthRequired → Working | Canceled | Failed
Terminal states (Completed, Failed, Canceled, Rejected) → no transitions
```

#### SSE Streaming

`message/stream` and `tasks/resubscribe` use `tokio::sync::broadcast::channel::<StreamEvent>(32)` per task, stored in `AppState.a2a_broadcasters: Arc<DashMap<...>>`. A `BroadcasterGuard` RAII type ensures cleanup on panic:

```rust
struct BroadcasterGuard {
    map: Arc<DashMap<String, broadcast::Sender<StreamEvent>>>,
    key: String,
}
impl Drop for BroadcasterGuard {
    fn drop(&mut self) { self.map.remove(&self.key); }
}
```

#### SSRF Protection

The `a2a_call` tool rejects: non-http/https schemes, localhost/127.0.0.1/::1, RFC-1918 private IPv4 ranges, link-local addresses, and cloud metadata IP (169.254.169.254).

### Schema Changes

**SQLite v12→v13:** Full `tasks` table rebuild adding `a2a` to trigger_type CHECK. Creates `a2a_task_map`, `a2a_artifacts`, `a2a_push_notification_configs`. Recreates `unified_timeline` VIEW.

**Postgres migration 003:** Creates `a2a_api_keys` table with SHA-256 hashed keys, customer_id binding, expiry, and revocation.

### Endpoints

| Endpoint | Method | Auth | Purpose |
|----------|--------|------|---------|
| `/a2a/{agent_name}` | POST | Internal token | JSON-RPC dispatcher (2MB limit) |
| `/a2a/{agent_name}/agent.json` | GET | Internal token | Agent Card |
| `/a2a/{customer_id}/{agent_name}` | POST | A2A API key | Gateway proxy (2MB limit) |
| `/a2a/{customer_id}/{agent_name}/agent.json` | GET | None | Proxied Agent Card |

## Prevention & Best Practices

### Critical Patterns (from P1 code review findings)

**Agent concurrency — always acquire the global lock.** Every code path that calls `run_*_agent` must acquire `agent_state.agent_lock.try_lock_owned()` first with 429/busy on contention. The A2A handlers initially missed this, risking concurrent SQLite writes.

**SQL parameterization — no exceptions for numeric values.** A LIMIT clause was formatted with `format!()` instead of `rusqlite::params!`. Use `params!` for all values including LIMIT/OFFSET, and clamp user-supplied bounds at the Rust layer.

**SSRF — any tool making outbound HTTP calls requires URL validation.** Create a shared `validate_outbound_url()` helper (like `validate_and_resolve_path` for file tools). Reject private IPs, cloud metadata, non-http(s) schemes. The agent can be instructed by untrusted input to call arbitrary URLs.

**Migration ordering — drop views before rebuilding referenced tables.** `DROP VIEW IF EXISTS` must be the first statement when rebuilding a table referenced by a view. SQLite 3.25+ validates view references during `ALTER TABLE RENAME`.

**Pagination direction — validate against the protocol spec.** For "latest N" queries, use `SELECT * FROM (... ORDER BY id DESC LIMIT ?) ORDER BY id ASC`. The `historyLength` parameter was initially returning the oldest messages instead of the newest.

### Security Patterns

| Risk | Mitigation |
|------|------------|
| SSRF via `a2a_call` | Shared `validate_outbound_url` helper; reject private IPs, cloud metadata, non-http(s) |
| Credential exposure | Named credential store instead of `api_key` in tool JSON schema |
| Concurrent agent execution | Mandatory `try_lock_owned()` at every agent entry point |
| Large payload DoS | Explicit `DefaultBodyLimit::max(N)` on every route group |
| Auth contract mismatch | Agent card and middleware updated atomically |
| API key in LLM history | Never put secrets in tool JSON schemas; redact from tool call summaries |

### General Patterns

| Do | Don't |
|----|-------|
| Acquire `agent_lock` before calling `run_*_agent` | Call agent loop without the lock |
| `rusqlite::params!` for all SQL values | `format!("LIMIT {n}")` even for typed integers |
| RAII guards for spawned-task resource cleanup | `resource.remove()` as last line of closure |
| `pub(crate)` helpers imported across modules | Copy-paste identical functions |
| Typed structs with `#[serde(default)]` for JSON | `serde_json::Value` + `.clone()` for extraction |
| Implement only what a current caller needs | Speculative client library methods with no caller |
| Advertise only working capabilities in Agent Card | `push_notifications: true` before delivery works |

### Testing Gaps

- **`a2a_call` tool:** Add coverage for SSRF-guard logic (private IPs, cloud metadata, non-http schemes)
- **State machine integration:** Drive a task through full lifecycle (Submitted → Working → Completed) and verify illegal transitions return correct JSON-RPC errors
- **SSE event ordering:** Verify streaming clients receive `StatusUpdate(Working)` before completion
- **Migration v12→v13:** Test on populated database with existing tasks to verify data survival

## Related Documentation

### Planning Docs
- `docs/brainstorms/2026-03-19-a2a-protocol-brainstorm.md` — A2A protocol brainstorm
- `docs/plans/2026-03-19-004-feat-implement-a2a-protocol-plan.md` — Implementation plan for issue #214

### Related ADRs
- `docs/adr/001-axum-http-server-architecture.md` — A2A endpoints extend the Axum server
- `docs/adr/004-multi-agent-teams-orchestration.md` — A2A complements internal team orchestration

### Related Solutions
- `docs/solutions/architecture/callback-resume-agent-lifecycle.md` — A2A task resumption follows the same callback/resume pattern
- `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` — Loop prevention guards applicable to A2A task execution
- `docs/solutions/architecture-patterns/trace-id-correlation-unified-observability.md` — Trace ID propagation across agent boundaries
- `docs/solutions/integration-issues/multi-agent-telegram-delivery-and-reply-routing.md` — Related routing pattern for agent identification
- `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md` — Relevant to SSRF/secret leakage concerns

### Code Review Findings
17 findings in `todos/700-716`: 5 P1 (all resolved), 9 P2 (1 pending: #707 a2a_call not in system prompt), 3 P3 (2 pending: #713 SSE race, #716 API key in history).

### GitHub Issues
- **#214** (OPEN) — Implement A2A protocol (v0.3) in mika-server and mika-gateway
