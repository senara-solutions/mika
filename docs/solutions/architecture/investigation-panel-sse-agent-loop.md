---
title: Investigation Panel — SSE Streaming with Read-Only Agent Loop
date: 2026-03-09
category: architecture
tags:
  - observability
  - dashboard
  - sse-streaming
  - read-only-agent
  - investigation
  - tool-sandboxing
  - utf8-truncation
modules:
  - crates/mika-agent/src/server/investigate.rs
  - crates/mika-agent/src/server/state.rs
  - crates/mika-agent/src/server/mod.rs
  - dashboard/src/components/InvestigationPanel.tsx
  - dashboard/src/api/investigate.ts
  - dashboard/src/pages/SessionDetail.tsx
severity: medium
symptoms:
  - Dashboard shows what happened but not why the agent made specific decisions
  - Understanding agent behavior requires mental reconstruction of context
  - No way to ask questions about agent decisions from the observability dashboard
root_cause: >
  The observability dashboard was passive — it displayed events, messages, and
  tool calls but provided no interactive analysis capability. Developers had to
  manually correlate tool inputs/outputs with conversation context to understand
  agent reasoning.
---

# Investigation Panel — SSE Streaming with Read-Only Agent Loop

## Problem

The observability dashboard showed *what* happened (tool calls, inputs, outputs,
sequence) but not *why* an agent made specific decisions. Understanding agent
behavior required mentally reconstructing the conversational context, memory
state, and decision logic — a slow, error-prone process.

## Solution

A dedicated lightweight read-only agent loop accessible from the dashboard via
SSE streaming. Users can ask natural-language questions about agent behavior
("Why did it use curl instead of web_search?") and get contextual answers
powered by Claude with read-only access to the database.

### Architecture

```
Dashboard (React)                    mika-spirit (Axum)
┌──────────────────┐    POST SSE     ┌──────────────────────────┐
│ SessionDetail    │───────────────→ │ handle_investigate       │
│  └─ InvestPanel  │                 │  ├─ Validate request     │
│     ├─ Chat UI   │◄──── SSE ──────│  ├─ Load context from DB │
│     └─ Tool uses │    events       │  ├─ investigation_lock   │
└──────────────────┘                 │  └─ Spawn agent loop     │
                                     │     ├─ Claude API calls  │
                                     │     ├─ 5 read-only tools │
                                     │     └─ SSE event stream  │
                                     └──────────────────────────┘
```

### Key Isolation Boundaries

The investigation agent is completely separate from the main agent:

- **Own lock**: `investigation_lock` (not the main agent lock) — `try_lock`
  returns 429 if busy
- **Stateless**: No session persistence, no message history storage
- **Read-only tools only**: Cannot write to DB, execute skills, or access MCP
- **Lazy initialization**: Tool registry via `OnceCell`, zero cost if unused
- **Minimal ToolContext**: Dummy values for `home_dir`, `session_id`, `trace_id`,
  `message_sender`, `embedding_client` — prevents any side effects

### Constraints

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Max steps | 5 | Investigations are query-heavy, not long chains |
| Total timeout | 120s | Hard cap on entire investigation |
| Per-tool timeout | 10s | Individual query cap |
| Max history turns | 10 | Prevents context explosion in follow-ups |
| Max question length | 4,000 chars | Prevents unbounded input |
| Body limit | 64 KB | Hard cap on request payload |

### Read-Only Tools

| Tool | Purpose |
|------|---------|
| `query_timeline` | Cross-agent event querying (messages, audits, tasks) |
| `query_messages` | Load specific session messages |
| `query_audit_events` | Memory mutation history |
| `search_memory` | FTS5 search across facts/people/preferences |
| `get_agent_info` | Agent stats, core memory, soul preview |

All queries are capped at 50 results. Output summaries use character-based
truncation (see UTF-8 section below).

### SSE Event Protocol

Four event types streamed to the client:

```
event: text_delta     → { text: "..." }           // Incremental text
event: tool_use       → { name, status, summary }  // Tool start/complete
event: error          → { message: "..." }          // Error occurred
event: done           → {}                          // Stream complete
```

### Frontend Integration

**Two trigger points in SessionDetail:**

1. **Message-level**: Search icon on assistant message header → investigates
   the full message context
2. **Tool-call-level**: Search icon on individual tool call row → investigates
   that specific tool execution (step number, input, output, success)

**SSE client uses `fetch()` + `ReadableStream`** (not `EventSource`) because
`EventSource` doesn't support Bearer auth headers. Manual SSE parsing splits
on `\n\n` boundaries and extracts `event:` / `data:` fields.

**Panel UI**: Fixed 40% width (min 400px, max 600px) slide-out from right edge.
Chat thread with user questions and assistant responses. Tool use badges show
running/completed status inline. AbortController cancels in-flight streams on
panel close.

### Context Assembly

The system prompt includes:

1. Target agent's core memory entries
2. Conversation window: 5 messages before target, the target message, 3 after
3. If tool-specific: tool name, step number, input/output summaries
4. Role declaration with read-only access guidance

### UTF-8 Truncation Fix (commit f733334)

All 5 truncation sites in `investigate.rs` were fixed from byte-based to
character-based truncation to prevent panics on multi-byte UTF-8 characters:

```rust
// Before (panics on emoji/arrows/etc.):
format!("{}...", &m.content[..200])

// After (safe):
let truncated: String = m.content.chars().take(200).collect();
format!("{truncated}...")
```

Affected sites: `QueryMessagesTool` (200 chars), `QueryAuditEventsTool`
(100 chars), `GetAgentInfoTool` (500 chars), `build_investigation_context`
(500 chars), `run_investigation` tool output summaries (100 chars).

### Error Handling Strategy

Three layers:

1. **HTTP-level** (before stream): 400 Bad Request, 404 Not Found, 429 Too
   Many Requests — standard JSON error responses
2. **Stream-level**: Claude API errors and timeouts emit `error` event then
   `done` event, closing the stream gracefully
3. **Tool-level**: Errors returned as `ToolOutput::error()` — agent sees the
   error in `tool_result` and can attempt recovery

Client disconnect detection: every `send_event` call checks for `Err` (closed
channel) and exits immediately.

## Agent Mode Taxonomy

This adds a fourth agent mode variant:

| Mode | Tools | Persistence | Output | Use Case |
|------|-------|-------------|--------|----------|
| Conversation | Full | Session/messages | Text to user | Normal chat |
| Silent | safe_always_on | Session/messages | send_message only | Background tasks |
| Callback | Full (no long_running) | Session/messages | Text to user | Task completion |
| **Investigation** | **5 read-only** | **None** | **SSE stream** | **Dashboard analysis** |

## Prevention Strategies

### Read-only agent checklist

When adding a new read-only agent variant:

1. Define a restricted tool set — no write tools, no skills, no MCP
2. Use `try_lock` with 429 for concurrency — don't block the main agent
3. Use `OnceCell` for lazy initialization — zero cost if unused
4. Construct minimal `ToolContext` with dummy values for unused fields
5. Cap all query results (LIMIT clauses)
6. Use character-based truncation for all string previews
7. Set bounded timeouts (both total and per-tool)

### SSE endpoint checklist

1. Use `fetch()` + `ReadableStream` if auth headers are needed (not `EventSource`)
2. Always send `done` event on all exit paths (success, error, timeout)
3. Check `send_event` return for client disconnect
4. Use `AbortController` on the client for cleanup on unmount
5. Include keep-alive comments (`: keep-alive\n\n`) for proxy compatibility

## Related Documentation

- [Observability OTel + TUI Dashboard](observability-otel-tui-dashboard.md)
- [Trace ID Correlation](../architecture-patterns/trace-id-correlation-unified-observability.md)
- [Background Agent Mode Checklist](../code-review-patterns/background-agent-mode-design-checklist.md)
- [Dashboard Investigation Panel Plan](../../plans/2026-03-09-feat-dashboard-investigation-panel-plan.md)
