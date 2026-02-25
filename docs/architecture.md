# Mika Architecture

## 1. Overview

Mika is a conversation-first AI executive assistant built in Rust. It operates in two
distinct modes:

- **CLI mode (embedded):** The `mika` binary runs locally. User input flows through a
  ratatui TUI, the agent loop runs in-process, and SQLite stores all data on the local
  filesystem. No network services are required beyond the Claude API.

- **Hosted mode (per-customer containers on Kubernetes):** A shared gateway receives
  Telegram webhooks and routes messages to per-customer agent containers. Each container
  runs `mika-server` (Axum HTTP), owns its own SQLite database on a K8s encrypted
  volume, and communicates with the Claude API independently. Outbound messages flow
  back through the gateway to the Telegram Bot API.

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

### Hosted Mode (Kubernetes)

```
+----------+     +--------------------+     +----------------------------+
| Telegram  |---->|  mika-gateway      |     |  Per-customer container    |
| Bot API   |<----|  (shared, Axum)    |     |  mika-server (Axum)        |
+----------+     |                    |     |                            |
                  |  Postgres          |     |  SQLite (encrypted vol)    |
                  |  (customer         |     |  Agent loop + tools        |
                  |   registry)        |     |                            |
                  +--------+-----------+     +-------------+--------------+
                           |                               |
                           |   POST /message               |
                           +------------------------------>|
                           |                               |
                           |   POST /send (outbound)       |
                           |<------------------------------+
                           |                               |
                           |                        +------+------+
                           |                        |  Claude API  |
                           |                        +-------------+
                           |
                           |  (one container per customer)
                           |
                  +--------+-----------+
                  |  mika-{customer-id} |  <-- deterministic DNS:
                  |  .mika-agents.svc   |      http://mika-{uuid}.mika-agents
                  |  .cluster.local:8080|      .svc.cluster.local:8080
                  +--------------------+
```


## 3. Crate Structure

| Crate | Path | Responsibility |
|-------|------|---------------|
| `mika-common` | `crates/mika-common/` | Shared library: config (config-rs with `MIKA_` prefix), Claude API client (`ClaudeClient` with typed `ClaudeApiError`), logging (tracing), home directory resolution |
| `mika-agent` | `crates/mika-agent/` | Agent container: SQLite database (`Database`, `AsyncDatabase`), agent loop (`run_agent`, `run_silent_agent`), 8 builtin tools, prompt assembly, conversation compaction, reminder scheduler, HTTP server binary (`mika-server`) |
| `mika-cli` | `crates/mika-cli/` | TUI CLI binary (`mika`): ratatui chat interface, clap subcommands (`status`, `memory`, `reminders`, `config`, `setup`) |
| `mika-gateway` | `crates/mika-gateway/` | Telegram webhook router: Postgres-backed customer registry, inbound webhook handling, outbound relay (`/send`), customer pairing via deep links, K8s health probes |


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
     message and tool results onto the request, loop back to step 5.

7. **Post-turn compaction** -- after the agent returns, check if conversation
   compaction is needed (`compaction::maybe_compact()`). In CLI mode this runs
   inline. In server mode (`skip_compaction: true`), compaction is spawned
   outside the agent lock.

### Constants

Defined in `crates/mika-agent/src/agent.rs`:

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_TOOL_STEPS` | 10 | Maximum tool-use iterations per agent turn |
| `TOOL_TIMEOUT_SECS` | 30 | Per-tool execution timeout (seconds) |
| `AGENT_TOTAL_TIMEOUT_SECS` | 300 | Total agent loop timeout (5 minutes) |

If the agent exceeds `MAX_TOOL_STEPS`, it returns a fallback message. If the total
timeout fires, the outer `tokio::time::timeout` wrapper catches it and saves a
fallback response to the database.


## 5. Memory Model

Mika uses a three-layer memory hierarchy. Each customer has their own isolated SQLite
database.

### Layer 1: Core Memory

Always present in the system prompt. The agent can edit these blocks via the
`update_core_memory` tool.

| Block | Default Value |
|-------|--------------|
| `user_summary` | "New user. No information yet." |
| `persona` | "Mika -- personal AI executive assistant." |
| `current_priorities` | "Get to know the user and understand their needs." |
| `key_people` | "No one tracked yet." |

Source: `crates/mika-agent/src/db.rs` -- `CORE_MEMORY_SECTIONS`

**Constraints:**
- Per-block limit: `MAX_TOKENS_PER_BLOCK = 500` (~2000 characters at 4 chars/token)
- Per-session edit limit: `MAX_CORE_MEMORY_EDITS_PER_SESSION = 3` (onboarding sessions exempt)
- Actions: `replace`, `append`, `remove_line`, `reset`
- All mutations are recorded in the `memory_events` audit table

Source: `crates/mika-agent/src/tools/update_core_memory.rs`

### Layer 2: Structured Facts

Stored in dedicated SQLite tables. Managed by the agent via `store_fact`,
`update_fact`, and `search_memory` tools.

| Category | Table | Key Columns |
|----------|-------|-------------|
| People | `people` | `canonical_name` (UNIQUE COLLATE NOCASE), `relationship`, `notes` |
| Commitments | `commitments` | `description` (UNIQUE COLLATE NOCASE), `status` (pending/completed/cancelled), `due_date`, `person_id` FK |
| Preferences | `preferences` | `category` (UNIQUE COLLATE NOCASE), `value` |
| Events | `events` | `description`, `event_date`, `context` |

### Layer 3: Vector Search (Future)

Not yet implemented. Planned: `sqlite-vec` + FTS5 hybrid search for long-term
archival memory retrieval.


## 6. Tools

All 8 builtin tools, registered in `crates/mika-agent/src/tools/mod.rs` via
`default_tools()`:

| Tool | Description | Category |
|------|-------------|----------|
| `update_core_memory` | Update persistent core memory blocks (Layer 1). Actions: replace, append, remove_line, reset. Rate limited to 3 edits/session. | Memory |
| `store_fact` | Store a new structured fact (person, commitment, preference, or event) into Layer 2 tables. | Memory |
| `search_memory` | Search across all Layer 2 categories (people, commitments, preferences, events) using LIKE queries. | Memory |
| `update_fact` | Update an existing Layer 2 fact (e.g., change commitment status, update person notes). | Memory |
| `create_reminder` | Schedule a future reminder with ISO 8601 `fire_at` timestamp and message text. | Reminders |
| `list_reminders` | List pending and future reminders. | Reminders |
| `cancel_reminder` | Cancel a pending reminder by ID. | Reminders |
| `send_message` | Send a message to the user out-of-band. In CLI mode, prints to stdout. In server mode, POSTs to the gateway `/send` endpoint. Required for silent mode (heartbeat/reminders). | Messaging |

**Tool trait:** `#[async_trait]` with `Send + Sync` bounds (required for `tokio::spawn`
in server handlers). Each tool validates inputs: empty string check + 10,000 character
maximum (`MAX_INPUT_LEN`).

**ToolContext** provided to every tool execution:

```rust
pub struct ToolContext<'a> {
    pub db: &'a AsyncDatabase,
    pub session_id: &'a str,
    pub home_dir: &'a Path,
    pub core_memory_edit_count: &'a AtomicU32,
    pub is_onboarding: bool,
    pub message_sender: Option<Arc<dyn MessageSender>>,
}
```


## 7. Conversation Compaction

When conversation history grows beyond a threshold, older messages are summarized via
a Claude API call and replaced with a summary row. The summary is injected into the
system prompt (not into message history).

Source: `crates/mika-agent/src/compaction.rs`

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
2. If total messages <= `COMPACTION_THRESHOLD` (50), skip.
3. Load all messages outside the context window (`load_messages_before_window(20)`).
4. Cap the batch at `MAX_COMPACTION_BATCH` (100) messages.
5. Call Claude API with a summarization system prompt and the message batch
   (optionally merged with any existing summary).
6. Truncate the generated summary to `MAX_SUMMARY_CHARS` (4000) if needed.
7. Call `replace_with_summary()` -- deletes old messages up to `compacted_through_id`,
   inserts or updates the summary row (role = 'summary').
8. Recent `CONTEXT_WINDOW` (20) messages remain untouched.

Compaction is **incremental** -- subsequent compaction rounds merge the existing summary
with newly compacted messages.


## 8. AsyncDatabase

Source: `crates/mika-agent/src/async_db.rs`

### Problem

`rusqlite::Connection` is `!Send` -- it cannot be moved across threads, which is
required by `tokio::spawn` in the HTTP server handlers.

### Solution: Closure-Based Dispatch

`AsyncDatabase` wraps the synchronous `Database` with a dedicated OS thread and an
`mpsc` channel:

```
Caller (any tokio task)                  Dedicated OS thread ("mika-db")
        |                                        |
        |-- mpsc::send(closure) ----------------->|
        |                                        |-- closure(&Database)
        |<-- oneshot::send(Result<T>) ------------|
        |                                        |
```

1. `AsyncDatabase::new(db)` spawns a named OS thread (`"mika-db"`) that owns the
   `Database` instance and loops on `mpsc::Receiver::recv()`.
2. Each async method (e.g., `save_message()`) clones owned values, creates a
   `oneshot::channel()`, and sends a `Box<dyn FnOnce(&Database) + Send>` closure over
   the `mpsc` channel.
3. The background thread executes the closure and sends the result back via the
   `oneshot` sender.
4. The caller `.await`s the `oneshot` receiver.

### Properties

- **Clone-able:** `AsyncDatabase` wraps `Arc<AsyncDatabaseInner>`, so clones share
  the same background thread and connection.
- **Send + Sync:** Safe to hold in `AppState` and pass to `tokio::spawn` tasks.
- **Panic-resilient:** The background thread wraps each closure in
  `std::panic::catch_unwind()` -- a panicking closure does not kill the thread.
- **Graceful shutdown:** `shutdown()` drops the `mpsc::Sender` (causing `recv()` to
  return `Err`), then joins the background thread. Subsequent operations return
  `"database has been shut down"`.


## 9. Heartbeat System

The heartbeat system enables proactive check-ins without user initiation. A K8s CronJob
periodically POSTs to the container's `/heartbeat` endpoint.

Source: `crates/mika-agent/src/server/handlers.rs` -- `handle_heartbeat()`,
`heartbeat_should_run()`

### Pre-Filter Checks

All checks run **before** acquiring the agent Mutex (cheap, no Claude API call):

1. **Active hours:** Customer's local time must be between 08:00 and 21:00
   (via `chrono-tz` timezone conversion).
2. **Hourly rate limit:** Maximum 1 heartbeat send per hour
   (`count_heartbeat_sends_last_hour() >= 1`).
3. **Daily rate limit:** Maximum 3 heartbeat sends per day
   (`count_heartbeat_sends_today() >= 3`).
4. **Recent activity:** Skip if the user sent a message within the last 2 hours
   (`last_user_message_time()` + `TimeDelta::hours(2)`).

If any check fails, the handler returns `204 No Content` immediately.

### Execution Flow

1. Pre-filter passes.
2. `try_lock_owned()` on the agent Mutex -- if busy, return `204` (heartbeat is
   skippable).
3. Spawn a background task that:
   a. Creates a `SilentAgentParams` with `SilentTrigger::Heartbeat`.
   b. Runs `run_silent_agent()` (see Silent Mode below).
   c. Records the heartbeat send (`record_heartbeat_send()`) for rate limiting.
4. Return `200 OK` immediately.


## 10. Silent Mode

Silent mode is used for background tasks (heartbeat check-ins and reminders) where
the agent's text output is NOT automatically delivered to the user.

Source: `crates/mika-agent/src/agent.rs` -- `run_silent_agent()`, `run_silent_inner()`

### Key Differences from Normal Agent Loop

| Aspect | Normal Mode | Silent Mode |
|--------|-------------|-------------|
| User message | Actual user input | Synthetic trigger: `"[heartbeat trigger]"` or `"[reminder trigger: ...]"` |
| Text output delivery | Returned to caller / sent via gateway | NOT delivered -- saved to DB for audit only |
| How to reach the user | Automatic | Agent must explicitly use `send_message` tool |
| System prompt | `build_system_prompt()` | `build_silent_prompt()` with `SilentPromptContext` |
| Message history | Last 20 messages loaded | No history loaded -- single trigger message only |
| Compaction | Runs post-turn | Does not run |

### SilentPromptContext

The silent prompt includes:
- Soul content and identity
- Core memory blocks
- Pending commitments (so the agent can reason about timely follow-ups)
- Trigger context (heartbeat instructions or reminder data)
- Current UTC time and customer timezone

### Trigger Types

```rust
pub enum SilentTrigger {
    Heartbeat,
    Reminder { id: i64, message: String },
}
```

For reminders, the agent loop marks the reminder as `delivered` on success or `failed`
on error/timeout.


## 11. Gateway Architecture

The gateway (`mika-gateway`) is a stateless Axum HTTP service that routes messages
between Telegram and per-customer agent containers. It uses Postgres for the customer
registry.

Source: `crates/mika-gateway/src/routes.rs`, `crates/mika-gateway/src/telegram.rs`

### Endpoints

| Endpoint | Method | Auth | Purpose |
|----------|--------|------|---------|
| `/webhook/telegram` | POST | `X-Telegram-Bot-Api-Secret-Token` header (constant-time comparison) | Receive Telegram updates |
| `/send` | POST | `Authorization: Bearer <MIKA_INTERNAL_TOKEN>` (constant-time comparison) | Containers deliver outbound messages to Telegram |
| `/health` | GET | None | K8s readiness probe (checks `ready` flag + Postgres `SELECT 1`) |
| `/readyz` | GET | None | K8s readiness probe (alias for `/health`) |
| `/livez` | GET | None | K8s liveness probe (unconditional `200 OK`) |

### Inbound Flow (Telegram -> Container)

1. Telegram POSTs an update to `/webhook/telegram`.
2. Gateway validates the `X-Telegram-Bot-Api-Secret-Token` header (constant-time).
3. Concurrency check: acquire a semaphore permit (`try_acquire_owned`). If at
   capacity, return `503` (Telegram will retry).
4. Return `200 OK` to Telegram immediately.
5. Spawn async task to process the update:
   a. **Parse update** via `parse_update()` -- yields one of: `Start` (pairing),
      `Text` (message), `BareStart`, `Unsupported`, `NoMessage`.
   b. **For text messages:**
      - Look up customer by `telegram_chat_id` in Postgres.
      - Check customer status (drop silently if `suspended`).
      - **Atomic dedup:** `UPDATE customers SET last_update_id = $1 WHERE id = $2
        AND last_update_id < $1 RETURNING id` -- prevents duplicate processing.
      - Compute deterministic container URL: `http://mika-{customer_id}.mika-agents
        .svc.cluster.local:8080` (no user-controlled URLs, prevents SSRF).
      - Forward to container `POST /message` with Bearer auth, 2s timeout.
      - On forwarding failure: reset dedup counter (CAS-safe) so Telegram retry
        can succeed.
   c. **For pairing (`/start <token>`):**
      - Validate token format: 64-character hex string.
      - Atomic pair: `UPDATE customers SET telegram_chat_id = $1 ... WHERE
        pairing_token = $2 AND telegram_chat_id IS NULL AND status = 'provisioned'
        AND pairing_expires_at > now()`.
      - On success: forward synthetic `"Hello!"` to the container for onboarding.
      - On failure: return opaque "Invalid or expired invite link."

### Outbound Flow (Container -> Telegram)

1. Container's `GatewayMessageSender` POSTs to gateway `/send` with
   `{ chat_id, text, request_id }` and Bearer auth.
2. Gateway validates the Bearer token (constant-time).
3. Gateway calls `TelegramClient::send_message(chat_id, text)`.
4. Returns status based on Telegram response:
   - `200 OK` -- sent successfully
   - `410 Gone` -- bot blocked by user
   - `429 Too Many Requests` -- Telegram rate limited (includes `Retry-After` header)
   - `502 Bad Gateway` -- other Telegram API errors

### Telegram Client

Source: `crates/mika-gateway/src/telegram.rs`

`TelegramClient` wraps `reqwest::Client` with a `SecretString` bot token. Typed errors
via `TelegramApiError`:

| Variant | HTTP Status | Meaning |
|---------|-------------|---------|
| `Unauthorized` | 401 | Invalid bot token |
| `BotBlocked` | 403 | User blocked the bot |
| `RateLimited` | 429 | Telegram rate limit (with optional `retry_after`) |
| `BadRequest` | 400 | Invalid payload |
| `Other` | * | Unrecognized error |
| `Network` | -- | reqwest transport error |

### Customer Pairing (Deep Link)

1. Provisioning creates a customer row in Postgres with a 64-character hex
   `pairing_token` and `pairing_expires_at` timestamp.
2. User clicks a Telegram deep link: `https://t.me/<bot>?start=<token>`.
3. Telegram sends `/start <token>` to the webhook.
4. Gateway validates the token and atomically pairs the customer (`UPDATE` with
   conditions: token matches, not already paired, status is `provisioned`,
   token not expired).
5. On success, the customer status transitions from `provisioned` to `active`.


## 12. Failed Sends (Durable Outbox Pattern)

When the gateway's `/send` endpoint is unreachable, outbound messages must not be lost.

Source: `crates/mika-agent/src/messaging.rs` -- `GatewayMessageSender`
Source: `crates/mika-agent/src/server/handlers.rs` -- `flush_failed_sends()`

### Write Path

`GatewayMessageSender::send()` implements retry-then-persist:

1. **First attempt:** POST to `{gateway_url}/send` with 10s timeout.
2. **On failure:** Wait 2 seconds, retry once.
3. **On second failure:** Save the message text to the `failed_sends` SQLite table
   (`save_failed_send()`). Return `Ok(())` to the caller -- the agent loop does not
   see a tool error (the message is queued, not lost).

### Read Path (Flush)

At the start of each `/message` handler invocation, the server spawns a best-effort
flush task:

1. Load up to 5 pending failed sends (`get_pending_failed_sends(5)`).
2. For each, re-attempt sending via `GatewayMessageSender`.
3. On success: delete the row (`delete_failed_send()`).
4. On failure: increment the retry counter (`increment_failed_send_retry()`).

The flush runs in a separate `tokio::spawn` task -- it does not block the main
message processing path.

### Schema

```sql
CREATE TABLE failed_sends (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text TEXT NOT NULL,
    request_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    retry_count INTEGER NOT NULL DEFAULT 0
);
```


## Appendix: Database Schema

**Schema version:** 7 (defined as `CURRENT_SCHEMA_VERSION` in `crates/mika-agent/src/db.rs`)

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

### SQLite Pragmas

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA auto_vacuum = INCREMENTAL;
```
