---
title: "Mika v2: Rust Rewrite with Per-Customer Container Isolation"
type: feat
status: active
date: 2026-02-23
brainstorm: docs/brainstorms/2026-02-23-mika-v2-rust-rewrite-brainstorm.md
---

# Mika v2: Rust Rewrite with Per-Customer Container Isolation

## Overview

Ground-up rewrite of Mika from Python to Rust. Per-customer container isolation on Kubernetes. Three-layer memory model with encrypted SQLite. Explicit agent loop with no framework dependencies. Shared Telegram/WhatsApp bot with stateless routing. Target: 20-30 paying exec users at 200-500 EUR/month within 3 months.

## Problem Statement

The Python v1 MVP validated the product concept but has architectural limitations that block production deployment:

1. **24 known issues** (8 P1 critical) including hardcoded secrets, broken proactive messages, deprecated asyncio patterns, and memory leaks
2. **Framework overhead** — LangGraph, LangChain, Celery, Neo4j, Redis are heavy dependencies for a 4-node agent loop
3. **No tenant isolation** — shared process with `user_id` filtering is insufficient for paying customers' private data
4. **LLM over-reliance** — memory extraction, tool selection, and behavior guardrails are all LLM-driven with no deterministic fallbacks
5. **High memory footprint** — Python process (~80-150 MB) makes per-customer containers economically impractical

## Proposed Solution

A Rust backend that is:
- **Deterministic** — LLM for creativity, explicit Rust code for everything else
- **Isolated** — one container per customer with encrypted local SQLite
- **Lightweight** — ~15 MB per container, making per-customer K8s pods economical
- **Simple** — explicit agent loop, no framework, trait-based tool system

## Technical Approach

### Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Shared Infrastructure                  │
│                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │  PostgreSQL   │  │   Routing    │  │  Observability │  │
│  │  (platform)   │  │   Layer      │  │  (Loki/Prom)  │  │
│  └──────┬───────┘  └──────┬───────┘  └───────────────┘  │
│         │                 │                               │
└─────────┼─────────────────┼───────────────────────────────┘
          │                 │
          │    ┌────────────┼────────────┐
          │    │            │            │
     ┌────▼────▼──┐  ┌─────▼─────┐  ┌──▼──────────┐
     │  Customer A │  │ Customer B│  │  Customer C  │
     │  ┌────────┐│  │ ┌────────┐│  │ ┌──────────┐│
     │  │Agent   ││  │ │Agent   ││  │ │Agent     ││
     │  │Loop    ││  │ │Loop    ││  │ │Loop      ││
     │  ├────────┤│  │ ├────────┤│  │ ├──────────┤│
     │  │SQLite  ││  │ │SQLite  ││  │ │SQLite    ││
     │  │+vec    ││  │ │+vec    ││  │ │+vec      ││
     │  ├────────┤│  │ ├────────┤│  │ ├──────────┤│
     │  │Calendar││  │ │Calendar││  │ │Calendar  ││
     │  │Sidecar ││  │ │Sidecar ││  │ │Sidecar   ││
     │  └────────┘│  │ └────────┘│  │ └──────────┘│
     │  PV (enc.) │  │ PV (enc.) │  │ PV (enc.)   │
     └────────────┘  └───────────┘  └─────────────┘
```

### Rust Crate Stack

| Component | Crate | Version | Notes |
|-----------|-------|---------|-------|
| HTTP framework | `axum` | 0.8.x | Tokio-native, Tower middleware |
| Telegram bot | `teloxide` | 0.17.x | Webhook + long-polling support |
| Claude API | `reqwest` (direct) | 0.12.x | No official Rust SDK; own typed structs |
| Embeddings | `async-openai` | 0.33.x | OpenAI text-embedding-3-small |
| PostgreSQL | `sqlx` | 0.8.x | Async, compile-time checked queries |
| SQLite + vectors | `rusqlite` + `sqlite-vec` | 0.38.x / 0.1.7 | `spawn_blocking` wrapper for async |
| Encryption | `ring` | 0.17.x | AES-256-GCM for field encryption |
| Configuration | `config` (config-rs) | 0.15.x | Layered: file → env vars |
| Scheduling | `tokio-cron-scheduler` | 0.15.x | In-process cron expressions |
| Templates | `askama` | 0.15.x | Compile-time checked HTML |
| Serialization | `serde` + `serde_json` | 1.x | Throughout |
| Logging | `tracing` + `tracing-subscriber` | 0.1.x | Structured JSON logs |
| Password hashing | `argon2` | 0.5.x | Replaces bcrypt |
| Session tokens | `jsonwebtoken` | 9.x | Replaces itsdangerous |

### Key Risk: sqlite-vec is alpha (0.1.7-alpha.10)

sqlite-vec is the least mature dependency. Mitigation:
- Abstract vector search behind a trait so the implementation can be swapped
- If sqlite-vec proves unstable, fall back to pgvector on shared Postgres (each customer gets a schema)
- Pin the exact version and vendor the C source

### Key Risk: SQLCipher + sqlite-vec compatibility is untested

The brainstorm proposed SQLCipher for full-DB encryption. Research shows this combination is unverified.

**Decision: Use application-level encryption with `ring` (AES-256-GCM) instead of SQLCipher.**

Rationale:
- sqlite-vec compatibility is guaranteed (uses vanilla SQLite)
- Encrypt only sensitive columns (conversation content, core memory, fact details)
- Leave metadata columns (timestamps, IDs, status) unencrypted so indexes work
- Per-customer encryption key from Vault, cached in container memory at startup
- Trade-off: more encryption/decryption code, but no C library binding risk

## API Contracts

### Routing Layer ↔ Customer Container (HTTP/JSON)

```
# Incoming message (routing → container)
POST /agent/message
{
  "channel_type": "telegram",
  "channel_user_id": "12345678",
  "text": "Schedule a meeting with Sarah",
  "message_id": "msg_abc123"
}
→ 200 {
  "text": "I'll help you schedule that. When works best?",
  "attachments": []
}
Timeout: 120s (Claude can be slow with tool use loops)

# Commands (routing → container)
POST /agent/command
{
  "command": "export",       # export | delete | settings
  "channel_user_id": "12345678"
}
→ 200 { "type": "file", "url": "/files/export.zip" }
   | { "type": "text", "text": "All data deleted." }

# Outbound message (container → routing)
POST /send
{
  "customer_id": "uuid",
  "channel_type": "telegram",
  "channel_user_id": "12345678",
  "text": "Good morning! Here's your briefing..."
}
→ 200 { "delivered": true }

# Health (K8s probes → container)
GET /healthz → 200   # liveness: process alive
GET /ready   → 200   # readiness: SQLite open, encryption key loaded
```

### Routing Layer External API

```
# Telegram webhook (Telegram → routing)
POST /webhook/telegram
  Body: Telegram Update JSON
  Validation: X-Telegram-Bot-Api-Secret-Token header

# WhatsApp webhook (Meta → routing)
GET  /webhook/whatsapp?hub.mode=subscribe&hub.verify_token=...&hub.challenge=...
POST /webhook/whatsapp
  Body: Meta webhook payload
  Validation: X-Hub-Signature-256 header (HMAC-SHA256)

# Admin (internal)
POST /admin/provision   { customer_id, name, plan }
GET  /admin/customers   → list of customers with status
```

### Shared PostgreSQL Schema

```sql
-- Customer registry
CREATE TABLE customers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,                    -- encrypted
    plan TEXT NOT NULL DEFAULT 'pro',
    status TEXT NOT NULL DEFAULT 'provisioning',  -- provisioning, active, suspended, deleted
    timezone TEXT NOT NULL DEFAULT 'UTC',
    preferred_channel TEXT NOT NULL DEFAULT 'telegram',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ
);

-- Channel identity mapping (routing layer lookups)
CREATE TABLE channel_mappings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID NOT NULL REFERENCES customers(id),
    channel_type TEXT NOT NULL,            -- telegram, whatsapp
    channel_user_id TEXT NOT NULL,         -- telegram user ID or phone number
    last_incoming_at TIMESTAMPTZ,          -- for WhatsApp 24h window tracking
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(channel_type, channel_user_id)
);
CREATE INDEX idx_channel_lookup ON channel_mappings(channel_type, channel_user_id);

-- Invitation tokens (onboarding flow)
CREATE TABLE invitations (
    token TEXT PRIMARY KEY,                -- random 16-char token
    customer_id UUID NOT NULL REFERENCES customers(id),
    claimed BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ
);

-- Audit log (GDPR compliance)
CREATE TABLE audit_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID,                      -- nullable (survives customer deletion)
    action TEXT NOT NULL,                   -- message, export, delete, provision, etc.
    details JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_audit_customer ON audit_log(customer_id);

-- Usage metering (billing)
CREATE TABLE usage (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID NOT NULL REFERENCES customers(id),
    period_start DATE NOT NULL,
    messages_sent INT NOT NULL DEFAULT 0,
    claude_tokens_used BIGINT NOT NULL DEFAULT 0,
    embedding_tokens_used BIGINT NOT NULL DEFAULT 0,
    UNIQUE(customer_id, period_start)
);
```

### Per-Customer SQLite Schema

```sql
-- Conversations (message history)
CREATE TABLE conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    role TEXT NOT NULL,                     -- 'user' or 'assistant'
    content_encrypted BLOB NOT NULL,       -- AES-256-GCM encrypted
    channel_type TEXT NOT NULL,
    metadata TEXT,                          -- JSON, unencrypted (non-sensitive)
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_conv_created ON conversations(created_at);

-- Core memory (Layer 1 — always in context, agent-editable)
CREATE TABLE core_memory (
    key TEXT PRIMARY KEY,                  -- 'user_summary', 'persona', 'current_goals'
    value_encrypted BLOB NOT NULL,         -- AES-256-GCM encrypted
    token_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- People (Layer 2 — structured facts)
CREATE TABLE people (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    canonical_name TEXT NOT NULL UNIQUE,
    relationship TEXT,                     -- 'colleague', 'manager', 'spouse', etc.
    notes_encrypted BLOB,                  -- AES-256-GCM encrypted
    first_mentioned TEXT NOT NULL DEFAULT (datetime('now')),
    last_mentioned TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Commitments (Layer 2)
CREATE TABLE commitments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    description_encrypted BLOB NOT NULL,   -- AES-256-GCM encrypted
    description_hash TEXT NOT NULL UNIQUE,  -- SHA-256 for dedup (hash of plaintext)
    status TEXT NOT NULL DEFAULT 'pending', -- pending, completed, cancelled
    due_date TEXT,                          -- ISO 8601
    person_id INTEGER REFERENCES people(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);
CREATE INDEX idx_commit_status ON commitments(status);
CREATE INDEX idx_commit_due ON commitments(due_date);

-- Preferences (Layer 2)
CREATE TABLE preferences (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    category TEXT NOT NULL UNIQUE,          -- 'communication_style', 'meeting_time', etc.
    value_encrypted BLOB NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Events (Layer 2)
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    description_encrypted BLOB NOT NULL,
    event_date TEXT,                        -- ISO 8601
    context TEXT,                           -- unencrypted category/tag
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_events_date ON events(event_date);

-- Vector embeddings (Layer 3 — sqlite-vec)
CREATE VIRTUAL TABLE embeddings USING vec0(
    embedding float[1536]                  -- text-embedding-3-small dimensions
);

-- Embedding metadata (links vec rowid to source)
CREATE TABLE embedding_sources (
    rowid INTEGER PRIMARY KEY,             -- matches embeddings.rowid
    source_type TEXT NOT NULL,             -- 'conversation', 'fact', 'summary'
    source_id INTEGER NOT NULL,
    content_preview TEXT,                  -- first 100 chars, for debugging
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- FTS5 for BM25 keyword search (Layer 3 — hybrid search)
CREATE VIRTUAL TABLE search_index USING fts5(
    content,
    source_type,
    source_id UNINDEXED
);

-- Cron schedule persistence
CREATE TABLE schedules (
    name TEXT PRIMARY KEY,                 -- 'morning_briefing', 'weekly_summary'
    cron_expr TEXT NOT NULL,
    timezone TEXT NOT NULL,
    last_fired TEXT,
    next_fire TEXT,
    enabled BOOLEAN NOT NULL DEFAULT true
);

-- Schema version (for migrations)
CREATE TABLE schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### Onboarding Flow (End-to-End)

```
Admin: ./provision.sh --name "Vincent Dupont" --plan pro --tz "Europe/Paris"
  │
  ├─ 1. INSERT INTO customers (name, plan, timezone, status='provisioning')
  ├─ 2. INSERT INTO invitations (token=random_16_char, customer_id)
  ├─ 3. kubectl apply -f tenant-manifest.yaml (namespace, deployment, PVC, sidecar)
  ├─ 4. Wait for pod ready (kubectl wait --for=condition=ready)
  ├─ 5. UPDATE customers SET status='active'
  └─ 6. Output: https://t.me/MikaBot?start={token}

User clicks deep link → Telegram sends: /start {token}
  │
  ├─ 7. Routing layer receives webhook
  ├─ 8. Detects /start command with token
  ├─ 9. SELECT customer_id FROM invitations WHERE token=$1 AND claimed=false
  ├─ 10. INSERT INTO channel_mappings (customer_id, channel_type='telegram', channel_user_id)
  ├─ 11. UPDATE invitations SET claimed=true, claimed_at=now()
  ├─ 12. Forward to container: POST /agent/message { text: "/start", ... }
  └─ 13. Container starts consent flow (onboarding FSM)

Consent flow (in container, deterministic — no LLM):
  ├─ AWAITING_CONSENT: Show privacy notice, ask for consent
  ├─ User replies "yes" → keyword match (no LLM)
  └─ CONSENT_GRANTED: Seed core memory, start normal conversation
```

**Note:** v1's 6-state onboarding (consent → basics → pain → stuck task → wow → completed) is simplified for v2 MVP. Start with consent-only. Add guided onboarding in Phase 2 once real user patterns emerge.

### Proactive Outbound Message Path

```
Container heartbeat/cron triggers
  │
  ├─ Pre-filter: check SQLite for pending commitments, recent events, time since last interaction
  ├─ If no changes → skip (no LLM call)
  ├─ If changes → invoke agent loop in silent mode (response not auto-sent)
  │   ├─ Agent decides to reach out → uses send_message tool
  │   │   └─ Tool calls: POST http://routing-layer:8080/send
  │   │       { customer_id, channel_type, channel_user_id, text }
  │   │       Routing layer holds bot token → sends via Telegram/WhatsApp API
  │   └─ Agent decides no action → returns NO_ACTION → heartbeat ends
  │
  └─ Rate limiting: in-process counter (per container = per customer)
      Max: 1 proactive message/hour, 3/day
```

**WhatsApp 24h window:** Routing layer checks `channel_mappings.last_incoming_at`. If >24h since last incoming WhatsApp message, routing layer either:
- Uses a pre-approved template message wrapper
- Falls back to Telegram (if customer has Telegram linked)
- Skips delivery (logs the skip for debugging)

### Message Serialization

Each customer container runs a single Tokio task that processes messages sequentially:

```rust
// In customer container
let (tx, mut rx) = tokio::sync::mpsc::channel::<InboundMessage>(32);

// HTTP handler pushes to channel
async fn handle_message(State(tx): State<Sender<InboundMessage>>, Json(msg): Json<InboundMessage>) -> impl IntoResponse {
    tx.send(msg).await.map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    // Response sent via /send callback, not here
    StatusCode::ACCEPTED
}

// Single consumer serializes processing
tokio::spawn(async move {
    while let Some(msg) = rx.recv().await {
        let response = agent_loop(&msg, &db, &claude, &tools).await;
        send_reply(&routing_client, &msg, &response).await;
    }
});
```

Wait — this changes the API contract. If the container returns `202 Accepted` immediately and sends the reply asynchronously via `/send`, then the routing layer does not need to hold the connection open for 120s. This is cleaner:

1. Routing layer receives webhook → forwards to container → gets `202 Accepted` → done
2. Container processes asynchronously → calls routing `POST /send` with response
3. Routing layer sends via Telegram/WhatsApp API

This also handles typing indicators: the container can call `POST /typing` on the routing layer while processing.

**Revised contract:**

```
# Routing → Container (fire-and-forget)
POST /agent/message → 202 Accepted

# Container → Routing (async callbacks)
POST /send    { customer_id, channel_type, channel_user_id, text }
POST /typing  { customer_id, channel_type, channel_user_id }
```

## Implementation Phases

### Phase 1: Foundation + Agent Core (Weeks 1-3)

Build the minimal agent that can have a conversation via CLI (no channels yet).

#### 1.1 Project Setup (Week 1)

- [ ] Initialize Rust workspace with Cargo.toml
  ```
  mika/
    Cargo.toml (workspace)
    crates/
      mika-agent/       # Agent loop, tools, memory
      mika-routing/     # Routing layer (shared service)
      mika-common/      # Shared types, encryption, config
    config/
      default.toml
    migrations/
      postgres/         # sqlx migrations
    templates/          # askama HTML templates
  ```
- [ ] Set up `mika-common`: config loading (config-rs), encryption (ring AES-256-GCM), tracing setup, shared types (`InboundMessage`, `OutboundMessage`, `ToolDefinition`)
- [ ] Set up `mika-agent`: rusqlite + sqlite-vec initialization, SQLite schema creation with embedded migrations
- [ ] Encryption module: `encrypt(key, plaintext) -> Vec<u8>`, `decrypt(key, ciphertext) -> String`
- [ ] Key loading from env var (Phase 1) or Vault (Phase 4)
- [ ] CI: `cargo clippy`, `cargo test`, `cargo fmt --check`

#### 1.2 Claude API Client (Week 1)

- [ ] Typed request/response structs for Anthropic Messages API
  ```rust
  // mika-common/src/claude.rs
  struct MessagesRequest { model, max_tokens, system, messages, tools }
  struct MessagesResponse { id, content, stop_reason, usage }
  enum ContentBlock { Text { text }, ToolUse { id, name, input } }
  enum StopReason { EndTurn, ToolUse, MaxTokens }
  ```
- [ ] `ClaudeClient::send_message(&self, req) -> Result<MessagesResponse>`
- [ ] `ClaudeClient::send_message_streaming(&self, req) -> impl Stream<Item=StreamEvent>`
- [ ] Retry with exponential backoff (429, 500, 529)
- [ ] Usage tracking: tokens consumed per request
- [ ] Unit tests with recorded responses (no live API calls in CI)

#### 1.3 Agent Loop (Week 2)

- [ ] Tool trait definition
  ```rust
  #[async_trait]
  pub trait Tool: Send + Sync {
      fn name(&self) -> &str;
      fn definition(&self) -> ToolDefinition;
      async fn execute(&self, input: Value, ctx: &AgentContext) -> Result<Value>;
  }
  ```
- [ ] Agent context: `AgentContext { db, claude, customer_id, routing_client }`
- [ ] Core loop with step limit (max 10 tool iterations)
  ```rust
  pub async fn run_agent(ctx: &AgentContext, message: &str) -> Result<String> {
      let history = load_recent_messages(&ctx.db, 20).await?;
      let core_memory = load_core_memory(&ctx.db).await?;
      let system = build_system_prompt(&core_memory);
      let mut tool_results: Vec<ToolResult> = vec![];

      for step in 0..10 {
          let messages = assemble_messages(&system, &history, message, &tool_results);
          let response = ctx.claude.send_message(messages, &ctx.tools).await?;

          match response.stop_reason {
              StopReason::EndTurn => return Ok(extract_text(&response)),
              StopReason::ToolUse => {
                  tool_results = execute_tools(&response.tool_calls(), ctx).await?;
              }
              StopReason::MaxTokens => return Ok(extract_text(&response)),
          }
      }
      Ok("I need a moment to think about that. Let me get back to you.".into())
  }
  ```
- [ ] Tool timeout: 30s per tool call via `tokio::time::timeout`
- [ ] Error handling: tool errors returned as ToolResult with `is_error: true`

#### 1.4 Memory Layer 1 — Core Memory (Week 2)

- [ ] Core memory CRUD in SQLite (encrypted values)
- [ ] `update_core_memory` tool: agent can read/write core memory blocks
- [ ] `search_memory` tool: hybrid search over facts + vector index
- [ ] Token counting for core memory (approximate: chars / 4)
- [ ] Hard limit: 2000 tokens. If agent tries to exceed, return tool error with current size
- [ ] Seed core memory on container first start:
  ```
  user_summary: "New user. No information yet."
  persona: "Mika — personal AI executive assistant."
  current_goals: "Get to know the user and understand their needs."
  ```

#### 1.5 Memory Layer 3 — Vector Search (Week 3)

- [ ] Embedding generation via async-openai (text-embedding-3-small, 1536 dims)
- [ ] Store embeddings in sqlite-vec virtual table
- [ ] FTS5 index for BM25 keyword search
- [ ] Hybrid search: vector + BM25, weighted rank fusion
  ```rust
  pub async fn hybrid_search(db: &Connection, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
      let embedding = generate_embedding(query).await?;
      let vec_results = vec_search(db, &embedding, limit * 2)?;
      let bm25_results = fts_search(db, query, limit * 2)?;
      rank_fusion(vec_results, bm25_results, limit)
  }
  ```
- [ ] Embed: conversation summaries (every 5 messages), extracted facts
- [ ] CLI test harness: `cargo run --bin mika-agent -- --cli` for interactive testing

**Phase 1 acceptance criteria:**
- [ ] Can have a multi-turn conversation via CLI
- [ ] Core memory persists across restarts
- [ ] Vector search returns relevant past context
- [ ] Tools execute correctly (start with `update_core_memory`, `search_memory`)
- [ ] All data encrypted at rest in SQLite
- [ ] `cargo test` passes with >80% coverage on agent + memory modules

---

### Phase 2: Channels + Routing (Weeks 3-5)

Connect the agent to Telegram via a shared routing layer.

#### 2.1 Routing Layer Service (Week 3-4)

- [ ] `mika-routing` crate: axum HTTP server
- [ ] Shared Postgres connection pool (sqlx)
- [ ] Telegram webhook handler with signature verification (`X-Telegram-Bot-Api-Secret-Token`)
- [ ] Customer lookup: `channel_mappings` table query
- [ ] `/start` token claiming flow (invitations table)
- [ ] Forward to customer container via HTTP client (reqwest)
- [ ] `POST /send` endpoint for outbound messages (container → routing → Telegram)
- [ ] `POST /typing` endpoint for typing indicators
- [ ] WhatsApp webhook handler with HMAC-SHA256 signature verification
- [ ] WhatsApp 24h window tracking via `channel_mappings.last_incoming_at`
- [ ] Health check endpoints: `/healthz`, `/ready`
- [ ] Structured logging with customer_id context

#### 2.2 Customer Container HTTP API (Week 4)

- [ ] axum server inside `mika-agent` binary
- [ ] `POST /agent/message` → pushes to mpsc channel → `202 Accepted`
- [ ] `POST /agent/command` → handles /export, /delete, /settings
- [ ] `GET /healthz` → process alive
- [ ] `GET /ready` → SQLite open + encryption key loaded
- [ ] Message consumer task: reads from mpsc, runs agent loop, calls `/send` on routing layer
- [ ] Typing indicator: calls `/typing` before starting agent loop
- [ ] Graceful shutdown: SIGTERM → stop accepting, drain in-flight, persist state, exit

#### 2.3 Onboarding Flow (Week 4-5)

- [ ] Consent-only onboarding FSM (simplified from v1's 6 states)
- [ ] Keyword-based consent detection (no LLM, matching v1 pattern)
- [ ] Privacy notice text
- [ ] Post-consent: seed core memory, enable normal conversation
- [ ] Store onboarding state in per-customer SQLite

#### 2.4 Provisioning Script (Week 5)

- [ ] `scripts/provision.sh --name "..." --plan pro --tz "Europe/Paris"`
- [ ] Generates K8s namespace manifest from template
- [ ] Creates: Namespace, Deployment, Service, PVC, NetworkPolicy, Secret
- [ ] Inserts customer + invitation into shared Postgres
- [ ] Waits for pod ready
- [ ] Outputs Telegram deep link
- [ ] `scripts/deprovision.sh --customer-id <uuid>` for cleanup

**Phase 2 acceptance criteria:**
- [ ] Send a Telegram message → get a response from Mika
- [ ] Onboarding flow works end-to-end (deep link → consent → first conversation)
- [ ] Memory persists across conversations
- [ ] Webhook signatures verified (Telegram + WhatsApp)
- [ ] Multiple customers can chat simultaneously (separate containers)
- [ ] Message ordering preserved per customer (mpsc serialization)

---

### Phase 3: Proactive Intelligence (Weeks 5-7)

Make the agent feel alive — it reaches out, not just responds.

#### 3.1 Memory Layer 2 — Structured Facts (Week 5-6)

- [ ] Async post-conversation extraction via Claude structured output
  ```rust
  // After agent responds, spawn extraction task
  tokio::spawn(async move {
      let entities = extract_entities(&claude, &conversation_text).await;
      store_entities(&db, &entities).await;
      embed_entities(&db, &openai, &entities).await;
  });
  ```
- [ ] 4 entity types: People, Commitments, Preferences, Events
- [ ] Entity resolution: canonical name lookup in SQLite before insert
- [ ] Extraction prompt with JSON schema enforcement
- [ ] Retry on extraction failure (max 2 retries)
- [ ] `get_commitments` tool: agent can query pending commitments
- [ ] `get_people` tool: agent can look up people by name

#### 3.2 Tokio Cron Scheduler (Week 6)

- [ ] Schedule persistence in SQLite `schedules` table
- [ ] Morning briefing: user's local time, DST-aware
  - Calculate UTC cron expression from user timezone
  - Recalculate on timezone change or DST transition (check daily)
- [ ] Weekly summary: Sunday 3 AM user local time
- [ ] Schedule recovery on restart: load from SQLite, skip missed jobs, fire next scheduled

#### 3.3 Heartbeat + Silent Mode (Week 6-7)

- [ ] Heartbeat: every 4h ± random jitter (0-30 min)
- [ ] Cost guard pre-filter:
  ```rust
  async fn should_wake(db: &Connection) -> bool {
      db.has_pending_commitments().await
          || db.has_events_in_next_24h().await
          || db.hours_since_last_interaction() > 48
  }
  ```
- [ ] Silent mode agent invocation: same loop, but response is discarded unless agent calls `send_message` tool
- [ ] `send_message` tool: calls routing layer `POST /send`
- [ ] Rate limiting: in-process counter. Max 1 proactive/hour, 3/day. Reset at midnight local time.

#### 3.4 Morning Briefing (Week 7)

- [ ] Compose briefing context: core memory + pending commitments + today's calendar events
- [ ] Calendar events from Python sidecar: `GET http://localhost:5000/calendar/events?from=today&to=+1d`
- [ ] Briefing prompt: structured template, Claude fills in the prose
- [ ] Deliver via routing layer `POST /send`
- [ ] Save briefing as conversation turn in SQLite

#### 3.5 Calendar Sidecar (Week 7)

- [ ] Extract Google Calendar code from Python v1 into standalone FastAPI service
- [ ] Endpoints: `GET /calendar/events`, `POST /calendar/create`, `GET /health`
- [ ] OAuth token storage: encrypted in container's SQLite (accessed via shared volume)
- [ ] Token refresh on expiry (background task)
- [ ] Dockerfile for sidecar
- [ ] K8s pod spec updated: add sidecar container

**Phase 3 acceptance criteria:**
- [ ] Mika sends a morning briefing at the right time with calendar events
- [ ] Heartbeat triggers organic follow-ups ("How did the interview go?")
- [ ] Structured facts are extracted and queryable
- [ ] Pre-filter prevents unnecessary Claude API calls
- [ ] Rate limiting prevents message spam
- [ ] Calendar events appear in briefings

---

### Phase 4: Operations + Polish (Weeks 7-10)

Production-ready for 20-30 paying customers.

#### 4.1 Kubernetes Manifests (Week 7-8)

- [ ] Namespace-per-tenant template (YAML)
- [ ] Deployment: Rust agent container + Calendar sidecar
- [ ] PVC: 1 GiB SSD, dynamic provisioning, reclaimPolicy: Retain
- [ ] NetworkPolicy: default-deny + allow from routing namespace + allow egress to shared services + allow HTTPS outbound
- [ ] ResourceQuota: 200m CPU, 128Mi memory per namespace
- [ ] PodDisruptionBudget: maxUnavailable: 0
- [ ] Pod Security Standards: restricted profile
- [ ] ServiceAccount: no K8s API access (automountServiceAccountToken: false)

#### 4.2 Shared Infrastructure (Week 8)

- [ ] Routing layer Deployment + Service + Ingress
- [ ] Shared PostgreSQL (managed: Cloud SQL / RDS / Azure Database)
- [ ] Secrets management: External Secrets Operator + HashiCorp Vault
  - Per-customer encryption key in Vault at `secret/tenants/{customer_id}/encryption_key`
  - Shared secrets (bot tokens, API keys) in `secret/shared/`
  - ExternalSecret per tenant namespace pulls customer key
- [ ] Container image CI/CD: build on push, push to GHCR, deploy via rolling update

#### 4.3 Observability (Week 8-9)

- [ ] Structured JSON logs to stdout (tracing-subscriber with JSON layer)
- [ ] Grafana Alloy DaemonSet → Loki for centralized log aggregation
- [ ] Prometheus metrics endpoint per container (`/metrics`):
  - `mika_messages_total` (counter, by channel)
  - `mika_agent_latency_seconds` (histogram)
  - `mika_claude_tokens_total` (counter, by model)
  - `mika_memory_entries` (gauge, by layer)
  - `mika_heartbeat_fired_total` (counter)
  - `mika_heartbeat_skipped_total` (counter)
- [ ] ServiceMonitor per tenant namespace
- [ ] Grafana dashboards: fleet overview + per-tenant drill-down
- [ ] Alerts: TenantPodDown (5m), HighClaudeErrorRate (>10% in 15m), DiskUsageHigh (>80%)

#### 4.4 GDPR: Export + Deletion (Week 9)

- [ ] Export: gather all SQLite data → decrypt → package as JSON ZIP → send as Telegram file via routing layer
- [ ] Deletion cascade:
  1. Container: drop all SQLite tables, delete database file
  2. Routing layer: DELETE FROM channel_mappings WHERE customer_id = $1
  3. Shared Postgres: UPDATE customers SET status='deleted', deleted_at=now()
  4. K8s: delete namespace (destroys PVC + pod)
  5. Vault: delete encryption key
  6. Audit: INSERT INTO audit_log (action='customer_deleted', customer_id)
- [ ] Two-step confirmation for deletion (via Telegram conversation)
- [ ] Export/delete via `/agent/command` endpoint on container

#### 4.5 Web Dashboard (Week 9-10)

- [ ] axum routes in routing layer (shared service, not per-container)
- [ ] askama templates: login, dashboard home, memory viewer, settings
- [ ] Auth: JWT session tokens, argon2 password hashing
- [ ] Dashboard reads from routing layer API (which proxies to containers as needed)
- [ ] Settings: timezone, preferred channel, briefing opt-in/out, calendar connect
- [ ] Memory viewer: display core memory + structured facts (read-only for v2 MVP)

#### 4.6 Security Hardening (Week 10)

- [ ] All v1 P1 issues addressed by design:
  | v1 Issue | v2 Resolution |
  |----------|---------------|
  | Hardcoded secret key (#001) | Vault-managed secrets, no fallback |
  | No CSRF (#002) | SameSite=Strict cookies + CSRF token |
  | WhatsApp webhook unsigned (#003) | HMAC-SHA256 verification in routing layer |
  | Google OAuth CSRF (#004) | Cryptographic state parameter |
  | InMemorySaver leak (#005) | No checkpointer; SQLite is the state |
  | Deprecated asyncio (#006) | Pure Rust async, no bridging |
  | Unencrypted credentials (#007) | All sensitive data AES-256-GCM encrypted |
  | Broken proactive messages (#008) | Outbound path via routing layer `POST /send` |
- [ ] Security headers: CSP, X-Frame-Options, X-Content-Type-Options
- [ ] Rate limiting on auth endpoints (axum Tower middleware)
- [ ] TLS for all inter-service communication (K8s service mesh or mTLS)

**Phase 4 acceptance criteria:**
- [ ] 20-30 customers running on K8s with full isolation
- [ ] Provisioning takes <2 minutes via script
- [ ] Centralized logs and metrics visible in Grafana
- [ ] Data export and deletion work end-to-end
- [ ] Web dashboard functional for basic settings
- [ ] All v1 P1 security issues resolved

---

## Alternative Approaches Considered

| Approach | Why Rejected |
|----------|-------------|
| **Build on OpenClaw** | Single-user, TypeScript, 512 security vulnerabilities, flat-file memory |
| **Build on Letta** | Only replaces memory layer; still need channels, scheduling, dashboard |
| **Keep Python, fix issues** | 24 issues + per-container economics don't work at ~100 MB/container |
| **Go instead of Rust** | Viable, but user has production Rust experience and values memory safety |
| **Shared process with user_id isolation** | Insufficient for premium exec customers' private data |
| **SQLCipher for encryption** | Untested compatibility with sqlite-vec; application-level encryption more flexible |
| **Microservices (agent, memory, scheduler separate)** | Over-engineering for a per-customer container; one binary is simpler |

## Success Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Container memory | <30 MB idle | `kubectl top pod` |
| Cold start time | <5s to ready | Startup probe timing |
| Message response latency | <10s p95 | Prometheus histogram |
| Heartbeat cost efficiency | >50% fired result in actual message | `heartbeat_fired / heartbeat_skipped` |
| Claude API cost per user | <50 EUR/month | Usage metering table |
| Customer retention (30 day) | >80% | Active customers / total |
| System uptime | >99.5% | Pod restart count + alerting |

## Dependencies & Prerequisites

- [ ] Kubernetes cluster provisioned (AKS recommended for cost: ~$21/month at 30 tenants)
- [ ] HashiCorp Vault or equivalent KMS deployed
- [ ] Shared PostgreSQL instance provisioned
- [ ] Telegram bot created (shared) with webhook URL configured
- [ ] WhatsApp Business API access approved
- [ ] Anthropic API key with sufficient rate limits for 30 concurrent users
- [ ] OpenAI API key for embeddings (text-embedding-3-small)
- [ ] Container registry (GHCR or equivalent)
- [ ] Domain + TLS certificate for webhook endpoint

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| sqlite-vec alpha instability | Medium | High | Abstract behind trait; fallback to pgvector |
| Claude API rate limits at 30 users | Low | High | Per-container rate limiting; request queuing |
| K8s operational complexity | Medium | Medium | Start with managed K8s (AKS); automate with operator later |
| Rust rewrite takes longer than 10 weeks | Medium | Medium | Phase 1-2 is the MVP; Phase 3-4 can ship incrementally |
| Per-customer cost exceeds pricing | Low | High | Cost model validation in Phase 1; adjust plan/pricing if needed |
| Calendar sidecar adds memory overhead | Low | Low | ~30 MB Python overhead; acceptable at 20-30 users |

## Cost Model (30 customers)

| Item | Monthly Cost |
|------|-------------|
| AKS cluster (free control plane) | $0 |
| 1x B2s node (2 vCPU, 4 GB) | ~$15 |
| 30x 1 GiB SSD PVs | ~$6 |
| Managed PostgreSQL (Basic) | ~$25 |
| HashiCorp Vault (dev mode or HCP free tier) | $0 |
| Loki + Grafana (self-hosted on same node) | $0 |
| Claude API (~$1.50/user/day estimated) | ~$1,350 |
| OpenAI Embeddings (~$0.10/user/day) | ~$90 |
| **Total infrastructure** | **~$1,486/month** |
| **Revenue (30 users × 300 EUR avg)** | **~$9,000/month** |
| **Gross margin** | **~83%** |

Claude API is the dominant cost. Optimize via: prompt caching, shorter context, skip heartbeats when no context changes, Haiku for extraction tasks.

## Documentation Plan

- [ ] `README.md` — project setup, local development, deployment
- [ ] `CLAUDE.md` — updated for Rust project structure and conventions
- [ ] `docs/architecture.md` — system architecture with diagrams
- [ ] `docs/api-contracts.md` — routing layer and container API specs
- [ ] `docs/runbooks/` — provisioning, debugging, backup/restore procedures

## References & Research

### Internal References

- Brainstorm: `docs/brainstorms/2026-02-23-mika-v2-rust-rewrite-brainstorm.md`
- Previous brainstorm: `docs/brainstorms/2026-02-23-letta-openclaw-evaluation-brainstorm.md`
- v1 architecture: `docs/brainstorms/2026-02-16-mika-technical-architecture-brainstorm.md`
- v1 implementation plan: `docs/plans/2026-02-16-feat-mika-mvp-implementation-plan.md`
- v1 code review: `docs/solutions/code-review/multi-agent-mvp-code-review.md`
- v1 known issues: `todos/001-024` (8 P1, 10 P2, 6 P3)
- OpenClaw reference: `/home/samidarko/workspace/senara-solutions/openclaw/`
- LettaBot reference: `/home/samidarko/workspace/senara-solutions/lettabot/`

### External References

- [axum 0.8 documentation](https://docs.rs/axum/0.8)
- [teloxide documentation](https://docs.rs/teloxide/0.17)
- [rusqlite + sqlite-vec guide](https://alexgarcia.xyz/sqlite-vec/rust.html)
- [sqlx documentation](https://docs.rs/sqlx/0.8)
- [Anthropic Messages API](https://docs.anthropic.com/en/api/messages)
- [K8s Gateway API](https://gateway-api.sigs.k8s.io/)
- [K8s namespace-per-tenant patterns](https://kubernetes.io/docs/concepts/security/multi-tenancy/)
- [Velero backup documentation](https://velero.io/docs/)
- [External Secrets Operator](https://external-secrets.io/)
