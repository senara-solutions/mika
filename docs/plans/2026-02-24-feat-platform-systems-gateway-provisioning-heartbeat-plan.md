---
title: "feat: Platform Systems — Gateway, Provisioning, Heartbeat, Compaction"
type: feat
status: active
date: 2026-02-24
brainstorm: docs/brainstorms/2026-02-24-platform-systems-brainstorm.md
---

# Platform Systems — Gateway, Provisioning, Heartbeat, Compaction

## Overview

Turn Mika from a CLI agent into a deployed platform where executives get their own AI assistant by clicking a Telegram link. This plan covers three systems:

1. **mika-gateway** — Thin Axum service receiving Telegram webhooks, routing to per-customer containers
2. **Provisioning pipeline** — `provision.sh` + Helm chart on AWS EKS
3. **Agent features** — Conversation compaction, heartbeat/reminders, silent mode, `send_message` + `create_reminder` tools

## Problem Statement

Mika currently runs as a CLI binary. To serve 20-30 paying executives, we need:
- A Telegram-facing gateway that routes messages to isolated per-customer containers
- A provisioning pipeline that creates a customer's Mika in ~2 minutes
- Proactive intelligence (heartbeat, reminders) so Mika isn't just reactive
- Conversation compaction so context stays manageable over weeks of use

## Proposed Solution

See brainstorm at `docs/brainstorms/2026-02-24-platform-systems-brainstorm.md` for full architectural decisions. This plan details the implementation phases.

## Technical Approach

### Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                          AWS EKS Cluster                          │
│                                                                   │
│  Telegram ──webhook──▶ ┌──────────────┐    ┌──────────────┐      │
│                        │ mika-gateway │───▶│  Postgres    │      │
│                        │ (Axum)       │    │  (shared)    │      │
│                        └──────┬───────┘    └──────────────┘      │
│                               │                                   │
│              POST /message    │    POST /send (callback)          │
│                               ▼         ▲                         │
│                   ┌─────────────────────┐                        │
│                   │ mika-customer-<id>  │                        │
│                   │ Axum + agent loop   │                        │
│                   │ SQLite (PVC)        │                        │
│                   │ Tokio timers        │                        │
│                   └─────────────────────┘                        │
│                                                                   │
│  ┌──────────────┐                                                │
│  │ AWS Secrets  │  (synced via External Secrets Operator)        │
│  │ Manager      │                                                │
│  └──────────────┘                                                │
└──────────────────────────────────────────────────────────────────┘
```

### Implementation Phases

---

#### Phase 0: Prerequisites

Must complete before any Phase 2+ work.

##### 0.1 Async SQLite Wrapper (todo #027)

Wrap sync `rusqlite::Connection` for async access. Required before the Axum HTTP server can serve requests without blocking Tokio.

**Approach:** Dedicated OS thread with `std::sync::mpsc` channel + `tokio::sync::oneshot` for replies. The dedicated thread blocks on `std::sync::mpsc::Receiver::recv()` (which is fine — it's a non-Tokio thread). Callers use `tokio::sync::oneshot` to await results without blocking the Tokio runtime.

**Important:** Current `Database` methods take `&self` (not `&mut self`) because `rusqlite::Connection` uses interior mutability via `execute`/`query_row`. The wrapper can hold a `Database` directly — no `Mutex` needed on the dedicated thread since it's the sole accessor.

**Note on `save_message` signature:** The current `Database::save_message` takes `channel_type: &str` as a third parameter (see `db.rs:215`). The `DbCommand` variants must match the actual signatures, not simplified versions.

```rust
// crates/mika-agent/src/async_db.rs
use std::sync::mpsc;
use tokio::sync::oneshot;

pub struct AsyncDatabase {
    sender: mpsc::Sender<DbCommand>,
}

impl Clone for AsyncDatabase {
    fn clone(&self) -> Self {
        Self { sender: self.sender.clone() }
    }
}

enum DbCommand {
    SaveMessage {
        role: String,
        content: String,
        channel_type: String,
        reply: oneshot::Sender<Result<i64>>,
    },
    LoadRecentMessages {
        limit: usize,
        channel_types: Option<Vec<String>>,  // None = all, Some = filter
        reply: oneshot::Sender<Result<Vec<ConversationMessage>>>,
    },
    // ... one variant per Database method (~25 variants total including Phase 1 additions)
    // Each variant carries owned Strings (not &str) since they cross thread boundaries
}

impl AsyncDatabase {
    pub fn new(db: Database) -> Self {
        let (tx, rx) = mpsc::channel::<DbCommand>();
        std::thread::spawn(move || {
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    DbCommand::SaveMessage { role, content, channel_type, reply } => {
                        let _ = reply.send(db.save_message(&role, &content, &channel_type));
                    }
                    // ...
                }
            }
        });
        Self { sender: tx }
    }
}
```

**Implementation note:** With ~25 DB methods, consider a macro to reduce boilerplate:
```rust
macro_rules! db_command {
    ($name:ident { $($field:ident: $ty:ty),* } -> $ret:ty, $body:expr) => { ... }
}
```
Alternatively, accept the boilerplate — it's mechanical, type-safe, and each variant documents the async contract explicitly.

**Files:**
- New: `crates/mika-agent/src/async_db.rs`
- Edit: `crates/mika-agent/src/agent.rs` — accept `AsyncDatabase` in `AgentParams`
- Edit: `crates/mika-agent/src/cli.rs` — wrap `Database` in `AsyncDatabase` before use

**Tests:**
- Round-trip: save + load via async API
- Concurrent: multiple simultaneous reads
- Timeout: verify operations don't block Tokio runtime

##### 0.2 Fix Stale CLAUDE.md (todo #066)

Update CLAUDE.md to remove encryption references, update test count, update ToolContext description. Blocks correct AI-assisted development.

**File:** `CLAUDE.md`

---

#### Phase 1: Agent Features

No external infrastructure needed. All changes within `mika-agent` crate. Can be tested with CLI.

##### 1.1 Schema Migration v5

Add new tables and columns to support compaction, reminders, and heartbeat rate limiting.

```sql
-- Migration v5

-- Add compaction support to conversations
-- role column already exists (user/assistant), add 'summary' as valid value
-- Add compacted_through_id for summary rows
ALTER TABLE conversations ADD COLUMN compacted_through_id INTEGER;

-- Reminders (persisted Tokio timer state)
CREATE TABLE IF NOT EXISTS reminders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    fire_at TEXT NOT NULL,            -- ISO 8601 UTC
    message TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, delivered, failed, cancelled
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    delivered_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_reminders_status_fire_at ON reminders(status, fire_at);

-- Heartbeat send rate limiting
CREATE TABLE IF NOT EXISTS heartbeat_sends (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sent_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_heartbeat_sends_sent_at ON heartbeat_sends(sent_at);

-- Customer config (timezone, chat_id for outbound)
CREATE TABLE IF NOT EXISTS customer_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Failed outbound sends (retry queue for /send failures)
CREATE TABLE IF NOT EXISTS failed_sends (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text TEXT NOT NULL,
    request_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    retry_count INTEGER NOT NULL DEFAULT 0
);
```

**Files:**
- Edit: `crates/mika-agent/src/db.rs` — add `migrate_v5()`, bump `CURRENT_SCHEMA_VERSION` to 5
- Edit: `crates/mika-agent/src/db.rs` — add DB methods:
  - `add_reminder(fire_at, message) -> Result<i64>`
  - `get_pending_reminders() -> Result<Vec<Reminder>>`
  - `get_future_reminders() -> Result<Vec<Reminder>>` (fire_at > now)
  - `get_past_due_reminders() -> Result<Vec<Reminder>>` (fire_at <= now AND status = pending)
  - `mark_reminder_delivered(id) -> Result<()>`
  - `mark_reminder_failed(id) -> Result<()>`
  - `cancel_reminder(id) -> Result<bool>`
  - `list_active_reminders() -> Result<Vec<Reminder>>`
  - `record_heartbeat_send() -> Result<()>`
  - `count_heartbeat_sends_today(timezone: &str) -> Result<u32>`
  - `count_heartbeat_sends_last_hour() -> Result<u32>`
  - `prune_old_heartbeat_sends(days: u32) -> Result<()>`
  - `save_conversation_summary(summary, compacted_through_id) -> Result<i64>`
  - `delete_compacted_messages(through_id) -> Result<u32>`
  - `load_conversation_summary() -> Result<Option<ConversationMessage>>`
  - `get_customer_config(key) -> Result<Option<String>>`
  - `set_customer_config(key, value) -> Result<()>`

**Tests:**
- Reminder CRUD lifecycle (create, list, deliver, cancel)
- Heartbeat rate limit counting (hourly, daily with timezone)
- Summary save + load + old message deletion
- Migration v4 → v5 (verify existing data preserved)

##### 1.2 Conversation Compaction

Async post-turn summarization of old messages.

**Compaction logic:**

```rust
// crates/mika-agent/src/compaction.rs

const COMPACTION_THRESHOLD: usize = 50;  // Total messages before triggering
const CONTEXT_WINDOW: usize = 20;        // Recent messages to keep in full

pub async fn maybe_compact(db: &AsyncDatabase, claude: &ClaudeClient) -> Result<()> {
    let total = db.count_messages().await?;
    if total <= COMPACTION_THRESHOLD {
        return Ok(());
    }

    let summary_exists = db.load_conversation_summary().await?.is_some();
    let old_messages = db.load_messages_before_window(CONTEXT_WINDOW).await?;
    if old_messages.is_empty() {
        return Ok(());
    }

    // Build summarization prompt
    let existing_summary = if summary_exists {
        db.load_conversation_summary().await?
    } else {
        None
    };

    let summary_text = summarize_messages(claude, &old_messages, existing_summary.as_ref()).await?;

    // Atomic: save summary + delete old messages in one transaction
    let highest_id = old_messages.last().map(|m| m.id).unwrap_or(0);
    db.replace_with_summary(summary_text, highest_id).await?;

    Ok(())
}
```

**Summarization prompt** (in `crates/mika-agent/src/compaction.rs`):
```
You are summarizing a conversation between an AI executive assistant and their user.
Preserve: key decisions, action items, commitments, user preferences, important facts about people.
Discard: pleasantries, small talk, repeated information.
Keep the summary concise (under 500 tokens). Use bullet points.
If there is an existing summary, merge it with the new information.
```

**Context assembly update:**
```rust
// In prompt.rs or agent.rs, update context loading:
let summary = db.load_conversation_summary().await?;
let recent = db.load_recent_messages(CONTEXT_WINDOW).await?;
// Build messages: [summary as system context] + recent messages
```

**Files:**
- New: `crates/mika-agent/src/compaction.rs`
- Edit: `crates/mika-agent/src/agent.rs` — spawn compaction after response:
  ```rust
  // After saving assistant response (line ~133):
  let db_clone = db.clone();
  let claude_clone = claude.clone();
  tokio::spawn(async move {
      if let Err(e) = compaction::maybe_compact(&db_clone, &claude_clone).await {
          tracing::warn!(error = %e, "compaction failed");
      }
  });
  ```
- Edit: `crates/mika-agent/src/agent.rs` — load summary in context assembly
- Edit: `crates/mika-agent/src/db.rs` — add `replace_with_summary()` (transactional), `count_messages()`, `load_messages_before_window()`

**Concurrency safety:** Compaction runs async but modifies the same SQLite. The async DB wrapper serializes all operations through a single thread, so no concurrent writes are possible. If a new message arrives during compaction, it queues behind the compaction's DB operations.

**Tests:**
- Compaction triggers at threshold (50 messages)
- Compaction skips below threshold
- Summary replaces old messages atomically
- Context assembly includes summary + recent messages
- Multiple compaction rounds (summary grows incrementally)

##### 1.3 create_reminder Tool

New agent tool for scheduling reminders.

```rust
// crates/mika-agent/src/tools/create_reminder.rs

pub struct CreateReminderTool;

impl Tool for CreateReminderTool {
    fn name(&self) -> &str { "create_reminder" }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_reminder".into(),
            description: "Schedule a reminder for the user at a specific time. \
                The fire_at parameter must be an ISO 8601 datetime string (e.g., '2026-02-25T15:00:00Z'). \
                Parse the user's natural language time into ISO 8601 before calling this tool.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "fire_at": {
                        "type": "string",
                        "description": "ISO 8601 datetime (UTC) when the reminder should fire"
                    },
                    "message": {
                        "type": "string",
                        "description": "The reminder message to deliver to the user"
                    }
                },
                "required": ["fire_at", "message"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let fire_at = input["fire_at"].as_str().unwrap_or("");
        let message = input["message"].as_str().unwrap_or("");

        if fire_at.is_empty() || message.is_empty() {
            return Ok(ToolOutput::error("Both 'fire_at' and 'message' are required."));
        }

        // Validate ISO 8601 and ensure future time
        let parsed = DateTime::parse_from_rfc3339(fire_at)
            .map_err(|_| anyhow!("Invalid ISO 8601 datetime"))?;
        if parsed <= Utc::now() {
            return Ok(ToolOutput::error("Reminder time must be in the future."));
        }

        validate_len(message, 10_000)?;

        let id = ctx.db.add_reminder(fire_at, message)?;

        // Schedule Tokio timer (via callback in ToolContext)
        if let Some(scheduler) = &ctx.reminder_scheduler {
            scheduler.schedule(id, parsed).await?;
        }

        Ok(ToolOutput::success(format!(
            "Reminder #{} scheduled for {}.", id, fire_at
        )))
    }
}
```

**Companion tools:**
- `list_reminders` — Shows active reminders (id, fire_at, message, status)
- `cancel_reminder` — Cancels by ID

**Files:**
- New: `crates/mika-agent/src/tools/create_reminder.rs`
- New: `crates/mika-agent/src/tools/list_reminders.rs`
- New: `crates/mika-agent/src/tools/cancel_reminder.rs`
- Edit: `crates/mika-agent/src/tools/mod.rs` — register in `default_tools()`
- Edit: `crates/mika-agent/src/tools/mod.rs` — add `reminder_scheduler: Option<Arc<ReminderScheduler>>` to `ToolContext`

**ToolContext breaking change:** Adding `reminder_scheduler` and `message_sender` (Phase 1.4) to `ToolContext` changes the struct shape. Currently `ToolContext` has 5 fields and is constructed in `agent.rs:94` and in all tool tests. Both new fields are `Option<...>` so existing code sets them to `None`.

Impact: Every `ToolContext { ... }` construction site must add the new fields. There are ~5 test files that construct `ToolContext`. Add a builder or `Default`-based constructor to reduce future churn:
```rust
impl<'a> ToolContext<'a> {
    pub fn new(db: &'a Database, session_id: &'a str, home_dir: &'a Path) -> Self {
        Self {
            db, session_id, home_dir,
            core_memory_edit_count: &AtomicU32::new(0),  // problem: needs owned AtomicU32
            is_onboarding: false,
            reminder_scheduler: None,
            message_sender: None,
        }
    }
}
```
Note: `core_memory_edit_count: &'a AtomicU32` borrows, so a simple `new()` can't create it. Keep struct literal construction but document the field additions clearly.

**Tests:**
- Create reminder with valid future time
- Reject past time
- Reject invalid ISO 8601
- List shows active reminders
- Cancel marks as cancelled
- Cancelled reminders don't fire

##### 1.4 send_message Tool

Outbound messaging tool used in silent mode (heartbeat/reminders).

```rust
// crates/mika-agent/src/tools/send_message.rs

pub struct SendMessageTool;

impl Tool for SendMessageTool {
    fn name(&self) -> &str { "send_message" }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "send_message".into(),
            description: "Send a message to the user. Use this in heartbeat and reminder mode \
                to deliver proactive messages. In conversation mode, prefer responding directly.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The message to send to the user"
                    },
                    "urgency": {
                        "type": "string",
                        "enum": ["high", "normal", "low"],
                        "description": "Message priority. Currently all deliver immediately during active hours."
                    }
                },
                "required": ["text"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let text = input["text"].as_str().unwrap_or("");
        if text.is_empty() {
            return Ok(ToolOutput::error("'text' is required."));
        }
        validate_len(text, 10_000)?;

        // Route based on context:
        // - CLI mode: print to stdout (for testing)
        // - HTTP mode: POST to gateway /send endpoint
        match &ctx.message_sender {
            Some(sender) => {
                sender.send(text).await?;
                Ok(ToolOutput::success("Message sent."))
            }
            None => {
                // CLI fallback: just log it
                tracing::info!(text, "send_message (CLI mode)");
                Ok(ToolOutput::success("Message delivered (CLI)."))
            }
        }
    }
}
```

**Files:**
- New: `crates/mika-agent/src/tools/send_message.rs`
- Edit: `crates/mika-agent/src/tools/mod.rs` — register, add `message_sender: Option<MessageSender>` to `ToolContext`

**MessageSender trait:**
```rust
// crates/mika-agent/src/messaging.rs
#[async_trait]
pub trait MessageSender: Send + Sync {
    async fn send(&self, text: &str) -> Result<()>;
}

// CLI implementation (for testing)
pub struct CliMessageSender;

// HTTP implementation (for production)
pub struct GatewayMessageSender {
    client: reqwest::Client,
    gateway_url: String,
    chat_id: i64,
}
```

**Tests:**
- Send with CLI sender (prints to stdout)
- Send with mock HTTP sender
- Reject empty text
- Validate input length

##### 1.5 Silent Mode Agent Loop

Variant of `run_agent` for background tasks (heartbeat, reminders).

```rust
// crates/mika-agent/src/agent.rs

pub struct SilentAgentParams<'a> {
    pub db: &'a Database,
    pub claude: &'a ClaudeClient,
    pub tools: &'a ToolRegistry,
    pub trigger: SilentTrigger,  // Heartbeat or Reminder { id, message }
    pub home_dir: &'a Path,
}

pub enum SilentTrigger {
    Heartbeat,
    Reminder { id: i64, message: String },
}

pub async fn run_silent_agent(params: &SilentAgentParams<'_>) -> Result<()> {
    // Build heartbeat/reminder-specific system prompt
    // Include: core memory, pending commitments, timezone
    // Add: "You are in SILENT MODE. Your text output is NOT delivered.
    //        Use send_message tool to contact the user."

    // Run agent loop (same structure as run_agent but):
    // - No user message saved to conversations
    // - Save agent actions to conversations with channel_type = "heartbeat"/"reminder"
    // - If no send_message tool call: no-op (silent exit)

    // For reminders: mark as delivered/failed after execution
}
```

**Conversation history for silent mode:** Save with `channel_type = "heartbeat"` or `"reminder"`. Exclude from `load_recent_messages` for user-initiated turns.

**`load_recent_messages` filter change:** Add an optional `channel_types` parameter:
```rust
// crates/mika-agent/src/db.rs
pub fn load_recent_messages(
    &self,
    limit: usize,
    channel_types: Option<&[&str]>,  // None = all, Some = whitelist
) -> Result<Vec<ConversationMessage>> {
    // When Some(&["cli", "telegram"]), add:
    //   WHERE channel_type IN ('cli', 'telegram')
    // When None, no filter (used by compaction which needs all messages)
}
```

**Breaking change:** This changes the signature of an existing public method. All call sites must be updated:
- `agent.rs:81` — pass `Some(&["cli", "telegram"])`
- `cli.rs` — unchanged (goes through agent)
- New `compaction.rs` — pass `None` (needs all messages to count/compact)
- Tests — update `load_recent_messages(N)` → `load_recent_messages(N, None)`

**Summary rows in context assembly:** Summary rows (role = "summary") must NOT be mixed into the recent messages by ID order. The context loading should be:
```rust
// In agent.rs context assembly:
let summary = db.load_conversation_summary()?;  // Separate query: WHERE role = 'summary'
let recent = db.load_recent_messages(20, Some(&["cli", "telegram"]))?;  // Excludes summaries
// Build: [system prompt with summary injected] + [recent messages]
```
The summary is injected into the system prompt (after core memory), NOT as a message in the conversation history. This avoids Claude seeing a "summary" role it doesn't understand.

**Files:**
- Edit: `crates/mika-agent/src/agent.rs` — add `run_silent_agent()`, `SilentAgentParams`, `SilentTrigger`
- Edit: `crates/mika-agent/src/prompt.rs` — add `build_heartbeat_prompt()`, `build_reminder_prompt()`
- Edit: `crates/mika-agent/src/db.rs` — add `channel_type` filter to `load_recent_messages()`

**Tests:**
- Silent agent with no send_message call → no output
- Silent agent with send_message call → message delivered
- Heartbeat messages excluded from normal conversation context
- Reminder delivery updates status

##### 1.6 Reminder Scheduler

Manages Tokio timers for scheduled reminders. Handles startup recovery.

```rust
// crates/mika-agent/src/scheduler.rs

pub struct ReminderScheduler {
    db: AsyncDatabase,
    // ... agent dependencies for running silent loops
}

impl ReminderScheduler {
    /// On startup: recover all pending reminders
    pub async fn recover(&self) -> Result<()> {
        // Past-due reminders: fire immediately
        let past_due = self.db.get_past_due_reminders().await?;
        for reminder in past_due {
            self.fire_reminder(reminder).await;
        }

        // Future reminders: schedule Tokio timers
        let future = self.db.get_future_reminders().await?;
        for reminder in future {
            self.schedule(reminder.id, reminder.fire_at_parsed()?).await?;
        }
        Ok(())
    }

    pub async fn schedule(&self, id: i64, fire_at: DateTime<Utc>) -> Result<()> {
        let delay = (fire_at - Utc::now()).to_std().unwrap_or(Duration::ZERO);
        let db = self.db.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // Run silent agent with reminder trigger
            // Mark as delivered or failed
        });
        Ok(())
    }
}
```

**Files:**
- New: `crates/mika-agent/src/scheduler.rs`
- Edit: `crates/mika-agent/src/cli.rs` — call `scheduler.recover()` on startup

**Tests:**
- Schedule future reminder → fires after delay
- Recover past-due reminders on startup → fire immediately
- Recover future reminders on startup → re-scheduled
- Cancelled reminder doesn't fire after recovery

---

#### Phase 2: Container HTTP Server

Transform mika-agent from CLI-only to an HTTP server that accepts messages from the gateway.

##### 2.1 Axum HTTP Server

Add an HTTP server mode to mika-agent.

```rust
// crates/mika-agent/src/server.rs

pub async fn run_server(settings: &Settings) -> Result<()> {
    let db = AsyncDatabase::new(Database::open(&settings.db_path)?);
    let claude = ClaudeClient::new(/* ... */);
    let tools = default_tools();

    // Recover reminders
    let scheduler = ReminderScheduler::new(db.clone(), /* ... */);
    scheduler.recover().await?;

    let state = AppState {
        db: Arc::new(db),
        claude: Arc::new(claude),
        tools: Arc::new(tools),
        scheduler: Arc::new(scheduler),
        request_queue: Arc::new(Mutex::new(())),  // Serialization lock
        settings: settings.clone(),
    };

    let app = Router::new()
        .route("/message", post(handle_message))
        .route("/heartbeat", post(handle_heartbeat))
        .route("/health", get(handle_health))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], settings.server_port.unwrap_or(8080)));
    info!(%addr, "starting mika-agent server");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

**Request serialization:** Use a `Mutex<()>` to serialize all agent loop executions. Only one agent loop runs at a time per container (single customer). Incoming requests queue behind the lock. This prevents SQLite contention and ensures conversation history is consistent.

**Endpoints:**

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `POST /message` | Inbound | Gateway forwards user messages |
| `POST /heartbeat` | Inbound | K8s CronJob triggers heartbeat evaluation |
| `GET /health` | Health | K8s liveness/readiness probe |

**POST /message request:**
```json
{
  "text": "Can you check on the Q3 strategy deck?",
  "chat_id": 123456789,
  "message_id": 42,
  "channel": "telegram",
  "request_id": "req-a1b2c3"
}
```

**POST /message response:** `202 Accepted` (immediately). Agent runs async, replies via gateway `/send`.
```json
{ "request_id": "req-a1b2c3", "status": "accepted" }
```

**Error responses from /message:**
- `429 Too Many Requests` — agent loop already running (request queued behind Mutex but queue depth > 1 should be rejected)
- `503 Service Unavailable` — container is still starting (health check not yet passed)

**POST /heartbeat request:**
```json
{ "trigger": "organic", "request_id": "hb-d4e5f6" }
```

**POST /heartbeat response:**
- `200 OK` — heartbeat accepted, agent will evaluate
- `204 No Content` — pre-filter rejected (recently messaged, outside active hours, rate limit)
- `429 Too Many Requests` — agent loop busy

**GET /health response:**
```json
{ "status": "ok", "uptime_secs": 3600, "db_ok": true, "reminders_recovered": true }
```
Health returns `503` until DB is opened AND reminder recovery completes.

**Internal authentication:** Gateway ↔ container communication uses a shared bearer token (`MIKA_INTERNAL_TOKEN`) set during provisioning via Helm chart. Both gateway and all customer containers receive the same token. Gateway includes `Authorization: Bearer <token>` on all requests to containers. Containers validate the token on `/message` and `/heartbeat` endpoints. This prevents a compromised pod from sending messages to other customer containers.

**Request tracing:** All requests carry a `request_id` field (generated by gateway, UUID format). Container logs the `request_id` on every operation. `/send` callbacks include the `request_id` for correlation. This enables end-to-end tracing: Telegram webhook → gateway → container → /send → Telegram API.

**Session ID in HTTP mode:** Each inbound `/message` request starts a new `session_id` (UUID). This matches the CLI behavior where each exchange is independent for audit logging purposes. Heartbeat and reminder triggers also get unique session IDs.

**Files:**
- New: `crates/mika-agent/src/server.rs`
- New: `crates/mika-agent/src/server/handlers.rs`
- New: `crates/mika-agent/src/server/state.rs`
- Edit: `crates/mika-agent/Cargo.toml` — add `axum`, `tower`, `tower-http` dependencies
- Edit: `crates/mika-agent/src/main.rs` or new binary — `mika-server` binary alongside `mika-cli`

**Binary structure:**
```toml
# Cargo.toml
[[bin]]
name = "mika-cli"
path = "src/cli.rs"

[[bin]]
name = "mika-server"
path = "src/server_main.rs"
```

**Heartbeat pre-filter logic** (runs BEFORE acquiring the Mutex or calling Claude):
```rust
// crates/mika-agent/src/server/handlers.rs
async fn heartbeat_pre_filter(db: &AsyncDatabase) -> bool {
    let tz = db.get_customer_config("timezone").await
        .ok().flatten().unwrap_or_else(|| "UTC".to_string());

    // 1. Active hours check (8:00-21:00 in customer's timezone)
    let now_local = Utc::now().with_timezone(&tz.parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC));
    let hour = now_local.hour();
    if hour < 8 || hour >= 21 {
        return false;  // Outside active hours
    }

    // 2. Rate limit: max 1 organic heartbeat/hour
    if db.count_heartbeat_sends_last_hour().await.unwrap_or(0) >= 1 {
        return false;
    }

    // 3. Rate limit: max 3 organic heartbeats/day (customer's timezone)
    if db.count_heartbeat_sends_today(&tz).await.unwrap_or(0) >= 3 {
        return false;
    }

    // 4. Skip if user messaged recently (within last 2 hours)
    if let Ok(Some(last_msg)) = db.last_user_message_time().await {
        if Utc::now() - last_msg < chrono::Duration::hours(2) {
            return false;
        }
    }

    // 5. Skip if no actionable context (no pending commitments, no overdue items)
    let has_pending = db.list_commitments("pending").await.map(|c| !c.is_empty()).unwrap_or(false);
    let has_overdue_reminders = db.get_past_due_reminders().await.map(|r| !r.is_empty()).unwrap_or(false);
    if !has_pending && !has_overdue_reminders {
        // Also check: has it been >48h since last interaction?
        if let Ok(Some(last_msg)) = db.last_user_message_time().await {
            if Utc::now() - last_msg < chrono::Duration::hours(48) {
                return false;  // Nothing to say, and user was active recently
            }
        }
    }

    true
}
```

**Additional DB methods needed for pre-filter:**
- `last_user_message_time() -> Result<Option<DateTime<Utc>>>` — most recent `WHERE role = 'user' AND channel_type IN ('cli', 'telegram')`

**Dependency:** `chrono-tz` crate for timezone-aware active hours check. Add to `Cargo.toml`.

**Tests:**
- Health endpoint returns 200
- Health endpoint returns 503 during startup (before DB + recovery)
- Message endpoint returns 202 and processes asynchronously
- Message endpoint returns 429 when queue depth exceeded
- Heartbeat endpoint with pre-filter (skip if recently messaged)
- Heartbeat pre-filter: outside active hours → 204
- Heartbeat pre-filter: rate limit exceeded → 204
- Heartbeat pre-filter: no actionable context → 204
- Concurrent messages serialized (second waits for first to finish)

##### 2.2 Outbound Message Routing

Wire `send_message` tool to POST to gateway's `/send` endpoint.

```rust
// crates/mika-agent/src/messaging.rs

pub struct GatewayMessageSender {
    client: reqwest::Client,
    gateway_url: String,  // From MIKA_ROUTING_URL
    chat_id: i64,         // From customer_config table or MIKA_CHAT_ID env
}

#[async_trait]
impl MessageSender for GatewayMessageSender {
    async fn send(&self, text: &str) -> Result<()> {
        let resp = self.client
            .post(format!("{}/send", self.gateway_url))
            .json(&json!({
                "chat_id": self.chat_id,
                "text": text
            }))
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Gateway send failed: {}", resp.status());
        }
        Ok(())
    }
}
```

**Retry logic on container side:** If the gateway `/send` call fails, retry once after 2 seconds. If it fails again, log the error and save the unsent message to a `failed_sends` table (new in migration v5). On next successful interaction, attempt to flush failed sends. This prevents silent message loss when the gateway is temporarily down.

```sql
-- In migration v5 (add to 1.1 schema)
CREATE TABLE IF NOT EXISTS failed_sends (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text TEXT NOT NULL,
    request_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    retry_count INTEGER NOT NULL DEFAULT 0
);
```

**Config:**
- `MIKA_ROUTING_URL` — Gateway's internal K8s URL (already in `Settings`)
- `MIKA_INTERNAL_TOKEN` — Shared bearer token for gateway ↔ container auth (new config field)
- `MIKA_CHAT_ID` — Customer's Telegram chat ID (set during pairing, stored in `customer_config` SQLite table)

**Files:**
- New: `crates/mika-agent/src/messaging.rs`
- Edit: `crates/mika-agent/src/server.rs` — inject `GatewayMessageSender` into `ToolContext`
- Edit: `crates/mika-common/src/config.rs` — add `internal_token: Option<String>` and `chat_id: Option<i64>` to `Settings`, redact `internal_token` in Debug impl

---

#### Phase 3: mika-gateway

New crate. Thin Axum service (~500 lines).

##### 3.1 Crate Setup

```
crates/mika-gateway/
├── Cargo.toml
├── src/
│   ├── main.rs          # Entry point
│   ├── config.rs        # Gateway-specific settings
│   ├── routes.rs        # Axum router
│   ├── telegram.rs      # Webhook parsing + signature validation
│   ├── routing.rs       # customer lookup + container forwarding
│   ├── pairing.rs       # /start deep link handling
│   └── db.rs            # Postgres connection (sqlx)
```

**Cargo.toml dependencies:**
```toml
[dependencies]
mika-common = { path = "../mika-common" }
axum = "0.8"
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono"] }
reqwest = { workspace = true }
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }
uuid = { version = "1", features = ["v4"] }
hmac = "0.12"
sha2 = "0.10"
```

**Files:**
- New: entire `crates/mika-gateway/` directory
- Edit: root `Cargo.toml` — already auto-includes via `members = ["crates/*"]`

##### 3.2 Shared Postgres Schema

```sql
-- migrations/001_customers.sql

CREATE TABLE customers (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    plan TEXT NOT NULL DEFAULT 'standard',
    status TEXT NOT NULL DEFAULT 'provisioned',
    telegram_chat_id BIGINT UNIQUE,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    service_url TEXT,
    paired_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_customers_telegram_chat_id ON customers(telegram_chat_id);
CREATE INDEX idx_customers_status ON customers(status);
```

**sqlx migration approach:** Use `sqlx::migrate!()` macro with `migrations/` directory in the gateway crate.

##### 3.3 Telegram Webhook Handler

```rust
// crates/mika-gateway/src/telegram.rs

/// Validate Telegram webhook signature
pub fn validate_webhook(
    secret_token: &str,
    header_token: Option<&str>,
) -> bool {
    header_token.map_or(false, |h| h == secret_token)
}

/// Parse Telegram Update into internal message
pub fn parse_update(update: &TelegramUpdate) -> Option<InboundMessage> {
    let message = update.message.as_ref()?;
    let chat_id = message.chat.id;
    let text = message.text.as_deref()?;

    // Check for /start command (pairing)
    if let Some(payload) = text.strip_prefix("/start ") {
        return Some(InboundMessage::Start {
            chat_id,
            customer_id: payload.trim().to_string(),
        });
    }

    Some(InboundMessage::Text {
        chat_id,
        text: text.to_string(),
        message_id: message.message_id,
    })
}
```

**Webhook setup:** On gateway startup, call Telegram's `setWebhook` API with the gateway's public URL and `secret_token`.

##### 3.4 Customer Routing

```rust
// crates/mika-gateway/src/routing.rs

pub async fn route_message(
    pool: &PgPool,
    client: &reqwest::Client,
    chat_id: i64,
    text: &str,
) -> Result<(), RoutingError> {
    // Lookup customer
    let customer = sqlx::query_as!(Customer,
        "SELECT id, status, service_url FROM customers WHERE telegram_chat_id = $1",
        chat_id
    )
    .fetch_optional(pool)
    .await?;

    let customer = match customer {
        Some(c) => c,
        None => return Err(RoutingError::UnknownUser),
    };

    match customer.status.as_str() {
        "active" => {}
        "suspended" => return Err(RoutingError::Suspended),
        _ => return Err(RoutingError::NotPaired),
    }

    let service_url = customer.service_url
        .ok_or(RoutingError::NoServiceUrl)?;

    // Forward to container (fire-and-forget)
    let resp = client
        .post(format!("{}/message", service_url))
        .json(&json!({
            "text": text,
            "chat_id": chat_id,
            "channel": "telegram"
        }))
        .timeout(Duration::from_secs(5))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => Ok(()),
        Ok(r) => Err(RoutingError::ContainerError(r.status().as_u16())),
        Err(e) => Err(RoutingError::ContainerUnreachable(e.to_string())),
    }
}
```

##### 3.5 Customer Pairing

```rust
// crates/mika-gateway/src/pairing.rs

pub async fn pair_customer(
    pool: &PgPool,
    customer_id_str: &str,
    chat_id: i64,
) -> Result<PairingResult, PairingError> {
    let customer_id = Uuid::parse_str(customer_id_str)
        .map_err(|_| PairingError::InvalidUuid)?;

    let result = sqlx::query!(
        "UPDATE customers SET telegram_chat_id = $1, paired_at = now(), status = 'active'
         WHERE id = $2 AND telegram_chat_id IS NULL",
        chat_id,
        customer_id
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        // Check why: does the customer exist? Already paired?
        let exists = sqlx::query_scalar!(
            "SELECT telegram_chat_id FROM customers WHERE id = $1",
            customer_id
        )
        .fetch_optional(pool)
        .await?;

        match exists {
            None => Err(PairingError::CustomerNotFound),
            Some(Some(_)) => Err(PairingError::AlreadyPaired),
            Some(None) => Err(PairingError::Unknown),
        }
    } else {
        Ok(PairingResult::Success)
    }
}
```

##### 3.6 Outbound Relay (/send)

```rust
// In routes.rs

async fn handle_send(
    State(state): State<AppState>,
    Json(payload): Json<SendPayload>,
) -> impl IntoResponse {
    let result = state.telegram_client
        .send_message(payload.chat_id, &payload.text)
        .await;

    match result {
        Ok(_) => StatusCode::OK,
        Err(e) => {
            warn!(chat_id = payload.chat_id, error = %e, "Telegram send failed");

            // Detect bot blocked (403)
            if e.is_blocked() {
                // Mark customer as suspended after N consecutive failures
                // (tracked in-memory or Postgres)
            }

            StatusCode::BAD_GATEWAY
        }
    }
}
```

##### 3.7 Error Handling

| Scenario | Gateway behavior |
|----------|-----------------|
| Unknown chat_id | Reply "I don't recognize you. If you have an invite link, please use it." |
| Suspended customer | Reply "Your account is currently suspended. Please contact your administrator." |
| Container unreachable | Reply "I'm having a moment. Please try again shortly." + log error |
| Telegram send fails | Log error, retry once after 1s, then give up |
| User blocked bot | After 3 consecutive 403s from Telegram, mark customer `suspended` |
| Duplicate webhook | Deduplicate by `update_id` (track last seen in-memory) |

**Files:**
- New: `crates/mika-gateway/src/errors.rs` — `RoutingError`, `PairingError` enums with thiserror
- Edit: `crates/mika-gateway/src/routes.rs` — error-to-Telegram-message mapping

**Tests:**
- Route to active customer → 202
- Route to unknown chat_id → error message sent
- Route to suspended customer → suspension message sent
- Pair new customer → success
- Pair already-paired customer → rejection message
- Pair invalid UUID → rejection message
- Send to Telegram → success
- Deduplicate same update_id → skip

---

#### Phase 4: Provisioning & Deployment

##### 4.1 Dockerfile

```dockerfile
# Dockerfile
FROM rust:1.85-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin mika-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/mika-server /usr/local/bin/
USER 1000
EXPOSE 8080
CMD ["mika-server"]
```

**Files:**
- New: `Dockerfile` (root)
- New: `.dockerignore`

##### 4.2 Helm Chart

```
helm/mika-customer/
├── Chart.yaml
├── values.yaml
├── templates/
│   ├── deployment.yaml
│   ├── service.yaml
│   ├── pvc.yaml
│   ├── external-secret.yaml
│   └── _helpers.tpl
```

**Key values.yaml:**
```yaml
customer:
  id: ""          # Set by provision.sh
  name: ""
  plan: "standard"
  timezone: "UTC"

image:
  repository: <ecr-repo>/mika-agent
  tag: latest

resources:
  requests:
    memory: "32Mi"
    cpu: "50m"
  limits:
    memory: "128Mi"
    cpu: "500m"

persistence:
  size: "1Gi"
  storageClass: "gp3"

gateway:
  url: "http://mika-gateway.mika-system.svc.cluster.local"

secrets:
  awsSecretName: ""  # mika/customers/<id>
```

**Files:**
- New: entire `helm/mika-customer/` directory
- New: `helm/mika-gateway/` — separate chart for the gateway

##### 4.3 provision.sh

```bash
#!/bin/bash
set -euo pipefail

CUSTOMER_NAME=${1:?"Usage: provision.sh <name> [plan] [--timezone TZ]"}
PLAN=${2:-standard}
TIMEZONE=${TIMEZONE:-UTC}
CUSTOMER_ID=$(uuidgen | tr '[:upper:]' '[:lower:]')
NAMESPACE="mika-customers"

echo "Provisioning customer: ${CUSTOMER_NAME} (${PLAN})"

# Step 1: Create AWS secret
aws secretsmanager create-secret \
  --name "mika/customers/${CUSTOMER_ID}" \
  --secret-string "{\"anthropic_api_key\": \"${MIKA_ANTHROPIC_API_KEY}\"}" \
  --region "${AWS_REGION:-us-east-1}" || true

# Step 2: Helm install
SERVICE_URL="http://mika-${CUSTOMER_ID}.${NAMESPACE}.svc.cluster.local:8080"
helm install "mika-${CUSTOMER_ID}" ./helm/mika-customer \
  --namespace "${NAMESPACE}" --create-namespace \
  --set customer.id="${CUSTOMER_ID}" \
  --set customer.name="${CUSTOMER_NAME}" \
  --set customer.plan="${PLAN}" \
  --set customer.timezone="${TIMEZONE}" \
  --set secrets.awsSecretName="mika/customers/${CUSTOMER_ID}" \
  --wait --timeout 120s

# Step 3: Register in Postgres
psql "${DATABASE_URL}" -c "
  INSERT INTO customers (id, name, plan, timezone, service_url, status)
  VALUES ('${CUSTOMER_ID}', '${CUSTOMER_NAME}', '${PLAN}', '${TIMEZONE}', '${SERVICE_URL}', 'provisioned')
  ON CONFLICT (id) DO NOTHING;
"

echo ""
echo "Customer provisioned: ${CUSTOMER_ID}"
echo "Telegram link: https://t.me/${TELEGRAM_BOT_USERNAME}?start=${CUSTOMER_ID}"
```

**Idempotency:** `--wait` ensures pod is ready. `ON CONFLICT DO NOTHING` prevents duplicate Postgres rows. AWS secret `|| true` skips if exists.

**SQL injection prevention:** The `psql` call interpolates `CUSTOMER_NAME` directly into SQL. This is a command injection vector. Fix by using parameterized queries:
```bash
psql "${DATABASE_URL}" -c "
  INSERT INTO customers (id, name, plan, timezone, service_url, status)
  VALUES (\$1, \$2, \$3, \$4, \$5, 'provisioned')
  ON CONFLICT (id) DO NOTHING;
" --set=1="${CUSTOMER_ID}" --set=2="${CUSTOMER_NAME}" ...
```
Or better: use a small Python/Rust helper binary that uses parameterized queries. For 20-30 customers, even a `curl` call to the gateway's admin API would work.

**Container startup sequence and error handling:**
1. Open SQLite DB → if fails, exit with non-zero (K8s restarts)
2. Run migrations → if fails, exit with non-zero
3. Seed core memory if empty → if fails, exit with non-zero
4. Create ClaudeClient → always succeeds (just builds reqwest client)
5. Recover reminders → **if fails, log warning but continue** (non-fatal: reminders will be recovered on next restart)
6. Bind Axum listener → if fails, exit with non-zero
7. Health endpoint starts returning 200 → K8s marks pod as ready
8. Start serving requests

Steps 1-4 happen before the Axum listener binds. Health check returns 503 until step 7. This means the gateway won't route traffic to a container that hasn't finished startup (K8s readiness probe gates traffic).

##### 4.4 deprovision.sh

```bash
#!/bin/bash
set -euo pipefail

CUSTOMER_ID=${1:?"Usage: deprovision.sh <customer_id>"}
NAMESPACE="mika-customers"

echo "Deprovisioning customer: ${CUSTOMER_ID}"

# Step 1: Mark suspended in Postgres (graceful)
psql "${DATABASE_URL}" -c "
  UPDATE customers SET status = 'suspended' WHERE id = '${CUSTOMER_ID}';
"

# Step 2: Helm uninstall
helm uninstall "mika-${CUSTOMER_ID}" --namespace "${NAMESPACE}" || true

# Step 3: Delete PVC
kubectl delete pvc "mika-${CUSTOMER_ID}-data" -n "${NAMESPACE}" || true

# Step 4: Delete AWS secret
aws secretsmanager delete-secret \
  --secret-id "mika/customers/${CUSTOMER_ID}" \
  --force-delete-without-recovery || true

# Step 5: Remove from Postgres
psql "${DATABASE_URL}" -c "
  DELETE FROM customers WHERE id = '${CUSTOMER_ID}';
"

echo "Customer ${CUSTOMER_ID} deprovisioned."
```

**Files:**
- New: `scripts/provision.sh`
- New: `scripts/deprovision.sh`

##### 4.5 Gateway Dockerfile

The gateway needs its own Dockerfile (or a multi-stage Dockerfile that builds both binaries):

```dockerfile
# Dockerfile.gateway
FROM rust:1.85-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin mika-gateway

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/mika-gateway /usr/local/bin/
USER 1000
EXPOSE 8080
CMD ["mika-gateway"]
```

**Alternative:** Single Dockerfile with build args to select which binary to build. Saves CI time by caching shared dependencies.

**Files:**
- New: `Dockerfile.gateway` (or modify `Dockerfile` with `ARG BINARY=mika-server`)

##### 4.6 Gateway Configuration

| Env var | Description |
|---------|-------------|
| `DATABASE_URL` | Postgres connection string (gateway's shared DB) |
| `TELEGRAM_BOT_TOKEN` | Bot API token for sending messages |
| `TELEGRAM_WEBHOOK_SECRET` | Secret token for validating inbound webhooks |
| `MIKA_INTERNAL_TOKEN` | Shared bearer token for gateway ↔ container auth |
| `GATEWAY_PORT` | Listen port (default: 8080) |

Store `TELEGRAM_BOT_TOKEN` and `TELEGRAM_WEBHOOK_SECRET` in AWS Secrets Manager, synced via External Secrets Operator.

---

## Failure Modes & Recovery

Comprehensive analysis of what can go wrong and how the system responds.

### Container crash after 202 Accepted

**Scenario:** Gateway forwards message to container, gets 202, container crashes mid-agent-loop.
**Impact:** User gets no response. Message is saved in SQLite (first thing `run_agent` does), so it's not lost.
**Detection:** K8s restarts the container. On restart, the last conversation message has no assistant response following it.
**Current mitigation:** None — user must re-send. Acceptable for Phase 2 (20-30 users, white-glove support).
**Future mitigation:** Gateway tracks pending request IDs. If no `/send` callback within 5 minutes, send "Sorry, I had a hiccup. Could you repeat that?" via Telegram.

### Gateway unavailable during /send callback

**Scenario:** Container finishes agent loop, calls gateway `/send`, gateway is temporarily down.
**Impact:** User gets no response despite agent successfully processing.
**Mitigation:** Container retries once after 2s. On second failure, saves to `failed_sends` table. Flushes on next successful interaction.

### Reminder fires during active agent loop

**Scenario:** Tokio timer fires for a reminder, but the `Mutex<()>` is held by a user-initiated agent loop.
**Impact:** Reminder delivery delayed until current loop finishes (up to 5 minutes worst case).
**Mitigation:** Acceptable. Reminders are not real-time critical. The 5-minute agent timeout caps the delay.

### Compaction failure

**Scenario:** Claude summarization API call fails during async compaction.
**Impact:** Old messages are preserved (the transactional `replace_with_summary` never runs). Compaction retries on the next conversation turn.
**Mitigation:** Built-in. The threshold check triggers again next turn.

### Concurrent /send calls ordering

**Scenario:** Agent calls `send_message` twice in one loop (e.g., multi-step reasoning with two outbound messages).
**Impact:** Messages arrive at Telegram in the order the container sends them. No reordering risk unless gateway-side retry introduces delay on the first message.
**Mitigation:** Container sends sequentially within a single agent loop (tool execution is sequential). Ordering is preserved.

### Postgres connection loss (gateway)

**Scenario:** Gateway loses connection to shared Postgres during routing.
**Impact:** All message routing fails. Users get "I'm having a moment" error message.
**Mitigation:** sqlx connection pool with automatic reconnect. Gateway health check includes DB connectivity. K8s restarts gateway if health fails.

### SQLite corruption (container)

**Scenario:** Container SQLite file is corrupted (disk failure, unexpected shutdown during WAL write).
**Impact:** Container fails to start, K8s restarts repeatedly (CrashLoopBackOff).
**Mitigation:** WAL mode + `PRAGMA synchronous = NORMAL` minimizes corruption risk. PVC snapshots provide backup. For Phase 2, manual intervention is acceptable (20-30 users).

---

## Acceptance Criteria

### Functional Requirements

- [ ] **Gateway routes Telegram messages** to correct customer container via Postgres lookup
- [ ] **Deep link pairing** works: click link → /start → paired → onboarding starts
- [ ] **Single-use enforcement**: second click on same deep link is rejected with user-friendly message
- [ ] **Unknown users** receive a polite "I don't recognize you" message
- [ ] **Suspended customers** receive a suspension notice
- [ ] **Container unreachable** triggers a "try again" message to user
- [ ] **Conversation compaction** triggers when history exceeds threshold
- [ ] **Compaction preserves** recent messages (last 20) and replaces older ones with summary
- [ ] **create_reminder** tool accepts ISO 8601 future time and schedules delivery
- [ ] **Reminders survive** container restart (persisted to SQLite, recovered on startup)
- [ ] **Past-due reminders** fire immediately on container restart
- [ ] **cancel_reminder** prevents future firing
- [ ] **Heartbeat pre-filter** skips if: outside active hours, recently messaged, rate limit exceeded
- [ ] **Silent mode** produces no output unless agent calls send_message
- [ ] **Rate limiting** enforced: max 1 organic heartbeat/hour, 3/day per customer
- [ ] **Scheduled reminders** are exempt from heartbeat rate limits
- [ ] **provision.sh** creates: AWS secret, Helm release, Postgres row, outputs deep link
- [ ] **deprovision.sh** cleanly removes: Helm release, PVC, AWS secret, Postgres row

### Non-Functional Requirements

- [ ] Gateway response time < 100ms (just routing, no agent logic)
- [ ] Container cold start < 5s (health check ready)
- [ ] Compaction does not block the user's response path
- [ ] All agent loops serialized per container (no concurrent SQLite writes)
- [ ] Webhook signature validation on every inbound request
- [ ] No customer data passes through gateway (routing metadata only)

### Quality Gates

- [ ] All existing tests pass (62+ tests)
- [ ] New tests for: compaction, reminders, send_message, gateway routing, pairing
- [ ] `cargo clippy` clean
- [ ] `cargo fmt` applied
- [ ] CLAUDE.md updated with new crate, tool, and command documentation

## Dependencies & Prerequisites

| Dependency | Status | Blocks |
|-----------|--------|--------|
| Todo #027 (async SQLite) | Pending | Phase 2 (HTTP server) |
| Todo #066 (stale CLAUDE.md) | Pending | AI-assisted development |
| AWS EKS cluster | Not provisioned | Phase 4 (deployment) |
| Shared Postgres instance | Not provisioned | Phase 3 (gateway) |
| Telegram bot token | Not created | Phase 3 (gateway) |
| ECR repository | Not created | Phase 4 (Docker image) |

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| sqlite-vec alpha instability | Medium | High | Trait abstraction; pgvector fallback path |
| Compaction loses critical context | Low | High | Summarization prompt tuned for exec context; keep last 20 messages |
| Reminder timer drift on busy runtime | Low | Low | Tokio timers are precise; agent timeout bounds max delay to 5 min |
| Gateway becomes bottleneck | Low | Medium | Stateless; scale replicas horizontally |
| Helm chart complexity | Medium | Low | Start minimal; add complexity when needed |
| Silent message loss (container crash after 202) | Medium | High | Phase 2: accept risk (20-30 users). Phase 3: gateway-side timeout detection |
| provision.sh SQL injection | High | High | Use parameterized queries or admin API; never interpolate user input into SQL |
| ToolContext field churn breaks tests | High | Low | Accept churn; new fields are Option so just add `None`. Consider builder pattern later |
| AsyncDatabase boilerplate explosion (~25 variants) | High | Low | Mechanical work; consider macro. Budget 2-3 hours for initial implementation |
| chrono-tz dependency size | Low | Low | Only used for heartbeat pre-filter; ~600KB. Acceptable. |

## References & Research

### Internal References
- Brainstorm: `docs/brainstorms/2026-02-24-platform-systems-brainstorm.md`
- v2 plan: `docs/plans/2026-02-23-feat-mika-v2-rust-rewrite-plan.md`
- Architecture: `docs/brainstorms/2026-02-16-mika-technical-architecture-brainstorm.md`
- Home directory: `docs/brainstorms/2026-02-23-mika-home-directory-agent-core-brainstorm.md`
- Agent loop: `crates/mika-agent/src/agent.rs`
- Database schema: `crates/mika-agent/src/db.rs`
- Tool system: `crates/mika-agent/src/tools/mod.rs`
- Config system: `crates/mika-common/src/config.rs`

### Reference Codebases
- OpenClaw: `/home/samidarko/workspace/senara-solutions/openclaw/` — Gateway patterns, compaction, cron system
- LettaBot: `/home/samidarko/workspace/senara-solutions/lettabot/` — Silent mode, heartbeat skip logic, channel adapters

### Pending Todos Affected
- #027: Async SQLite (prerequisite)
- #066: Stale CLAUDE.md (prerequisite)
- #063: update_commitment_status silent noop (fix during Phase 1)
- #067: System prompt missing tool docs (update when adding new tools)

---

## Post-Deepening Review Findings

Review conducted 2026-02-24 after the deepening agents' context was lost. Findings integrated into the plan above.

### Gaps Found and Fixed

1. **AsyncDatabase mpsc type mismatch (Phase 0.1):** Original code used `mpsc::channel(64)` (tokio semantics with capacity) but `while let Ok(cmd) = rx.recv()` (std::sync semantics). Fixed to explicitly use `std::sync::mpsc` for the receiver thread and documented the threading model.

2. **Missing internal authentication (Phase 2/3):** No auth between gateway and container. Added `MIKA_INTERNAL_TOKEN` bearer token shared via Helm values. Without this, any compromised pod could message any customer container.

3. **Missing request tracing (Phase 2):** No way to correlate gateway → container → /send callback. Added `request_id` field to all inter-service requests.

4. **Session ID undefined in HTTP mode (Phase 2):** Plan didn't specify whether session_id is per-request or per-container. Clarified: per-request UUID, matching CLI behavior.

5. **Container startup sequence undefined (Phase 4):** No specification for what happens if DB open, migration, or reminder recovery fails. Added explicit startup sequence with fatal vs. non-fatal error handling.

6. **Heartbeat pre-filter logic missing (Phase 2):** Plan mentioned pre-filter conceptually but had no implementation. Added complete pre-filter with: active hours check (8-21 local time), hourly/daily rate limits, recent interaction skip, and actionable context check.

7. **`load_recent_messages` breaking change (Phase 1.5):** Adding channel_type filtering changes the method signature. Documented the impact on all 4+ call sites and the migration strategy.

8. **Summary row context assembly (Phase 1.2):** Summary rows (role="summary") could be mixed into recent messages by ID order. Clarified that summaries are loaded separately and injected into the system prompt, not the message history.

9. **provision.sh SQL injection (Phase 4.3):** Customer name interpolated directly into SQL. Flagged with fix options.

10. **Gateway Dockerfile missing (Phase 4):** Only agent Dockerfile was specified. Added Phase 4.5 for gateway Dockerfile.

11. **Gateway configuration env vars missing (Phase 3):** No specification of env var names for Postgres, Telegram token, webhook secret. Added Phase 4.6 configuration table.

12. **Container /send retry logic missing (Phase 2.2):** If gateway is temporarily down when container calls /send, message is silently lost. Added retry-once logic + `failed_sends` table for persistence.

13. **Health check during startup (Phase 2.1):** Health endpoint behavior during startup was undefined. Added 503 response until DB open + reminder recovery complete.

14. **ToolContext breaking change (Phase 1.3):** Adding 2 Option fields to ToolContext breaks all test construction sites. Documented impact and mitigation.

### Risks Identified But Deferred

- **No circuit breaker for dead containers:** Gateway routes to container, gets timeout, tells user "try again." If container is in CrashLoopBackOff, every message gets the same error. Acceptable for 20-30 users (operator notices quickly). Add circuit breaker in Phase 3+.
- **No message delivery confirmation to user:** User sends message, gets no visual feedback that Mika is working on it. Could add "typing" indicator via Telegram `sendChatAction` API. Defer to post-MVP polish.
- **PVC backup/restore not defined:** If SQLite corruption occurs, no automated recovery. Manual intervention acceptable for Phase 2 scale.
- **`chrono-tz` adds a new dependency:** Required for timezone-aware heartbeat pre-filter. ~600KB, no security concerns. Add to workspace dependencies.
