---
issue: 214
type: feat
title: Implement A2A Protocol (v0.3) in mika-spirit and mika-gateway
created: 2026-03-19
status: in-progress
---

# Plan: Implement A2A Protocol (v0.3)

## Summary

Implement Google's Agent-to-Agent (A2A) protocol v0.3 enabling Mika agents to both **serve** (accept A2A requests) and **consume** (call external A2A agents) interactions. This adds a new `mika-a2a` crate for protocol types, integrates server-side A2A handling into `mika-agent`, and adds gateway proxy routes with API key authentication into `mika-gateway`.

## Architecture

```
External A2A Client
       │
       ▼
┌─────────────────┐     ┌──────────────────┐
│  mika-gateway   │────▶│   mika-agent     │
│  (A2A proxy)    │     │   (A2A server)   │
│  - API key auth │     │   - JSON-RPC     │
│  - SSE pass     │     │   - Task SM      │
│  - Agent Card   │     │   - Agent Card   │
│    routing      │     │   - SSE stream   │
└─────────────────┘     │   - a2a_call     │
                        │     tool         │
                        └──────────────────┘
                               │
                               ▼
                        External A2A Server
                        (via a2a_call tool)
```

### Crate Dependencies

```
mika-a2a (new) ← mika-agent ← mika-gateway
                ← mika-agent (client via a2a_call tool)
```

## Phase 1: `mika-a2a` Crate — Protocol Types & Client

### 1.1 Create crate scaffold

- New crate at `crates/mika-a2a/`
- Add to workspace `Cargo.toml` members (automatic via `crates/*` glob)
- Dependencies: `serde`, `serde_json`, `thiserror`, `uuid`, `chrono`, `reqwest`, `tokio`, `tokio-stream`, `bytes`, `tracing`

### 1.2 A2A types (`src/types.rs`)

All types derive `Serialize`, `Deserialize`, `Debug`, `Clone`. Follow the A2A v0.3 spec exactly:

- **`Part`** — enum with `Text { text, metadata }`, `File { file, metadata }`, `Data { data, metadata }` variants. Tag: `kind`.
- **`FileContent`** — struct with `name`, `mime_type`, `bytes` (Option<String> base64), `url` (Option<String>)
- **`Message`** — struct with `message_id`, `role` (enum `user`/`agent`), `parts`, `context_id`, `task_id`, `metadata`, `reference_task_ids`, `extensions`
- **`Artifact`** — struct with `artifact_id`, `name`, `description`, `parts`, `metadata`, `extensions`
- **`TaskState`** — enum: `Unknown`, `Submitted`, `Working`, `InputRequired`, `AuthRequired`, `Completed`, `Failed`, `Canceled`, `Rejected`. Rename serialization to kebab-case.
- **`TaskStatus`** — struct with `state`, `message` (Option), `timestamp`
- **`Task`** — struct with `id`, `context_id`, `status`, `artifacts`, `history`, `metadata`
- **`AgentCard`** — struct with `name`, `description`, `version`, `url`, `provider`, `capabilities`, `security_schemes`, `security_requirements`, `default_input_modes`, `default_output_modes`, `skills`, `icon_url`, `documentation_url`
- **`AgentCapabilities`** — struct with `streaming`, `push_notifications`
- **`AgentSkill`** — struct with `id`, `name`, `description`, `tags`, `examples`, `input_modes`, `output_modes`
- **`AgentProvider`** — struct with `organization`, `url`
- **`TaskPushNotificationConfig`** — struct with `id`, `task_id`, `url`, `token`, `authentication`
- **`AuthenticationInfo`** — struct with `scheme`, `credentials`

### 1.3 JSON-RPC 2.0 types (`src/jsonrpc.rs`)

- **`JsonRpcRequest`** — struct with `jsonrpc` (always "2.0"), `method`, `params` (serde_json::Value), `id` (Option<JsonRpcId>)
- **`JsonRpcId`** — enum: `Number(i64)`, `String(String)`, `Null`
- **`JsonRpcResponse`** — struct with `jsonrpc`, `result` (Option), `error` (Option), `id`
- **`JsonRpcError`** — struct with `code`, `message`, `data` (Option)
- **`A2aMethod`** — enum mapping method strings to variants:
  - `MessageSend` → `"message/send"`
  - `MessageStream` → `"message/stream"`
  - `TasksGet` → `"tasks/get"`
  - `TasksCancel` → `"tasks/cancel"`
  - `TasksResubscribe` → `"tasks/resubscribe"`
  - `TasksPushNotificationConfigSet` → `"tasks/pushNotificationConfig/set"`
  - `TasksPushNotificationConfigGet` → `"tasks/pushNotificationConfig/get"`
  - `TasksPushNotificationConfigList` → `"tasks/pushNotificationConfig/list"`
  - `TasksPushNotificationConfigDelete` → `"tasks/pushNotificationConfig/delete"`
- **Error code constants** matching A2A spec: `PARSE_ERROR`, `INVALID_REQUEST`, `METHOD_NOT_FOUND`, `INVALID_PARAMS`, `INTERNAL_ERROR`, `TASK_NOT_FOUND`, `TASK_NOT_CANCELABLE`, `PUSH_NOTIFICATION_NOT_SUPPORTED`, `UNSUPPORTED_OPERATION`, `CONTENT_TYPE_NOT_SUPPORTED`, `INVALID_AGENT_RESPONSE`
- Helper: `JsonRpcError::from_code(code) -> Self` with default messages

### 1.4 Request/Response types (`src/params.rs`)

- **`MessageSendParams`** — struct with `message` (Message), `configuration` (Option<SendMessageConfiguration>), `metadata`
- **`SendMessageConfiguration`** — struct with `accepted_output_modes`, `task_push_notification_config`, `history_length`, `return_immediately`
- **`TaskQueryParams`** — struct with `id`, `history_length`
- **`TaskIdParams`** — struct with `id`
- **`TaskPushNotificationConfigParams`** — for set/get/delete operations

### 1.5 Streaming types (`src/streaming.rs`)

- **`StreamEvent`** — enum:
  - `Task(Task)` — kind: `"task"`
  - `Message(Message)` — kind: `"message"`
  - `StatusUpdate(TaskStatusUpdateEvent)` — kind: `"status-update"`
  - `ArtifactUpdate(TaskArtifactUpdateEvent)` — kind: `"artifact-update"`
- **`TaskStatusUpdateEvent`** — struct with `task_id`, `context_id`, `status`, `final_` (bool), `metadata`
- **`TaskArtifactUpdateEvent`** — struct with `task_id`, `context_id`, `artifact`, `append`, `last_chunk`, `metadata`

### 1.6 Task state machine (`src/state_machine.rs`)

- `TaskStateMachine` with validation of state transitions
- Terminal states: `Completed`, `Failed`, `Canceled`, `Rejected`
- Allowed transitions:
  - `Submitted → Working | Canceled | Rejected`
  - `Working → Completed | Failed | Canceled | InputRequired | AuthRequired`
  - `InputRequired → Working | Canceled | Failed`
  - `AuthRequired → Working | Canceled | Failed`
- `can_transition(from, to) -> bool`
- `is_terminal(state) -> bool`

### 1.7 A2A HTTP client (`src/client.rs`)

- **`A2aClient`** struct wrapping `reqwest::Client`
- Constructor: `new(base_url, auth_token)` — sets Bearer auth header
- Methods:
  - `send_message(params) -> Result<Task>` — POST JSON-RPC `message/send`
  - `send_message_streaming(params) -> Result<impl Stream<Item = Result<StreamEvent>>>` — POST `message/stream`, parse SSE
  - `get_task(id, history_length) -> Result<Task>` — `tasks/get`
  - `cancel_task(id) -> Result<Task>` — `tasks/cancel`
  - `get_agent_card(url) -> Result<AgentCard>` — GET `/.well-known/agent.json`
- SSE parsing: split on `\n\n`, parse `data:` lines as JSON `StreamEvent`
- Error mapping: JSON-RPC error → `A2aClientError` enum

### 1.8 Error types (`src/error.rs`)

- `A2aError` enum (thiserror):
  - `InvalidStateTransition { from, to }`
  - `TaskNotFound(String)`
  - `InvalidJsonRpc(String)`
  - `ClientError(reqwest::Error)`
  - `SerializationError(serde_json::Error)`
  - `TaskNotCancelable(String)`
  - `UnsupportedOperation(String)`

### 1.9 Module structure (`src/lib.rs`)

```rust
pub mod types;
pub mod jsonrpc;
pub mod params;
pub mod streaming;
pub mod state_machine;
pub mod client;
pub mod error;
```

Re-export key types from root.

## Phase 2: A2A Server in `mika-agent`

### 2.1 SQLite schema (migration v13)

New tables for A2A task storage, isolated from internal tasks:

```sql
CREATE TABLE IF NOT EXISTS a2a_tasks (
    id TEXT PRIMARY KEY,
    context_id TEXT,
    state TEXT NOT NULL DEFAULT 'submitted',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    metadata TEXT  -- JSON
);

CREATE TABLE IF NOT EXISTS a2a_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES a2a_tasks(id),
    message_id TEXT NOT NULL,
    role TEXT NOT NULL,  -- 'user' or 'agent'
    parts TEXT NOT NULL, -- JSON array of Parts
    metadata TEXT,       -- JSON
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS a2a_artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES a2a_tasks(id),
    artifact_id TEXT NOT NULL,
    name TEXT,
    description TEXT,
    parts TEXT NOT NULL, -- JSON array of Parts
    metadata TEXT,       -- JSON
    created_at TEXT NOT NULL,
    UNIQUE(task_id, artifact_id)
);

CREATE TABLE IF NOT EXISTS a2a_push_notification_configs (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES a2a_tasks(id),
    url TEXT NOT NULL,
    token TEXT,
    auth_scheme TEXT,
    auth_credentials TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_a2a_messages_task_id ON a2a_messages(task_id);
CREATE INDEX idx_a2a_artifacts_task_id ON a2a_artifacts(task_id);
CREATE INDEX idx_a2a_push_configs_task_id ON a2a_push_notification_configs(task_id);
```

### 2.2 A2A database methods (`src/a2a_db.rs`)

Add to Database impl or create dedicated module:
- `a2a_create_task(id, context_id) -> Result<()>`
- `a2a_get_task(id) -> Result<Option<A2aTaskRow>>`
- `a2a_update_task_state(id, state) -> Result<()>`
- `a2a_insert_message(task_id, message) -> Result<()>`
- `a2a_get_messages(task_id, limit) -> Result<Vec<Message>>`
- `a2a_insert_artifact(task_id, artifact) -> Result<()>`
- `a2a_get_artifacts(task_id) -> Result<Vec<Artifact>>`
- `a2a_set_push_config(config) -> Result<()>`
- `a2a_get_push_config(id) -> Result<Option<TaskPushNotificationConfig>>`
- `a2a_list_push_configs(task_id) -> Result<Vec<TaskPushNotificationConfig>>`
- `a2a_delete_push_config(id) -> Result<bool>`
- `a2a_build_task(id, history_length) -> Result<Option<Task>>` — assembles full Task from tables

### 2.3 Agent Card builder (`src/a2a_card.rs`)

- `build_agent_card(agent_name, skills, base_url, config) -> AgentCard`
- Maps Mika skills to A2A `AgentSkill` entries
- Respects agent config allow-list for skill visibility
- Sets capabilities: `streaming: true`, `push_notifications: true`
- Default input/output modes: `["text/plain"]`
- Security schemes: `apiKey` in header

### 2.4 JSON-RPC dispatch handler (`src/server/a2a.rs`)

New Axum handler module:

- **`handle_a2a_jsonrpc(State, Json<JsonRpcRequest>) -> Response`** — main POST handler
  - Parse method → `A2aMethod` enum
  - Dispatch to method-specific handlers
  - Wrap results in `JsonRpcResponse`
  - Error handling: catch all, return proper JSON-RPC errors

- **Method handlers:**
  - `handle_message_send(state, params) -> Result<Task>`
    - Deserialize `MessageSendParams`
    - Create or reuse task (by `context_id`)
    - Store inbound message
    - Trigger agent processing (spawn async, similar to `/message` handler)
    - If `return_immediately`: return task in `submitted` state
    - Otherwise: wait for completion, return final task
  - `handle_message_stream(state, params) -> Sse<impl Stream>`
    - Same as send but returns SSE stream
    - Use `tokio::sync::broadcast` for status/artifact updates
    - Send `StatusUpdate` events as agent progresses
    - Send `ArtifactUpdate` for each artifact chunk
    - Final `Task` event on completion
  - `handle_tasks_get(state, params) -> Result<Task>`
  - `handle_tasks_cancel(state, params) -> Result<Task>`
  - `handle_push_config_set/get/list/delete` — CRUD on push notification configs

- **`handle_agent_card(State, Path<agent_name>) -> Json<AgentCard>`** — GET handler

### 2.5 SSE streaming infrastructure

- `A2aTaskBroadcaster` — wrapper around `tokio::sync::broadcast::Sender<StreamEvent>`
- Store in `AgentState` as `a2a_broadcasters: Arc<DashMap<String, A2aTaskBroadcaster>>`
- When agent processes A2A task, send events to broadcaster
- SSE handler subscribes to broadcaster, converts to `axum::response::sse::Event`
- Cleanup broadcaster when task reaches terminal state

### 2.6 A2A ↔ Agent integration

Bridge A2A messages to Mika's existing agent loop:
- `A2aMessage` → create a Mika session, inject the A2A message as user input
- Agent processes, produces response messages
- Intercept agent output → create A2A `Artifact` entries, update task state
- Map agent completion → `TaskState::Completed`
- Map agent error → `TaskState::Failed`
- Map agent asking for input → `TaskState::InputRequired`

### 2.7 Route registration

Add to `build_router()` in `server/mod.rs`:
```rust
// A2A routes
.route("/a2a/{agent_name}", post(handle_a2a_jsonrpc))
.route("/a2a/{agent_name}/agent.json", get(handle_agent_card))
```

Auth: Use internal token (gateway handles external API key auth).

## Phase 3: A2A Gateway Proxy in `mika-gateway`

### 3.1 Postgres schema (sqlx migration)

```sql
CREATE TABLE a2a_api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id TEXT NOT NULL,
    key_hash TEXT NOT NULL,       -- SHA-256 hash of the API key
    name TEXT NOT NULL,           -- human-readable label
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    UNIQUE(key_hash)
);

CREATE INDEX idx_a2a_api_keys_customer ON a2a_api_keys(customer_id);
CREATE INDEX idx_a2a_api_keys_hash ON a2a_api_keys(key_hash);
```

### 3.2 API key auth middleware (`src/a2a_auth.rs`)

- `validate_a2a_api_key(pool, api_key) -> Result<CustomerId>`
  - Hash the key with SHA-256
  - Look up in `a2a_api_keys` table
  - Check not revoked, not expired
  - Return `customer_id`
- Axum middleware: extract `x-api-key` header or `Authorization: Bearer` header

### 3.3 Gateway A2A routes (`src/a2a_routes.rs`)

- **`POST /a2a/{customer_id}/{agent_name}`** — JSON-RPC proxy
  - Validate API key → ensure customer_id matches
  - Forward request body to `container_url(customer_id)/a2a/{agent_name}`
  - Pass through response (JSON or SSE based on method)

- **`GET /a2a/{customer_id}/{agent_name}/agent.json`** — Agent Card proxy
  - Validate API key
  - Fetch from agent container
  - Rewrite URL in card to point to gateway's external URL

- **SSE passthrough:**
  - For `message/stream` and `tasks/resubscribe` methods
  - Detect by checking JSON-RPC method in request body
  - Stream response back using `axum::response::sse`
  - Set appropriate headers: `Content-Type: text/event-stream`, `Cache-Control: no-cache`

### 3.4 Route registration

Add to `build_router()` in gateway `routes.rs`:
```rust
.route("/a2a/{customer_id}/{agent_name}", post(handle_a2a_proxy))
.route("/a2a/{customer_id}/{agent_name}/agent.json", get(handle_a2a_agent_card))
```

## Phase 4: `a2a_call` Built-in Tool

### 4.1 Tool implementation (`src/tools/a2a_call.rs`)

- Tool name: `a2a_call`
- Parameters:
  - `url` (required): A2A agent URL
  - `message` (required): text message to send
  - `api_key` (optional): auth token for the remote agent
  - `stream` (optional, default false): whether to use streaming
- Execution:
  - Discover agent card at `{url}/.well-known/agent.json`
  - Create `A2aClient` with auth
  - Send message via `message/send` or `message/stream`
  - Return task result as tool output (text content from artifacts/messages)
- Register in `default_tools()` in `tools/mod.rs`

### 4.2 Tool definition

```json
{
  "name": "a2a_call",
  "description": "Call an external AI agent using the A2A protocol. Use this to delegate tasks to specialized agents.",
  "input_schema": {
    "type": "object",
    "properties": {
      "url": { "type": "string", "description": "Base URL of the A2A agent" },
      "message": { "type": "string", "description": "Message to send to the agent" },
      "api_key": { "type": "string", "description": "API key for authentication (optional)" },
      "stream": { "type": "boolean", "description": "Use streaming mode (default: false)" }
    },
    "required": ["url", "message"]
  }
}
```

## Phase 5: Tests

### 5.1 `mika-a2a` unit tests

- `types.rs`: serde round-trip for all types, kebab-case serialization
- `jsonrpc.rs`: request/response parsing, error code defaults
- `state_machine.rs`: all valid transitions, all invalid transitions, terminal state checks
- `client.rs`: mock server tests for send/get/cancel/stream (use `wiremock` or inline mock)
- `streaming.rs`: SSE parsing, event deserialization

### 5.2 `mika-agent` A2A tests

- Schema migration: verify tables created at v13
- DB methods: CRUD for tasks, messages, artifacts, push configs
- Agent Card: verify card generation from skills
- JSON-RPC dispatch: valid/invalid methods, error responses
- Integration: message send → agent processes → task completed

### 5.3 `mika-gateway` A2A tests

- API key validation: valid, expired, revoked, wrong customer
- Proxy routing: correct container URL resolution
- Agent Card URL rewriting

## Implementation Order

1. **Phase 1** (mika-a2a): Types → JSON-RPC → Params → Streaming → State machine → Error → Client
2. **Phase 2** (mika-agent): Schema v13 → DB methods → Agent Card → JSON-RPC handlers → SSE → Route registration
3. **Phase 3** (mika-gateway): Postgres migration → API key auth → Proxy routes
4. **Phase 4** (a2a_call tool): Tool implementation → Registration
5. **Phase 5** (tests): Unit tests per crate → Integration tests

## Out of Scope (Deferred)

- OAuth2 authentication (planned, not in this PR)
- gRPC / HTTP+JSON bindings (JSON-RPC only for v1)
- Push notification sender (tables ready, sender deferred)
- Agent Card signatures (JWS)
- Multi-tenant gateway paths with `/{tenant}/` prefix
- File/image part handling in a2a_call tool (text-only first pass)
