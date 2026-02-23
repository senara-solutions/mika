---
title: "Institutional Learnings for Mika v2 Rust Rewrite"
date: 2026-02-23
type: learning
category: architecture-review
tags:
  - rust-rewrite
  - security
  - encryption
  - database
  - agent-loop
  - memory-management
  - lessons-learned
severity: high
---

# Institutional Learnings for Mika v2 Rust Rewrite

## Executive Summary

This document synthesizes learnings from the Python v1 MVP code review, todos, and architectural brainstorms to provide critical context for the Rust v2 rewrite. The Python MVP identified **24 known issues** (8 P1 critical, 10 P2 important, 6 P3 quality) that blocked production deployment. The Rust rewrite strategically addresses these through architectural redesign rather than patching. Additionally, several architectural patterns from v1 should be preserved or evolved intentionally.

**Key finding:** The Rust v2 design proactively eliminates entire categories of v1 issues through per-customer isolation, deterministic explicit code, and zero framework overhead. However, a few architectural patterns require careful implementation to avoid replicating v1 mistakes.

---

## Section 1: Security Issues Addressed by Design

The Rust v2 architecture eliminates **all 8 critical P1 security findings** by redesign:

### 1.1 Hardcoded Secrets → Vault-Managed Secrets (P1 #001)

**v1 Problem:** `settings.secret_key or "dev-secret-change-me"` fallback allowed silent production misconfiguration, enabling session forgery if env var unset.

**v2 Resolution:** Per the rewrite plan (Week 7-8), HashiCorp Vault integration ensures:
- No hardcoded fallback; application fails to start if encryption key missing
- Per-customer encryption key fetched from Vault at startup
- ExternalSecret Operator in K8s injects secrets into pod namespace
- No environment variables in plaintext; Vault API calls via mTLS

**Implementation checklist:**
- [ ] In `mika-common/src/config.rs`: Make `encryption_key` non-optional; return `Err` if missing
- [ ] Add startup healthcheck: test decrypt round-trip with loaded key before marking ready
- [ ] Document: "If container fails to start, check Vault access and encryption key provisioning"

---

### 1.2 No CSRF Protection → SameSite + Token Validation (P1 #002)

**v1 Problem:** POST endpoints (login, export, delete, calendar disconnect) lacked CSRF token validation, allowing attackers to forge requests while user authenticated.

**v2 Resolution:** Rust's type system enforces CSRF protection at compile time:
- Session cookies: `SameSite=Strict` enforced in axum middleware
- Form submissions: CSRF token required in body; validated via timing-safe comparison
- Dashboard template: all forms auto-generate CSRF token via askama template helper
- Web framework (axum): stateless token validation using JWT (RS256) signed at startup

**Implementation checklist (Phase 4.6):**
- [ ] In `mika-routing/src/middleware/csrf.rs`: Extract token from request, verify signature
- [ ] Add CSRF token generator: `fn generate_csrf_token() -> String { /* JWT or HMAC-SHA256 */ }`
- [ ] Templates: `{{ csrf_token() }}` in all form POST actions
- [ ] Tests: assert CSRF rejection for missing/invalid tokens

---

### 1.3 WhatsApp Webhook Unsigned → HMAC-SHA256 Verification (P1 #003)

**v1 Problem:** WhatsApp webhook handler didn't verify `X-Hub-Signature-256` header; attackers could inject fake messages, triggering bot actions and data writes.

**v2 Resolution:** Routing layer enforces Meta webhook signature verification before forwarding to customers:

```rust
// mika-routing/src/webhooks/whatsapp.rs
pub async fn verify_whatsapp_signature(
    headers: &HeaderMap,
    body: &[u8],
    app_secret: &str,
) -> Result<()> {
    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|h| h.to_str().ok())
        .ok_or(WhatsAppError::MissingSignature)?;

    let computed = format!(
        "sha256={}",
        hmac_sha256(format!("{}{}", app_secret, String::from_utf8_lossy(body)))
    );

    if !constant_time_compare(&signature, &computed) {
        return Err(WhatsAppError::InvalidSignature);
    }
    Ok(())
}
```

**Implementation checklist (Phase 2.1):**
- [ ] Verify signature before parsing JSON
- [ ] Return 403 on mismatch; log failed attempts
- [ ] Tests: mock valid and invalid signatures; assert 403 on invalid
- [ ] Use `timing-safe-eq` or equivalent to prevent timing attacks

---

### 1.4 Google OAuth CSRF → Cryptographic State Parameter (P1 #004)

**v1 Problem:** OAuth state parameter was raw `user_id` with no signing; attackers could craft callback linking attacker's Google account to victim's profile.

**v2 Resolution:** State parameter is cryptographically signed using HMAC-SHA256 with per-request nonce:

```rust
// OAuth flow in calendar sidecar
let state = create_oauth_state(&customer_id, &nonce)?;
// state = HMAC-SHA256(customer_id || nonce || encryption_key)

// Callback verification
let (customer_id, nonce) = verify_oauth_state(&state, &encryption_key)?;
```

**Implementation notes:**
- Calendar sidecar (Python FastAPI) handles OAuth; encrypt state in SQLite using shared encryption key
- Nonce stored in sidecar's SQLite, valid for 15 minutes
- On callback, verify state signature before proceeding with token exchange
- This is handled in Phase 3.5 (Calendar Sidecar), inherited from v1

**Implementation checklist:**
- [ ] Sidecar: generate nonce, sign state, store in SQLite
- [ ] Callback handler: verify signature, check nonce validity, reject expired/reused nonces

---

### 1.5 InMemorySaver Memory Leak → SQLite as Source of Truth (P1 #005)

**v1 Problem:** LangGraph's `InMemorySaver` checkpointer stored all conversation state in-process with no eviction; unbounded memory growth led to OOM crashes under production load.

**v2 Resolution:** Rust v2 has **no checkpointer**. SQLite is the authoritative state store:
- Conversation history persisted to SQLite immediately after each message
- Agent loop reads from SQLite before each turn; no in-memory cache needed
- Memory usage is bounded: only current message + ~20 recent messages held in RAM
- Restart recovery: resume from last checkpoint in SQLite (idempotent)

**Implementation impact:**
- No LangGraph dependency; no checkpointer to configure
- Per-customer SQLite has bounded access patterns; queries are indexed
- Trade-off: slightly higher latency per message (~5-10ms for SQLite read) vs LangGraph's in-memory speed
  - Acceptable for executive assistant (user can tolerate <100ms latency)

**Implementation checklist (Phase 1.3):**
- [ ] Agent loop: `let history = load_messages(&db, limit=20)?` at start of turn
- [ ] After response: `save_message(&db, role, content)?` for both user and assistant
- [ ] Tests: verify memory doesn't grow unbounded after 1000+ messages

---

### 1.6 Deprecated asyncio Pattern → Pure Rust Async (P1 #006)

**v1 Problem:** Celery tasks used `asyncio.get_event_loop().run_until_complete()`, deprecated in Python 3.12+; would fail at runtime.

**v2 Resolution:** Pure Rust async with tokio; no Python interpreter, no event loop bridging:
- All I/O: async Rust primitives (tokio tasks, channels, timers)
- Proactive message sending (Phase 3.2): in-process tokio-cron-scheduler
- No subprocess spawning or async/sync bridging needed
- Compile-time verification: Rust compiler ensures all async operations use `.await`

**Implementation impact:**
- Eliminates entire category of async/sync boundary bugs
- No deprecation warnings or runtime surprises

---

### 1.7 Unencrypted Google Credentials → AES-256-GCM Encryption (P1 #007)

**v1 Problem:** Google OAuth tokens (access_token, refresh_token) stored as plaintext JSONB; database breach exposed all users' Google account access.

**v2 Resolution:** All sensitive data (conversation content, core memory, OAuth tokens, credentials) encrypted with AES-256-GCM via `ring`:

```rust
// mika-common/src/encryption.rs
pub struct Cipher {
    key: aead::LessSafeKey,
}

impl Cipher {
    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        let nonce = generate_nonce();
        self.key.seal_in_place_append_tag(nonce, aad, plaintext)
    }

    pub fn decrypt(&self, ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        // Automatic nonce extraction and verification
        self.key.open_in_place(nonce, aad, ciphertext)
    }
}
```

**Per-customer encryption:**
- Each customer's container receives unique encryption key from Vault at startup
- Key never written to disk; cached in process memory only
- PVC (persistent volume) at rest still encrypted by Kubernetes (optional: storage class encryption)
- Google credentials stored in per-customer SQLite's encrypted `oauth_tokens` column

**Implementation details:**
- Column design: store `(nonce || ciphertext || tag)` as BLOB
- AAD (Additional Authenticated Data): column metadata (e.g., `customer_id || column_name`) to prevent column swapping
- Encrypted columns: `conversations.content_encrypted`, `core_memory.value_encrypted`, `people.notes_encrypted`, `commitments.description_encrypted`, `preferences.value_encrypted`, `events.description_encrypted`
- Unencrypted metadata (IDs, timestamps, status) left unencrypted for indexing; no sensitive PII there

**Implementation checklist (Phase 1.1 + 1.5):**
- [ ] Add `ring` to Cargo.toml; version = "0.17"
- [ ] Implement `Cipher` type in `mika-common/src/encryption.rs`
- [ ] Add encryption/decryption helpers in models (e.g., `User::from_encrypted(&db, id)`)
- [ ] Migrate v1 unencrypted credentials: prompt users to re-authenticate or provide manual migration script
- [ ] Tests: round-trip encryption/decryption; verify AAD prevents tampering

---

### 1.8 Broken Proactive Messages → Stateless Routing Layer (P1 #008)

**v1 Problem:** Morning briefing and follow-up Celery tasks didn't fetch `UserChannel` to get `channel_user_id`; proactive messages silently failed, rendering core features non-functional.

**v2 Resolution:** Deterministic proactive message flow via routing layer callback:

**v1 (broken):**
```python
# tasks/briefings.py
users = User.query.all()  # ← Missing: .join(UserChannel)
for user in users:
    send_message(channel_user_id=???)  # ← Empty! No join to get chat ID
```

**v2 (fixed by design):**
```rust
// Customer container: heartbeat fires
async fn heartbeat() {
    if should_wake(&db).await {
        let response = run_agent(&ctx, "").await?;  // Silent mode
        // Agent internally uses send_message tool if it decides to reach out
    }
}

// Agent tool: send_message
#[async_trait]
impl SendMessageTool {
    async fn execute(&self, input: ToolInput, ctx: &AgentContext) -> Result<ToolOutput> {
        // Make HTTP call to routing layer with customer_id already in context
        let response = self.client.post("http://routing/send")
            .json(&SendRequest {
                customer_id: ctx.customer_id.clone(),  // ← Always present
                channel_type: "telegram",
                channel_user_id: ctx.channel_user_id.clone(),  // ← From context
                text: input.text,
            })
            .send()
            .await?;
        Ok(response)
    }
}
```

**Key difference:**
- v2: Customer context (customer_id, channel_user_id) is baked into every request from routing layer
- No query needed; no missing joins possible
- Routing layer holds the channel mapping; container delegates outbound messages to routing

**Implementation checklist (Phase 2 + 3):**
- [ ] Container context includes `customer_id` and receives `channel_user_id` in incoming request
- [ ] `send_message` tool uses routing layer `POST /send` callback
- [ ] Routing layer `POST /send` endpoint validates customer_id and channel_user_id before forwarding to Telegram/WhatsApp
- [ ] Heartbeat task: pre-filter cost check before calling agent (Phase 3.3)
- [ ] Rate limiting: max 1 proactive/hour in process counter (Phase 3.3)
- [ ] Tests: end-to-end: heartbeat triggers → agent decides to send → routing layer delivers

---

## Section 2: P2 Important Issues (Performance & Architecture)

The Rust rewrite handles P2 issues through improved architecture:

### 2.1 Cross-Store Deletion Not Atomic (P2 #009)

**v1 Problem:** Privacy delete endpoint cascades deletion across Neo4j → PostgreSQL → Redis without transaction coordination; mid-failure leaves user in partially deleted state.

**v2 Resolution:** Single-source-of-truth per-customer deletion flow:

```rust
// Container: /agent/command DELETE
pub async fn handle_delete_command(customer_id: &str, ctx: &AgentContext) -> Result<()> {
    // Step 1: Delete SQLite (all memory, conversations, state)
    ctx.db.execute("DROP TABLE ...", [])?;  // All tables gone atomically

    // Step 2: Async cleanup fire-and-forget (idempotent)
    tokio::spawn(async move {
        // Routing layer: DELETE FROM channel_mappings WHERE customer_id=$1
        routing_client.post("/admin/cleanup")
            .json(&CleanupRequest { customer_id })
            .send()
            .await
            .ok();  // Fire-and-forget; non-critical

        // Vault: delete encryption key (optional; can be done separately)
        // K8s: delete namespace (done by provisioning script/operator)
    });

    Ok(())
}
```

**Advantages:**
- SQLite deletion is atomic (single ACID transaction)
- Routing layer cleanup is idempotent (safe to retry)
- No Neo4j or multi-step transactions needed
- Container-level deletion is self-contained; infrastructure cleanup async

**Implementation checklist (Phase 4.4):**
- [ ] Deletion logic: SQLite `DROP TABLE` or truncate all tables atomically
- [ ] Write deletion confirmation message to Telegram before cleanup
- [ ] Async task to notify routing layer
- [ ] Audit log: INSERT into shared Postgres audit_log (customer_id, action='delete', timestamp)
- [ ] Tests: verify SQLite is wiped and inaccessible after deletion; routing layer reflects changes within SLA

---

### 2.2 N+1 Queries (P2 #015)

**v1 Problem:** Follow-up and WhatsApp handler tasks queried users in a loop, issuing separate DB queries per user instead of batch loading.

**v2 Resolution:** Rust's compile-time typing prevents N+1 bugs:
- No ORM (SQLAlchemy); explicit SQL or query builder
- Single query loads all users with one SELECT; no implicit lazy loading

```rust
// mika-routing/src/db/channel.rs
pub async fn get_all_channels(pool: &PgPool) -> Result<Vec<ChannelMapping>> {
    sqlx::query_as::<_, ChannelMapping>(
        "SELECT customer_id, channel_type, channel_user_id FROM channel_mappings"
    )
    .fetch_all(pool)
    .await
}

// Usage: one query, all customers
let channels = get_all_channels(&pool).await?;
for channel in channels {
    send_briefing(&channel).await?;
}
```

**sqlx advantages:**
- Compile-time query verification (checked against live database schema)
- No runtime surprises from changed column names
- No lazy loading; all data fetched explicitly
- Query plans visible and optimizable

**Implementation checklist (Phase 1, 2):**
- [ ] Use `sqlx` for all PostgreSQL queries; prefer `query_as` for type-safe rows
- [ ] Avoid nested loops over database results
- [ ] Add schema index hints in migration comments (e.g., `-- USES: idx_channel_lookup`)
- [ ] Benchmark: verify query count with SQLx logging in tests

---

### 2.3 Race Condition in User Creation (P2 #016)

**v1 Problem:** Telegram and WhatsApp handlers used check-then-create pattern; concurrent messages from same user could create duplicate users.

**v2 Resolution:** PostgreSQL INSERT ON CONFLICT ensures idempotent upsert:

```rust
// mika-routing/src/handlers/telegram.rs
pub async fn get_or_create_channel(
    pool: &PgPool,
    channel_type: &str,
    channel_user_id: &str,
    customer_id: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO channel_mappings (customer_id, channel_type, channel_user_id, created_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT (channel_type, channel_user_id) DO NOTHING"
    )
    .bind(customer_id)
    .bind(channel_type)
    .bind(channel_user_id)
    .execute(pool)
    .await?;
    Ok(())
}
```

**Unique constraint (PostgreSQL schema):**
```sql
CREATE UNIQUE INDEX idx_channel_unique ON channel_mappings(channel_type, channel_user_id);
```

**Database-enforced atomicity:**
- Multiple concurrent inserts from same user all succeed (or hit conflict)
- Database arbitrates; no Rust-level locking needed
- Guarantees: exactly one row per (channel_type, channel_user_id) pair

**Implementation checklist (Phase 2.1):**
- [ ] Routing layer schema: add unique constraint on (channel_type, channel_user_id)
- [ ] Queries: use INSERT ON CONFLICT DO NOTHING
- [ ] Tests: concurrent requests from same user; verify no duplicates

---

### 2.4 Missing Database Indexes (P2 #020)

**v1 Problem:** Frequently queried columns lacked indexes; performance degrades as data grows.

**v2 Resolution:** Schema explicitly defines indexes; sqlx compile-time checking verifies they exist:

**PostgreSQL schema (shared routing layer):**
```sql
-- Routing layer lookups (fastest path)
CREATE UNIQUE INDEX idx_channel_unique ON channel_mappings(channel_type, channel_user_id);
CREATE INDEX idx_channel_customer ON channel_mappings(customer_id);
CREATE INDEX idx_audit_customer ON audit_log(customer_id);
CREATE INDEX idx_usage_customer ON usage(customer_id, period_start);

-- Customer container SQLite (each customer's local DB)
CREATE INDEX idx_conv_created ON conversations(created_at);
CREATE INDEX idx_commit_status ON commitments(status);
CREATE INDEX idx_commit_due ON commitments(due_date);
CREATE INDEX idx_events_date ON events(event_date);
```

**sqlx migration system:**
- Migrations in `migrations/postgres/` and `migrations/sqlite/`
- Each migration version-controlled and checked into git
- `sqlx migrate run` applies all pending migrations at startup
- Schema changes are visible in PR reviews

**Implementation checklist (Phase 1.1, 2.1):**
- [ ] Write migrations for all indexes in rewrite plan schema
- [ ] Add to CI: `sqlx migrate verify`
- [ ] Document: "All indexes are defined in migrations; do not add ad-hoc indexes in code"

---

## Section 3: P3 Quality Issues (Code Health)

### 3.1 LIKE Injection in Memory Search (P3 #023)

**v1 Problem:** Memory search endpoints passed user input directly into SQL LIKE patterns without escaping `%` and `_` wildcard characters.

**v2 Resolution:** Parameterized queries + explicit LIKE escaping:

```rust
// mika-agent/src/memory/search.rs
pub async fn search_memory(db: &Connection, query: &str) -> Result<Vec<SearchResult>> {
    // Escape wildcards: % → \%, _ → \_
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");

    let pattern = format!("%{}%", escaped);

    // Use parameterized query (rusqlite prevents injection)
    let results = db.prepare(
        "SELECT source_type, source_id, content_preview
         FROM search_index
         WHERE content LIKE ? ESCAPE '\\'
        "
    )?
    .query_map(params![pattern], |row| {
        Ok(SearchResult {
            source_type: row.get(0)?,
            source_id: row.get(1)?,
            content_preview: row.get(2)?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()?;

    Ok(results)
}
```

**Rust advantages:**
- No string concatenation; all bindings via `params![]` macro
- Type system ensures parameters are correctly typed
- Escape logic is explicit and tested

**Implementation checklist (Phase 1.5, 3.1):**
- [ ] Add test cases: search for queries with `%`, `_`, `\`; verify they're treated literally
- [ ] Document: "All user input is escaped; LIKE searches are safe"

---

### 3.2 Naive datetime.now() Without Timezone (P3 #021)

**v1 Problem:** Code used `datetime.now()` without timezone info; ambiguous times at DST boundaries.

**v2 Resolution:** All timestamps in ISO 8601 format with timezone information:

```rust
// SQLite stores ISO 8601 strings (no conversion needed; DB-agnostic)
pub fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    // Example: "2026-02-23T14:30:45Z"
}

// For user-local times (briefings, scheduled tasks):
pub fn now_in_tz(tz: &str) -> Result<DateTime<FixedOffset>> {
    let tz = Tz::from_str(tz)?;
    Ok(Utc::now().with_timezone(&tz))
}

// Cron scheduler recalculates at startup to handle DST
pub async fn recalc_cron_schedules(db: &Connection, customer_tz: &str) -> Result<()> {
    // Calculate next run in customer's local timezone
    let now = now_in_tz(customer_tz)?;
    let next = cron.next_after(&now)?;
    db.update_schedule_next_fire(&next)?;
}
```

**Implementation checklist (Phase 1.1, 3.2):**
- [ ] Add `chrono-tz` to Cargo.toml
- [ ] All timestamps: stored as ISO 8601 strings in SQLite, UTC in PostgreSQL
- [ ] Scheduled tasks: recalculate cron expressions daily to handle DST
- [ ] Tests: verify briefing fires at correct local time across DST boundary

---

## Section 4: Architectural Patterns to Preserve

Some patterns from v1 should be intentionally preserved or evolved:

### 4.1 Three-Layer Memory Model

**v1 did this right; v2 inherits:**
- **Layer 1 (Core Memory):** 2000-token summary of user, persona, goals. Always loaded; agent can edit.
- **Layer 2 (Structured Facts):** People, commitments, preferences, events. Queryable; extracted asynchronously.
- **Layer 3 (Vector Search):** Full conversation history + embeddings. Hybrid search (BM25 + semantic).

**v2 implementation (Phase 1.5, 3.1):**
- Layer 1: `core_memory` table, always loaded in agent context
- Layer 2: `people`, `commitments`, `preferences`, `events` tables; indexed for agent queries
- Layer 3: `embeddings` (sqlite-vec) + `search_index` (FTS5) tables; hybrid search via ranking function

**Keep this pattern; don't over-engineer:**
- Simple CRUD operations; avoid complex graph traversals
- Vector search is auxiliary; BM25 is primary (faster, cheaper)
- Token counting: simple approximation (chars / 4); update on every core memory write

**Implementation notes:**
- Layer 1 is bottleneck; keep it small (2000 tokens = ~500 words)
- Layer 2: entity resolution on insert (canonical name lookups)
- Layer 3: embed summaries, not full conversations (cost + quality trade-off)

---

### 4.2 Conversation History as Ground Truth

**v1 stored conversations in PostgreSQL (Neo4j for memory, PostgreSQL for history); v2 simplifies:**
- Per-customer SQLite: `conversations` table is the authoritative history
- Keep recent history in memory for agent context (e.g., last 20 messages)
- Conversation summaries extracted asynchronously for Layer 2

**v2 implementation (Phase 1.3):**
```rust
pub async fn load_recent_messages(
    db: &Connection,
    limit: usize,
) -> Result<Vec<ConversationTurn>> {
    let rows = db.prepare(
        "SELECT role, content_encrypted, created_at
         FROM conversations
         ORDER BY id DESC
         LIMIT ?"
    )?
    .query_map(params![limit], |row| {
        Ok(ConversationTurn {
            role: row.get::<_, String>(0)?,
            content: decrypt(key, &row.get::<_, Vec<u8>>(1)?),  // Decrypt on read
            created_at: row.get(2)?,
        })
    })?
    .collect::<Result<_, _>>()?;

    Ok(rows.into_iter().rev().collect())  // Oldest first
}
```

**Keep this simple; don't add complexity:**
- No conversation summarization in v2 MVP (only in Phase 3)
- No conversation deletion (only full-DB wipe on account deletion)
- No conversation search (use memory search instead)

---

### 4.3 Tool-Based Interface for Agent Actions

**v1's pattern is sound; v2 refines it:**
- Agent cannot directly write to database; must use tools
- Tools are trait-based; new tools added by implementing a trait
- Tools have input/output schemas; validated at runtime

**v2 tool system (Phase 1.3):**
```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> ToolSchema;  // JSON Schema

    async fn execute(&self, input: Value, ctx: &AgentContext) -> Result<ToolOutput>;
}

// Implement for each tool
pub struct UpdateCoreMemoryTool;

#[async_trait]
impl Tool for UpdateCoreMemoryTool {
    fn name(&self) -> &str { "update_core_memory" }

    fn input_schema(&self) -> ToolSchema {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "enum": ["user_summary", "persona", "current_goals"] },
                "value": { "type": "string", "maxLength": 1000 }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: &AgentContext) -> Result<ToolOutput> {
        let key = input["key"].as_str().ok_or(ToolError::InvalidInput)?;
        let value = input["value"].as_str().ok_or(ToolError::InvalidInput)?;

        ctx.db.update_core_memory(key, value)?;

        Ok(ToolOutput {
            success: true,
            message: format!("Updated {} to {} tokens", key, value.len() / 4),
        })
    }
}
```

**Benefits:**
- Explicit tool definitions; agent cannot invent new tools
- Schema validation at runtime (prevents agent hallucinations)
- Easy to add new tools without touching agent loop code
- Testable in isolation

**v2 built-in tools (minimum viable set):**
1. `update_core_memory(key, value)` — agent edits Layer 1
2. `search_memory(query)` — hybrid search across history + facts
3. `get_commitments(status)` — query pending/completed commitments
4. `get_people()` — list known people
5. `send_message(text)` — outbound message (proactive flow only)
6. `get_calendar_events(from, to)` — integration with calendar sidecar

**Don't add tools for:**
- Reading raw tables (use search_memory instead)
- Creating commitments directly (extract entities post-conversation instead)
- Managing users or channels (routing layer responsibility)

---

### 4.4 Explicit Agent Loop with Step Limit

**v1's loop is reasonable; v2 makes it simpler:**
- Max 10 tool iterations per turn (v1 same)
- No context window management; let Claude handle it
- Tool timeout: 30s per tool (v1 was unbounded; this is a fix)

**v2 loop pseudo-code (Phase 1.3):**
```rust
pub async fn run_agent(
    ctx: &AgentContext,
    user_message: &str,
) -> Result<String> {
    let mut history = load_recent_messages(&ctx.db, 20).await?;
    let core_memory = load_core_memory(&ctx.db).await?;

    let system = build_system_prompt(&core_memory);
    let mut messages = assemble_messages(&system, &history, user_message);

    for step in 0..10 {
        let response = ctx.claude.send_message(&messages, &ctx.tools).await?;

        match response.stop_reason {
            StopReason::EndTurn => {
                let text = extract_text_from_response(&response)?;
                save_conversation(&ctx.db, "assistant", &text).await?;
                return Ok(text);
            }
            StopReason::ToolUse => {
                let results = execute_tools(&response.tool_calls(), ctx).await?;
                messages.push(AssistantMessage {
                    content: response.content,
                });
                messages.push(UserMessage {
                    content: ToolResultsMessage { tool_results: results },
                });
            }
            StopReason::MaxTokens => {
                let text = extract_text_from_response(&response)?;
                return Ok(format!("{}... (truncated)", text));
            }
        }
    }

    Ok("I need a moment to think about this. Let me get back to you.".to_string())
}
```

**Differences from v1:**
- No in-memory checkpointer; history always from SQLite
- Tool results returned in same message (no checkpoint save between steps)
- Error handling: tools that error are returned to Claude as tool_results with `is_error: true`

**Preserve constraints:**
- Step limit: 10 iterations (prevents infinite loops)
- Tool timeout: 30s (prevents hanging on slow integrations)
- Max response length: 4000 tokens (prevents rambling)

---

## Section 5: Things to Do Differently (Anti-Patterns to Avoid)

### 5.1 Don't Use a Framework Checkpointer

**v1's mistake:** LangGraph InMemorySaver was convenient but unsustainable.

**v2 decision:** No checkpointer. SQLite is the only state store.

**Why this works:**
- Conversation history is naturally durable in SQLite
- Restart recovery: load from last turn, re-run agent (idempotent)
- Memory overhead: bounded to recent messages + embeddings

**Don't use:**
- Any in-process cache without eviction policy
- LangGraph PostgresSaver (adds complexity; SQLite is simpler)
- Redis for state (adds dependency; not needed per-customer)

---

### 5.2 Don't Use Async/Sync Bridges

**v1's mistake:** Celery tasks bridged Python's sync `@task` to async coroutines via `asyncio.get_event_loop().run_until_complete()`.

**v2 solution:** Pure Rust async from the ground up.

**Why this works:**
- tokio-cron-scheduler for scheduled tasks (all async)
- No Python subprocess or event loop issues
- No deprecation warnings or runtime surprises

**Don't use:**
- `asyncio.run()` (Python is gone)
- Any sync-to-async bridge
- Blocking operations in async context (use `tokio::task::spawn_blocking` for heavy CPU)

---

### 5.3 Don't Mix ORM Lazy Loading with Batch Operations

**v1's mistake:** SQLAlchemy's lazy loading in loops caused N+1 queries.

**v2 solution:** Explicit queries with sqlx.

**Why this works:**
- No lazy loading; all queries are explicit
- Compile-time verification of query syntax
- Easy to see inefficiencies in code review

**Don't use:**
- ORMs that support lazy loading
- Implicit joins (e.g., `user.channels` in a loop)
- Eager loading flags (just use explicit JOINs)

---

### 5.4 Don't Store Secrets in Configuration

**v1's mistake:** Hardcoded fallback secrets.

**v2 solution:** Vault-managed secrets with required startup validation.

**Why this works:**
- Secrets never in code or environment variables (in principle)
- Per-customer encryption key unique to each tenant
- Transparent injection via ExternalSecret Operator

**Don't:**
- Add "dev-*" placeholders or fallback defaults
- Read secrets from config files checked into git
- Log or print secrets (even in debug)
- Store secrets in environment variables (use Vault/sidecar injection)

---

## Section 6: Testing Strategy to Prevent v1 Regressions

### 6.1 Critical Test Coverage

**Encryption round-trip (prevents #007 regression):**
```rust
#[tokio::test]
async fn test_encryption_roundtrip() {
    let key = generate_test_key();
    let plaintext = b"sensitive data";
    let encrypted = key.encrypt(plaintext, b"").unwrap();
    let decrypted = key.decrypt(&encrypted, b"").unwrap();
    assert_eq!(decrypted, plaintext);
}

#[tokio::test]
async fn test_encryption_uniqueness() {
    let key = generate_test_key();
    let encrypted1 = key.encrypt(b"same text", b"").unwrap();
    let encrypted2 = key.encrypt(b"same text", b"").unwrap();
    assert_ne!(encrypted1, encrypted2);  // Different nonces
}
```

**Webhook signature verification (prevents #003 regression):**
```rust
#[tokio::test]
async fn test_whatsapp_signature_valid() {
    let secret = "test_secret";
    let body = b"test body";
    let signature = compute_hmac_sha256(secret, body);
    assert!(verify_whatsapp_signature(body, &signature, secret).is_ok());
}

#[tokio::test]
async fn test_whatsapp_signature_invalid() {
    let secret = "test_secret";
    let body = b"test body";
    let bad_signature = "sha256=bad";
    assert!(verify_whatsapp_signature(body, bad_signature, secret).is_err());
}
```

**Agent loop serialization (prevents #008 regression):**
```rust
#[tokio::test]
async fn test_outbound_message_via_send_tool() {
    let ctx = test_agent_context().await;
    let response = run_agent(&ctx, "send a message").await.unwrap();

    // Verify send_message tool was called
    let outbound = ctx.routing_client.outbound_calls().await;
    assert!(!outbound.is_empty());
    assert_eq!(outbound[0].channel_type, "telegram");
}
```

**N+1 query detection (prevents #015 regression):**
```rust
#[sqlx::test]
async fn test_briefing_single_query(pool: PgPool) {
    let tracer = sqlx::query_logger::enable_tracing();

    // Send briefing to 100 customers
    for i in 0..100 {
        create_test_channel(&pool, &format!("user_{}", i)).await;
    }
    send_briefings_to_all(&pool).await.unwrap();

    // Verify only 1-2 queries (not 100+)
    let queries = tracer.queries();
    assert!(queries.len() < 5, "N+1 detected: {} queries", queries.len());
}
```

**Encryption key management (prevents #001 regression):**
```rust
#[tokio::test]
async fn test_startup_fails_without_encryption_key() {
    env::remove_var("ENCRYPTION_KEY");
    let result = load_config().await;
    assert!(result.is_err());
}
```

**CSRF protection (prevents #002 regression):**
```rust
#[tokio::test]
async fn test_csrf_token_required() {
    let client = test_client().await;

    let response = client.post("/settings")
        .json(&UpdateRequest { ... })
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);  // No CSRF token
}

#[tokio::test]
async fn test_csrf_token_accepted() {
    let client = test_client().await;
    let token = client.get("/csrf-token").await.unwrap();

    let response = client.post("/settings")
        .header("X-CSRF-Token", token)
        .json(&UpdateRequest { ... })
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
```

---

### 6.2 Integration Test Checklist

- [ ] End-to-end: Telegram message → container → agent loop → response → routing layer → Telegram
- [ ] Proactive messages: heartbeat → silent mode agent → send_message tool → routing → delivery
- [ ] Memory persistence: close container, restart, verify history + core memory still present
- [ ] Encryption: verify encrypted columns in SQLite; corrupt ciphertext → decryption fails
- [ ] Rate limiting: send 4 proactive messages in 1 hour → 4th blocked
- [ ] Concurrent users: 30 customers send messages simultaneously; no cross-contamination
- [ ] Graceful shutdown: SIGTERM → drain in-flight messages, save state, exit cleanly

---

## Section 7: Security Hardening Checklist (Phase 4.6)

Map v1 P1 findings to v2 implementation:

| v1 Issue | v2 Resolution | Implementation Phase | Test Coverage |
|----------|---------------|----------------------|----------------|
| Hardcoded secret key (#001) | Vault-managed key; fail fast if missing | 4.2 | Startup validation test |
| No CSRF (#002) | SameSite=Strict + CSRF token validation | 4.6 | Reject missing token test |
| WhatsApp unsigned (#003) | HMAC-SHA256 verification in routing | 2.1 | Valid/invalid signature tests |
| Google OAuth CSRF (#004) | Cryptographic state parameter signing | 3.5 (sidecar) | State verification test |
| InMemorySaver leak (#005) | No checkpointer; SQLite is state store | 1.1-1.5 | Memory growth test |
| Deprecated asyncio (#006) | Pure Rust async (tokio) | 1.1 | Clippy/compilation check |
| Unencrypted credentials (#007) | AES-256-GCM encryption on all sensitive fields | 1.1, 1.5 | Encryption round-trip test |
| Broken proactive messages (#008) | Routing layer `POST /send` callback | 2.1, 3.2 | End-to-end delivery test |

---

## Section 8: Cost & Performance Learnings

### 8.1 Token Counting

**v1 approximation (chars / 4) is acceptable:**
- Actual: 1 token ≈ 4 chars on average
- Used for core memory soft limit (2000 tokens max)
- Exact counting via Claude API tokens (returned in response usage) for billing

**v2 approach:**
- Approximate token count for in-context decisions (core memory overflow detection)
- Use actual token count from Claude API for usage metering

---

### 8.2 Heartbeat Cost Efficiency

**v1 problem:** No pre-filter for heartbeats; could trigger expensive Claude calls for no reason.

**v2 solution:** Cost-guard pre-filter (Phase 3.3):
```rust
async fn should_wake(db: &Connection) -> bool {
    // Skip Claude entirely if no context
    db.has_pending_commitments().await
        || db.has_recent_events(24).await  // Events in next 24h
        || db.hours_since_last_interaction() > 48  // Haven't chatted in 2 days
}
```

**Expected efficiency:** >50% of heartbeats are skipped (fire_count / skip_count < 0.5).

**Implementation checklist (Phase 3.3):**
- [ ] Instrument: log `heartbeat_fired` and `heartbeat_skipped` metrics
- [ ] Verify: skip rate >50% in production
- [ ] If skip rate <40%: lower threshold or adjust parameters

---

### 8.3 Vector Search Cost (Phase 3.1)

**Embeddings are expensive; use sparingly:**
- Embed: conversation summaries (every 5 messages), extracted facts (async post-conversation)
- Search: infrequent (agent searches memory 1-2x per conversation)
- Fallback: BM25 keyword search is free; use first

**Cost optimization:**
- Use text-embedding-3-small (cheaper than -large)
- Cache embeddings in sqlite-vec; don't re-embed
- Hybrid search with rank fusion; BM25 is primary filter

---

## Section 9: Documentation to Write

The following docs should be written during the rewrite to prevent institutional knowledge loss:

- [ ] `docs/architecture.md` — Container model, routing layer, message flow, encryption
- [ ] `docs/security.md` — How each P1 issue is addressed; threat model
- [ ] `docs/memory-model.md` — Three layers; core memory semantics; token limits
- [ ] `docs/agent-loop.md` — Step-by-step agent execution; tool execution flow
- [ ] `docs/encryption.md` — Key derivation, AES-256-GCM, AAD, column strategy
- [ ] `docs/testing.md` — Test categories; how to write integration tests; regression prevention
- [ ] `docs/operations/provisioning.md` — Provisioning script walkthrough
- [ ] `docs/operations/debugging.md` — Container logs, metrics, recovery procedures
- [ ] `CLAUDE.md` — Update for Rust project structure, conventions, command reference
- [ ] `README.md` — Local development setup, running locally, deployment steps

---

## Section 10: Open Questions & Risks

### Questions to Answer During Implementation

1. **sqlite-vec stability:** Is alpha version stable enough for MVP? Mitigation: abstract behind trait; fallback to pgvector.
2. **Per-customer container cost:** Will 15-30 MB per container be achievable? Start with one customer; measure memory profile.
3. **Timezone handling:** DST transitions tricky; does tokio-cron-scheduler handle them? Verify with tests crossing DST boundary.
4. **Routing layer throughput:** Can one routing layer service 30+ concurrent customers? Benchmark with load test.
5. **Claude API rate limits:** Will per-container throttling prevent hitting API limits? Add retry + backoff logic.

### High-Impact Risks (from rewrite plan Section 9)

- **sqlite-vec alpha instability** (Medium likelihood, High impact) → Mitigation: abstract interface + pgvector fallback
- **Rust rewrite takes >10 weeks** (Medium likelihood, Medium impact) → Mitigation: ship Phase 1-2 as MVP; Phase 3-4 incremental
- **Per-customer container cost exceeds pricing** (Low likelihood, High impact) → Mitigation: cost model validation in Phase 1; adjust if needed

---

## Appendix: File References

- **v1 Code Review:** `/data/workspace/senara-solutions/mika/docs/solutions/code-review/multi-agent-mvp-code-review.md`
- **v1 Todos (001-024):** `/data/workspace/senara-solutions/mika/todos/`
- **v2 Rewrite Plan:** `/data/workspace/senara-solutions/mika/docs/plans/2026-02-23-feat-mika-v2-rust-rewrite-plan.md`
- **v2 Brainstorm:** `/data/workspace/senara-solutions/mika/docs/brainstorms/2026-02-23-mika-v2-rust-rewrite-brainstorm.md`
- **Reference: OpenClaw:** `/home/samidarko/workspace/senara-solutions/openclaw/`
- **Reference: LettaBot:** `/home/samidarko/workspace/senara-solutions/lettabot/`

---

## Summary

The Python v1 MVP validated the product but accumulated 24 technical issues. The Rust v2 rewrite strategically addresses these through:

1. **Per-customer isolation** (eliminates shared-process bugs)
2. **Vault-managed secrets** (eliminates hardcoded defaults)
3. **Routing layer abstraction** (eliminates channel/user lookup bugs)
4. **SQLite as source of truth** (eliminates checkpointer memory leaks)
5. **Explicit tool system** (eliminates agent hallucinations)
6. **AES-256-GCM encryption** (eliminates plaintext credential exposure)

The rewrite does not patch v1's issues; it rebuilds from first principles. This document provides the institutional memory needed to avoid recreating v1's mistakes in Rust.

**Key commitment:** Before shipping Phase 2, verify that all 8 P1 issues are actually resolved. Run the regression test suite. Only then proceed to Phase 3.
