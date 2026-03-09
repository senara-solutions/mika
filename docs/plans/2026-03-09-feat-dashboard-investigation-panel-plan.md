---
title: "feat: Dashboard investigation panel"
type: feat
status: completed
date: 2026-03-09
origin: docs/brainstorms/2026-03-09-dashboard-investigation-panel-brainstorm.md
---

# Dashboard Investigation Panel

## Overview

A side panel in the observability dashboard that lets users ask questions about agent behavior — "Why did this agent use curl instead of web_search?" — answered by a dedicated investigation agent with read-only DB tools, streamed via SSE. The investigation agent reuses existing infrastructure (agent loop, tool trait, `AsyncDatabase`) without new architecture.

## Problem Statement / Motivation

When reviewing agent sessions in the dashboard, you can see *what* happened (tool calls, inputs, outputs) but not *why*. Understanding agent decisions currently requires mentally reconstructing the context the agent had, which is error-prone and slow. A built-in investigation capability turns the dashboard from a passive viewer into an active debugging tool — and later becomes the foundation for automated self-improvement (reflection, self-check skills).

(see brainstorm: `docs/brainstorms/2026-03-09-dashboard-investigation-panel-brainstorm.md`)

## Proposed Solution

### Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Dashboard (React)                                      │
│  ┌──────────────────────┐  ┌─────────────────────────┐  │
│  │  SessionDetail page  │  │  InvestigationPanel     │  │
│  │                      │  │  (slide-out, right)     │  │
│  │  [msg] [Investigate] │──│  [chat thread]          │  │
│  │  [tool] [🔍]         │  │  [streaming response]   │  │
│  │                      │  │  [follow-up input]      │  │
│  └──────────────────────┘  └────────┬────────────────┘  │
└──────────────────────────────────────┼──────────────────┘
                              POST /api/v1/investigate
                              (SSE streaming response)
                                       │
┌──────────────────────────────────────┼──────────────────┐
│  mika-server                         ▼                  │
│  ┌─────────────────────────────────────────────────┐    │
│  │  investigate handler                            │    │
│  │  1. Validate request + load context from DB     │    │
│  │  2. Lazy-init investigation agent (once)        │    │
│  │  3. Run agent loop with read-only tools         │    │
│  │  4. Stream text deltas as SSE events            │    │
│  └─────────────┬────────────────┬──────────────────┘    │
│                │                │                        │
│   ┌────────────▼──┐   ┌────────▼────────────┐          │
│   │  ClaudeClient │   │  Read-only tools     │          │
│   │  (shared)     │   │  query_timeline      │          │
│   │               │   │  query_messages      │          │
│   │               │   │  query_audit_events  │          │
│   │               │   │  search_memory       │          │
│   │               │   │  get_agent_info      │          │
│   └───────────────┘   └─────────┬────────────┘          │
│                                 │                        │
│                    ┌────────────▼──────────┐             │
│                    │  dashboard_db         │             │
│                    │  (shared, read-only)  │             │
│                    └──────────────────────-┘             │
└─────────────────────────────────────────────────────────┘
```

### Key Design Decisions

1. **Separate investigation agent** — not one-shot Claude (no tools = limited), not main agent loop (pollutes history, locks agent). Dedicated agent with read-only tools, fully independent of the main agent lock. (see brainstorm)

2. **Side panel UX** — right-side slide-out (~40% width). Main session content stays visible for reference while investigating. Matches Datadog/DevTools/Langfuse pattern. (see brainstorm)

3. **Two trigger levels** — per-tool-call magnifying glass icon + per-message "Investigate" button. Same panel, different initial context depth. (see brainstorm)

4. **SSE streaming** — via `fetch()` + `ReadableStream` (not `EventSource`) for Bearer auth header compatibility. Axum 0.8 has built-in `Sse` support. (see brainstorm)

5. **Lazy creation, stateless** — investigation agent resources (tool registry, system prompt) created on first request, held in `AppState`. Each investigation conversation is ephemeral — no DB writes, no session persistence.

6. **Independent concurrency** — investigation does NOT share the main agent's `agent_lock`. Uses its own `investigation_lock: Arc<Mutex<()>>` with `try_lock` → 429 if busy. Prevents concurrent investigations (one Claude API call at a time) without blocking main agent operations.

## Technical Approach

### Phase 1: Backend — SSE endpoint + investigation agent loop

#### 1.1 Dependencies

**File: `Cargo.toml` (workspace)**

Add `tokio-stream` as an explicit workspace dependency:

```toml
tokio-stream = "0.1"
```

**File: `crates/mika-agent/Cargo.toml`**

Add `tokio-stream` to mika-agent dependencies:

```toml
tokio-stream = { workspace = true }
```

No changes to axum — SSE is in the core crate (`axum::response::sse`).

#### 1.2 SSE event protocol

**File: `crates/mika-agent/src/server/investigate.rs` (new)**

Define the event types as the frontend/backend contract:

```rust
// SSE event types sent to the client
// event: text_delta    data: {"text": "partial response..."}
// event: tool_use      data: {"name": "query_messages", "status": "running"}
// event: tool_result   data: {"name": "query_messages", "status": "completed", "summary": "Found 12 messages"}
// event: done          data: {}
// event: error         data: {"message": "..."}
```

Corresponding TypeScript types in the dashboard:

```typescript
// dashboard/src/api/investigate.ts
type InvestigateEvent =
  | { type: 'text_delta'; text: string }
  | { type: 'tool_use'; name: string; status: 'running' | 'completed'; summary?: string }
  | { type: 'done' }
  | { type: 'error'; message: string }
```

#### 1.3 Request/response types

**File: `crates/mika-agent/src/server/investigate.rs`**

```rust
#[derive(Deserialize)]
struct InvestigateRequest {
    message_id: i64,
    tool_call_index: Option<usize>,  // 0-indexed into tool_calls array
    question: String,                 // max 4,000 chars
    history: Option<Vec<HistoryTurn>>, // max 10 turns
}

#[derive(Deserialize)]
struct HistoryTurn {
    role: String,    // "user" or "assistant"
    content: String,
}
```

Validation:
- `question`: non-empty, max 4,000 chars
- `history`: max 10 turns (silently drop older if more sent)
- `message_id`: must exist in DB (404 before stream starts)
- `tool_call_index`: if provided, must be within bounds of the message's tool_calls metadata (400 before stream starts)
- Body size limit: 64KB (via `tower_http::limit::RequestBodyLimitLayer`)

#### 1.4 Context loading

**File: `crates/mika-agent/src/server/investigate.rs`**

Given `message_id` M and optional `tool_call_index` I:

1. Load message M from DB → 404 if not found
2. Load the session for message M
3. Load the "turn context": the preceding user message + message M (assistant) + any tool_result messages after M, up to the next user message
4. Load up to 5 messages before the user message for conversational context
5. If `tool_call_index` is provided, extract that specific tool call from M's metadata JSON
6. Load the agent's core memory and soul.md (via `resolve_agent` on the session's `agent_id`)

Assemble into a system prompt section:

```
You are an investigation assistant analyzing Mika agent behavior.
You have read-only access to the database to help answer questions
about why the agent made certain decisions.

## Agent Under Investigation
Agent: {agent_id}
Session: {session_id}

## Core Memory
{core_memory_content}

## Context Being Investigated
{turn_messages}

{if tool_call_index:}
## Specific Tool Call
Tool: {name}
Input: {input_summary}
Output: {output_summary}
Success: {success}
{end if}
```

#### 1.5 Read-only investigation tools

**File: `crates/mika-agent/src/investigate/tools.rs` (new)**

Five tools, all wrapping existing `AsyncDatabase` query methods:

| Tool | Parameters | Maps to | Purpose |
|------|-----------|---------|---------|
| `query_timeline` | `agent_id?, event_type?, trace_id?, limit (default 20, max 50)` | `AsyncDatabase::query_timeline` | Browse unified timeline events |
| `query_messages` | `session_id, limit (default 20, max 50), before_id?` | `AsyncDatabase::load_session_messages_paginated` | Load messages from a session |
| `query_audit_events` | `agent_id, limit (default 20, max 50)` | `AsyncDatabase::list_audit_events_paginated` | Memory mutation history |
| `search_memory` | `agent_id, query` | `AsyncDatabase::fts_search` (FTS5-only, no vectors) | Search structured facts |
| `get_agent_info` | `agent_id` | `AsyncDatabase::get_agent_with_stats` + core memory load | Agent identity and memory |

All tools:
- Accept JSON input with the params above
- Return text output (formatted query results)
- Use `dashboard_db` from `AppState` (shared, unscoped)
- For agent-scoped queries (`search_memory`, `get_agent_info`): resolve agent DB via `AppState::resolve_agent()`
- Have 10-second per-tool timeout (override via `timeout_secs()`)
- Validate inputs (empty check, length limits)
- Include `LIMIT` caps on all queries to prevent unbounded results

No write tools, no skills, no MCP, no management tools.

#### 1.6 Investigation agent loop

**File: `crates/mika-agent/src/investigate/mod.rs` (new)**

A lightweight agent loop that does NOT use `run_agent` (too heavy — sessions, compaction, task engine). Instead, a purpose-built loop:

```rust
pub async fn run_investigation(
    claude: &ClaudeClient,
    tools: &ToolRegistry,
    system_prompt: String,
    messages: Vec<Message>,  // history + current question
    tx: mpsc::Sender<InvestigateEvent>,
    db: AsyncDatabase,
) -> Result<()> {
    // Max 5 tool steps (investigations should be quick)
    // 2-minute total timeout
    // 10-second per-tool timeout
    // On each text response: send text_delta events via tx
    // On each tool_use: send tool_use event, execute, send tool_result event
    // On done/error: send done/error event
    // Detect client disconnect via tx.send() returning Err
}
```

The loop calls `claude.send_message()` (non-streaming — the Claude API client doesn't support streaming yet). The full response text is sent as a single `text_delta` event. Tool calls are dispatched inline. This is simpler than true token-by-token streaming but still gives the SSE event structure for the frontend to display incrementally. True Claude streaming can be added later as an enhancement.

#### 1.7 SSE handler + routing

**File: `crates/mika-agent/src/server/investigate.rs`**

```rust
pub async fn handle_investigate(
    State(state): State<AppState>,
    Json(req): Json<InvestigateRequest>,
) -> Result<impl IntoResponse, Response> {
    // 1. Validate request
    // 2. Load context from DB (404/400 if invalid)
    // 3. Try investigation_lock (429 if busy)
    // 4. Create mpsc channel
    // 5. Spawn investigation agent loop task
    // 6. Return Sse::new(ReceiverStream::new(rx)).keep_alive(...)
}
```

**File: `crates/mika-agent/src/server/mod.rs`**

- Add `mod investigate;` declaration
- Add route: `.route("/investigate", post(investigate::handle_investigate))`
- Update CORS to allow POST: `.allow_methods([Method::GET, Method::OPTIONS, Method::POST])`
- Add body size limit on the investigate route: 64KB
- Update CORS comment

**File: `crates/mika-agent/src/server/state.rs`**

Add to `AppState`:

```rust
pub investigation_lock: Arc<tokio::sync::Mutex<()>>,
pub investigation_tools: Arc<OnceCell<ToolRegistry>>,  // lazy init
```

#### 1.8 Error handling

- **message_id not found**: HTTP 404 JSON response (before SSE starts)
- **tool_call_index out of range**: HTTP 400 JSON response (before SSE starts)
- **investigation_lock busy**: HTTP 429 JSON response
- **Claude API 429/500/529**: Send `error` SSE event with message, close stream
- **Tool execution failure**: Send `tool_result` event with error summary, agent continues
- **Agent exceeds 5 tool steps**: Send `text_delta` with summary, then `done`
- **2-minute timeout**: Send `error` event "Investigation timed out", close stream
- **Client disconnect**: `tx.send()` returns `Err`, break loop, drop resources

### Phase 2: Frontend — Side panel + SSE client

#### 2.1 SSE client utility

**File: `dashboard/src/api/investigate.ts` (new)**

```typescript
export interface InvestigateRequest {
  message_id: number
  tool_call_index?: number
  question: string
  history?: { role: string; content: string }[]
}

export type InvestigateEvent =
  | { type: 'text_delta'; text: string }
  | { type: 'tool_use'; name: string; status: 'running' | 'completed'; summary?: string }
  | { type: 'done' }
  | { type: 'error'; message: string }

export async function streamInvestigation(
  req: InvestigateRequest,
  onEvent: (event: InvestigateEvent) => void,
  signal: AbortSignal,
): Promise<void> {
  // POST to /api/v1/investigate with Bearer auth
  // Parse SSE from response.body ReadableStream
  // Call onEvent for each parsed event
  // Respect AbortSignal for cancellation
}
```

Uses `fetch()` + `ReadableStream` + `TextDecoder` (not `EventSource`) for Bearer auth support. SSE parsing: split on `\n\n`, extract `event:` and `data:` fields. Keep-alive comments (`:` prefix) are silently ignored.

#### 2.2 Investigation panel component

**File: `dashboard/src/components/InvestigationPanel.tsx` (new)**

Right-side slide-out panel, ~40% width. Structure:

```
┌──────────────────────────────┐
│  🔍 Investigation    [✕]     │  ← header with close button
│──────────────────────────────│
│  Context: run_shell step 1   │  ← what's being investigated
│  Session: abc123             │
│──────────────────────────────│
│                              │
│  [user] Why did Mika use     │  ← chat thread (scrollable)
│         curl instead of      │
│         web_search?          │
│                              │
│  [agent] Looking at the      │  ← streaming response
│          context...          │
│    🔧 query_messages ✓       │  ← tool use indicator
│          The agent used      │
│          curl because...     │
│                              │
│  [user] Is there a skill     │  ← follow-up
│         for this?            │
│                              │
│──────────────────────────────│
│  [Ask a question...]   [→]  │  ← input + send button
└──────────────────────────────┘
```

State management: local `useState` — no React Query (streaming doesn't fit query/cache model). Conversation history array of `{ role, content }` turns.

Key behaviors:
- **Close**: X button, Escape key. Aborts in-flight stream via `AbortController`.
- **Streaming indicator**: Pulsing dot or spinner while response is streaming.
- **Tool use display**: Inline badges showing tool name + status (running → completed).
- **Follow-ups**: Append previous turns to `history` field in next request.
- **Auto-scroll**: Scroll to bottom as new text_delta events arrive.
- **Error display**: Inline error message in the chat thread (not a toast/modal).
- **Empty state**: "Ask a question about this agent's behavior" placeholder.

#### 2.3 Trigger integration

**File: `dashboard/src/pages/SessionDetail.tsx`**

Two trigger points, both opening the same `InvestigationPanel`:

1. **Tool call row**: Add a magnifying glass icon (`Search` from lucide-react) to each tool call row. On click, opens panel with `{ message_id, tool_call_index }`.

2. **Assistant message bubble**: Add an "Investigate" button to assistant message bubbles (next to the timestamp area). On click, opens panel with `{ message_id }`.

Panel state managed via `useState<InvestigationContext | null>` in `SessionDetail`:

```typescript
interface InvestigationContext {
  messageId: number
  toolCallIndex?: number
  toolName?: string      // for display in panel header
  sessionId: string      // for display
  agentId: string        // for display
}
```

#### 2.4 Layout integration

**File: `dashboard/src/pages/SessionDetail.tsx`**

The panel renders as a fixed-position overlay on the right side of the viewport (not inside the layout grid). This avoids modifying `Layout.tsx` and keeps the panel scoped to `SessionDetail`.

```tsx
{investigationCtx && (
  <InvestigationPanel
    context={investigationCtx}
    onClose={() => setInvestigationCtx(null)}
  />
)}
```

Panel styling: `fixed top-0 right-0 h-full w-[40%] min-w-[400px]` with a semi-transparent backdrop on the left.

## Acceptance Criteria

- [x] `POST /api/v1/investigate` endpoint accepts `{ message_id, tool_call_index?, question, history? }`
- [x] Returns SSE stream with `text_delta`, `tool_use`, `tool_result`, `done`, `error` events
- [x] Investigation agent has 5 read-only tools: `query_timeline`, `query_messages`, `query_audit_events`, `search_memory`, `get_agent_info`
- [x] Investigation agent cannot write to the DB (no write tools registered)
- [x] 404 returned for invalid `message_id`, 400 for out-of-range `tool_call_index`
- [x] 429 returned if another investigation is already running
- [x] Investigation agent lazy-initialized on first request
- [x] 2-minute total timeout, 5 tool step limit, 10-second per-tool timeout
- [x] Magnifying glass icon on tool call rows opens side panel with tool-specific context
- [x] "Investigate" button on assistant messages opens side panel with message-level context
- [x] Side panel shows streaming chat thread with follow-up support
- [x] Panel close (X / Escape) aborts in-flight stream via `AbortController`
- [x] History capped at 10 turns, question capped at 4,000 chars
- [x] `cargo test` passes, `cargo clippy` clean, `npm run build --prefix dashboard` clean

## Dependencies & Risks

**Dependencies:**
- `tokio-stream` crate (already a transitive dependency, needs explicit addition)
- Claude API key available in server config (existing requirement)
- `dashboard_db` in `AppState` (existing)

**Risks:**
- **Claude API cost**: Each investigation is 1+ API calls. No rate limiting in v1 — acceptable for single user, revisit if exposed to multiple users.
- **Non-streaming Claude client**: The current `ClaudeClient` doesn't support streaming. Each agent turn sends the full response as one `text_delta` event. True token streaming is a future enhancement.
- **Context window**: Long sessions + many follow-ups could exceed Claude's context. Mitigated by: capping history to 10 turns, loading only the relevant turn + 5 messages of surrounding context.

## Sources & References

- **Origin brainstorm:** [docs/brainstorms/2026-03-09-dashboard-investigation-panel-brainstorm.md](docs/brainstorms/2026-03-09-dashboard-investigation-panel-brainstorm.md) — key decisions: side panel UX, separate stateless investigation agent, SSE streaming, lazy creation, both trigger levels
- Axum SSE: `axum::response::sse::{Sse, Event, KeepAlive}` (built into axum 0.8 core)
- Server router: `crates/mika-agent/src/server/mod.rs` (lines 44-93)
- Dashboard handlers: `crates/mika-agent/src/server/dashboard.rs`
- AppState: `crates/mika-agent/src/server/state.rs` (line 37)
- Dashboard DB queries: `crates/mika-agent/src/async_db.rs` (lines 1027-1115)
- Tool trait: `crates/mika-agent/src/tools/mod.rs` (line 78)
- Agent loop: `crates/mika-agent/src/agent.rs` (line 576)
- Background agent mode checklist: `docs/solutions/code-review-patterns/background-agent-mode-design-checklist.md`
