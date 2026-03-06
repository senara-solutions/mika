# Mika Architecture

## 1. Overview

Mika is a conversation-first AI executive assistant built in Rust. It operates in two
modes:

- **CLI mode (embedded):** The `mika` binary runs locally. User input flows through a
  ratatui TUI, the agent loop runs in-process, and SQLite stores all data on the local
  filesystem. No network services are required beyond the Claude API.

- **Hosted mode (per-customer containers):** Each customer gets their own
  agent container running `mika-server` (Axum HTTP), with an isolated SQLite database
  on a persistent volume. A shared gateway (`crates/mika-gateway/` in this repo)
  routes messages from Telegram to the correct container.

Both modes use the same agent loop, tools, memory model, and prompt assembly code
from the `mika-agent` crate.


## 2. Architecture Diagram

### CLI Mode

```
+-----------+     +----------------+     +-------------+
|  Terminal  |---->|  mika binary   |---->|  Claude API  |
|  (ratatui) |<----|  (agent loop)  |<----|  (Messages)  |
+-----------+     +-------+--------+     +-------------+
                          |
                          v
                    +----------+
                    |  SQLite   |
                    |  (local)  |
                    +----------+
```

### Hosted Mode

```
+----------+     +--------------------+     +----------------------------+
| Telegram  |---->|  Gateway           |     |  Per-customer container    |
| Bot API   |<----|  (mika-gateway)    |     |  mika-server (Axum)        |
+----------+     +--------+-----------+     |                            |
                           |                 |  SQLite (persistent vol)   |
                           |   POST /message |  Agent loop + tools        |
                           +---------------->|                            |
                           |                 +-------------+--------------+
                           |   POST /send                  |
                           |<------------------------------+
                           |                        +------+------+
                           |                        |  Claude API  |
                           |                        +-------------+
```


## 3. Crate Structure

| Crate | Path | Responsibility |
|-------|------|---------------|
| `mika-common` | `crates/mika-common/` | Shared library: config (config-rs with `MIKA_` prefix), Claude API client (`ClaudeClient` with typed `ClaudeApiError`), logging (tracing), telemetry (feature-gated OTel/OTLP export), home directory resolution |
| `mika-agent` | `crates/mika-agent/` | Agent container: SQLite database (`Database`, `AsyncDatabase`), agent loop (`run_agent`, `run_silent_agent`), 8 builtin tools, prompt assembly, conversation compaction, reminder scheduler, HTTP server binary (`mika-server`) |
| `mika-cli` | `crates/mika-cli/` | TUI CLI binary (`mika`): ratatui chat interface, clap subcommands (`status`, `memory`, `reminders`, `config`, `setup`) |
| `mika-gateway` | `crates/mika-gateway/` | Telegram webhook router: Postgres customer registry, message routing to per-customer containers, pairing flow, outbound relay to Telegram. Stateless, env-var-only config. |


## 4. Agent Loop

The agent loop is an explicit Rust async function -- no framework. It executes the
following steps for each inbound user message:

Source: `crates/mika-agent/src/agent.rs` -- `run_agent()` / `run_agent_inner()`

1. **Save user message** to the `conversations` table via `AsyncDatabase::save_message()`.

2. **Load context** for system prompt assembly:
   - `soul.md` (agent personality, read from `home_dir`)
   - Identity configuration (`identity.toml` or inline defaults)
   - All core memory blocks (`get_all_core_memory()`)
   - Customer timezone (`get_customer_config("timezone")`)
   - Existing conversation summary, if any (`load_conversation_summary()`)

3. **Match skills** against the user message via `SkillRegistry::match_message()`.
   For matched skills, lazy-load prompt snippets and inject them into the system
   prompt. Resolve the final set of tool definitions (builtin + skill-provided).
   If no skills directory exists, fall back to all builtin tools.

4. **Load recent messages** -- the last 20 conversation messages
   (`load_recent_messages(20, None)`).

5. **Send request to Claude API** with system prompt, message history, and tool
   definitions.

6. **Match `stop_reason`** from the Claude response:
   - `EndTurn` or `MaxTokens` -- save assistant text to DB, return response.
   - `StopSequence` -- save assistant text to DB, return response.
   - `ToolUse` -- execute each tool call with per-tool timeout, push assistant
     message and tool results onto the request, strip images from prior turns
     to prevent memory accumulation, loop back to step 5.

   **Step-awareness nudge:** At step 8 of 10 (conversation mode only), a nudge
   is appended to the system prompt telling the model to prioritize completing
   or summarizing its work.

   **Max-steps exceeded:** If the loop exhausts all 10 steps without producing
   text, a continuation turn is attempted: tools are disabled, thinking is
   disabled, and one final API call (60s timeout) forces the model to produce
   a text summary of what it accomplished. If the continuation fails (API error,
   timeout, empty response), a structured fallback shows the last 5 tool names
   with status and invites the user to ask for continuation.

   **Multi-modal tool results:** Tools can return images alongside text via
   `ToolOutput::success_with_images()`. When images are present, the tool result
   is sent as a multi-block content array (`[{type: "text"}, {type: "image"}]`)
   matching the Claude API spec. Prior-turn images are replaced with
   `[image(s) from previous turn omitted]` text before each API call.

7. **Post-turn compaction** -- after the agent returns, check if conversation
   compaction is needed (`compaction::maybe_compact()`). In CLI mode this runs
   inline. In server mode (`skip_compaction: true`), compaction is spawned
   outside the agent lock.

### Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_TOOL_STEPS` | 10 | Maximum tool-use iterations per agent turn |
| `TOOL_TIMEOUT_SECS` | 30 | Per-tool execution timeout (seconds) |
| `AGENT_TOTAL_TIMEOUT_SECS` | 300 | Total agent loop timeout (5 minutes) |
| `CONTINUATION_TIMEOUT_SECS` | 60 | Continuation turn timeout after max steps |


## 5. Memory Model

Mika uses a three-layer memory hierarchy. Each customer has their own isolated SQLite
database.

### Layer 1: Core Memory

Always present in the system prompt. The agent can edit these blocks via the
`update_core_memory` tool.

| Block | Default Value |
|-------|--------------|
| `user_summary` | "No information about the user yet." |
| `self_model` | "I am {agent_id}. No interaction history yet." |
| `current_priorities` | "No priorities set yet." |
| `key_people` | "No people tracked yet." |

**Constraints:**
- Per-block limit: `MAX_TOKENS_PER_BLOCK = 500` (~2000 characters at 4 chars/token)
- Per-session edit limit: `MAX_CORE_MEMORY_EDITS_PER_SESSION = 3` (onboarding sessions exempt)
- Actions: `replace`, `append`, `remove_line`, `reset`
- All mutations are recorded in the `memory_events` audit table

### Layer 2: Structured Facts

Stored in dedicated SQLite tables. Managed by the agent via `store_fact`,
`update_fact`, and `search_memory` tools.

| Category | Table | Key Columns |
|----------|-------|-------------|
| People | `people` | `canonical_name` (UNIQUE COLLATE NOCASE), `relationship`, `notes` |
| Commitments | `commitments` | `description` (UNIQUE COLLATE NOCASE), `status` (pending/completed/cancelled), `due_date`, `person_id` FK |
| Preferences | `preferences` | `category` (UNIQUE COLLATE NOCASE), `value` |
| Events | `events` | `description`, `event_date`, `context` |

### Layer 3: Hybrid Search

FTS5 full-text + sqlite-vec cosine similarity via Reciprocal Rank Fusion.
Optional OpenAI embeddings (`text-embedding-3-small`, 512 dims). Graceful
degradation: hybrid -> FTS5-only -> LIKE fallback. Indexed on `store_fact`/
`update_fact`, backfilled on startup.

See [ADR-003](adr/003-layer3-hybrid-vector-search.md) for implementation details.


## 6. Tools

All 8 builtin tools, registered in `crates/mika-agent/src/tools/mod.rs` via
`default_tools()`:

| Tool | Description | Category |
|------|-------------|----------|
| `update_core_memory` | Update persistent core memory blocks (Layer 1). Actions: replace, append, remove_line, reset. Rate limited to 3 edits/session. | Memory |
| `store_fact` | Store a new structured fact (person, commitment, preference, or event) into Layer 2 tables. | Memory |
| `search_memory` | Search across all Layer 2 categories (people, commitments, preferences, events). | Memory |
| `update_fact` | Update an existing Layer 2 fact (e.g., change commitment status, update person notes). | Memory |
| `create_reminder` | Schedule a future reminder with ISO 8601 `fire_at` timestamp and message text. | Reminders |
| `list_reminders` | List pending and future reminders. | Reminders |
| `cancel_reminder` | Cancel a pending reminder by ID. | Reminders |
| `send_message` | Send a message to the user out-of-band. In CLI mode, prints to stdout. In server mode, POSTs to the routing URL. Required for silent mode (heartbeat/reminders). | Messaging |

**Tool trait:** `#[async_trait]` with `Send + Sync` bounds (required for `tokio::spawn`
in server handlers). Each tool validates inputs: empty string check + 10,000 character
maximum (`MAX_INPUT_LEN`).


## 7. Conversation Compaction

When conversation history grows beyond a threshold, older messages are summarized via
a Claude API call and replaced with a summary row. The summary is injected into the
system prompt (not into message history).

### Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `COMPACTION_THRESHOLD` | 50 | Minimum message count before compaction triggers |
| `CONTEXT_WINDOW` | 20 | Number of recent messages to keep (not compacted) |
| `MAX_COMPACTION_BATCH` | 100 | Maximum messages per summarization call |
| `MAX_SUMMARY_CHARS` | 4000 | Truncation limit for generated summaries |
| `MAX_COMPACTION_INPUT_CHARS` | 50,000 | Character budget for messages sent to summarizer |

### Flow

1. After each agent turn, `maybe_compact()` checks `count_messages()`.
2. If total messages <= 50, skip.
3. Load all messages outside the context window.
4. Cap the batch at 100 messages.
5. Call Claude API with a summarization system prompt and the message batch.
6. Truncate the generated summary to 4000 characters if needed.
7. Delete old messages up to `compacted_through_id`, insert or update the summary row.
8. Recent 20 messages remain untouched.

Compaction is incremental -- subsequent rounds merge the existing summary with
newly compacted messages.


## 8. AsyncDatabase

Source: `crates/mika-agent/src/async_db.rs`

`AsyncDatabase` wraps the synchronous `Database` (rusqlite) with a dedicated OS
thread and an `mpsc` channel, making it Send+Sync and compatible with `tokio::spawn`.

```
Caller (any tokio task)                  Dedicated OS thread ("mika-db")
        |                                        |
        |-- mpsc::send(closure) ----------------->|
        |                                        |-- closure(&Database)
        |<-- oneshot::send(Result<T>) ------------|
```

Properties:
- **Clone-able:** Wraps `Arc<AsyncDatabaseInner>` — clones share the same connection.
- **Panic-resilient:** Each closure wrapped in `catch_unwind()`.
- **Graceful shutdown:** `shutdown()` drops the sender, joins the background thread.


## 9. Heartbeat System

The heartbeat system enables proactive check-ins without user initiation. An
external scheduler periodically POSTs to the container's `/heartbeat` endpoint.

### Pre-Filter Checks (before acquiring Mutex)

1. Active hours: 08:00-21:00 in customer's local timezone
2. Max 1 heartbeat per hour
3. Max 3 heartbeats per day
4. Skip if user messaged within 2 hours

If any check fails, the handler returns `204 No Content` immediately.


## 10. Silent Mode

Silent mode is used for background tasks (heartbeat check-ins and reminders) where
the agent's text output is NOT automatically delivered to the user.

| Aspect | Normal Mode | Silent Mode |
|--------|-------------|-------------|
| User message | Actual user input | Synthetic trigger |
| Text output | Returned to caller | NOT delivered (saved to DB for audit) |
| How to reach user | Automatic | Must use `send_message` tool |
| Message history | Last 20 messages | Single trigger message only |
| Compaction | Runs post-turn | Does not run |

Heartbeat mode uses `safe_always_on_skills()` which filters out exec/http-handler
skills for security — only builtin-handler skills are available in autonomous
background runs.


## 11. HTTP Server (mika-server)

The per-customer agent container runs an Axum HTTP server:

| Endpoint | Method | Auth | Purpose |
|----------|--------|------|---------|
| `/health` | GET | None | Liveness/readiness probe |
| `/message` | POST | Bearer | Receives messages (202 async processing, 10MB body limit) |
| `/heartbeat` | POST | Bearer | Scheduled job trigger for proactive check-ins |

`AppState` is Clone via Arc-wrapped dependencies. Agent lock
(`tokio::sync::Mutex<()>`) serializes agent loops with non-blocking `try_lock`
(429 if busy).

See [ADR-001](adr/001-axum-http-server-architecture.md) for design decisions.


## 12. Failed Sends (Durable Outbox Pattern)

When the outbound routing endpoint is unreachable, messages are not lost.

### Write Path

`GatewayMessageSender::send()` implements retry-then-persist:
1. First attempt: POST to routing URL with 10s timeout.
2. On failure: Wait 2 seconds, retry once.
3. On second failure: Save to `failed_sends` SQLite table. Return `Ok(())` so
   the agent loop does not see a tool error.

### Read Path (Flush)

At the start of each `/message` handler, the server flushes up to 5 pending failed
sends in a background task (does not block message processing).


## 13. Multi-Agent Support

- Global home directory: `~/.mika/`
- Agent homes: `~/.mika/agents/{name}/` (each with data/, skills/, logs/)
- Active agent tracked in `~/.mika/active_agent`
- CLI `--agent` flag overrides active agent
- Server discovers all agents on startup

See [ADR-004](adr/004-multi-agent-teams-orchestration.md) for team orchestration.


## 14. Observability & Telemetry

Mika follows an "always instrument, optionally export" pattern. Tracing spans are
compiled unconditionally into the binary — no feature flags needed. Spans cover the
agent loop (`agent_turn`), Claude API calls, per-tool execution, team engine
(`team_run`, `team_agent_task`), and server HTTP handlers (`tower_http::TraceLayer`).

### Optional OTLP Export

Export is feature-gated behind `--features telemetry`. When enabled,
`mika_common::telemetry::build_otel_layer()` builds an OpenTelemetry tracing layer
that exports spans via OTLP/HTTP. The layer composes into the tracing subscriber
alongside the normal log layer.

Three environment variables control export:

| Variable | Purpose |
|----------|---------|
| `MIKA_TELEMETRY_ENABLED` | Enable trace export (`true`/`false`, default: false) |
| `MIKA_OTLP_ENDPOINT` | OTLP HTTP endpoint URL (must include `/v1/traces` path) |
| `MIKA_OTLP_AUTH_HEADER` | Authorization header value (e.g., Base64 credentials) |

`build_otel_layer()` returns a `TelemetryGuard` that flushes pending spans on drop,
ensuring no traces are lost at shutdown. Both `mika-server` and `mika` CLI hold
the guard alive until process exit.

### Langfuse Compatibility

The OTLP export is compatible with Langfuse's OpenTelemetry ingestion endpoint.
Set `MIKA_OTLP_ENDPOINT` to `https://cloud.langfuse.com/api/public/otel/v1/traces`
and `MIKA_OTLP_AUTH_HEADER` to `publicKey:secretKey` (auto-encoded to Base64) for
authentication. For Jaeger, use `http://localhost:4318/v1/traces` (no auth needed).

### Graceful Degradation

When the `telemetry` feature is not compiled in, `build_otel_layer()` is a no-op
that returns `None`. When compiled but `MIKA_TELEMETRY_ENABLED` is false or unset,
no exporter is created. Spans still flow to the normal log subscriber either way.


## Appendix: Database Schema

**Schema version:** 11

### Tables

| Table | Purpose | Schema Version |
|-------|---------|----------------|
| `schema_version` | Migration tracking | v4 |
| `conversations` | Message history (user, assistant, summary rows) | v4 (+`compacted_through_id` in v5) |
| `core_memory` | Layer 1 persistent memory blocks | v4 |
| `people` | Layer 2 people/contacts | v4 |
| `commitments` | Layer 2 tasks/promises with status tracking | v4 |
| `preferences` | Layer 2 user preferences | v4 |
| `events` | Layer 2 notable events | v4 |
| `memory_events` | Audit log for all memory mutations | v4 (+`created_at` index in v7) |
| `reminders` | Scheduled future reminders | v5 |
| `heartbeat_sends` | Rate limiting for heartbeat sends | v5 |
| `customer_config` | Key-value store (timezone, chat_id) | v5 |
| `failed_sends` | Durable outbox for failed outbound messages | v5 |
| `memory_event_summaries` | Tiered retention summaries (monthly) | v6 |
| `skills` | Skill metadata (name, description, builtin flag, enabled) | v7 |
| `skill_tools` | Tool definitions per skill | v7 |
| `search_content` | Unified search content for Layer 3 hybrid search | v8 |
| `fts_search` | FTS5 virtual table for full-text search | v8 |
| `vec_search` | sqlite-vec virtual table (vec0) for vector similarity | v8 |
| `reflection_runs` | Periodic memory reflection tracking | v10 |
| `team_runs` | Team execution run metadata | v11 |
| `team_messages` | Graph-structured team messages with parent_id links | v11 |

### SQLite Pragmas

The database is initialized with WAL journal mode, NORMAL synchronous level, foreign
keys enabled, a 5-second busy timeout, and incremental auto-vacuum.
