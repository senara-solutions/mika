# Mika v2 — Rust Rewrite Brainstorm

**Date:** 2026-02-23
**Status:** Resolved
**Participants:** User, Claude

## What We're Building

A ground-up rewrite of Mika in Rust, with per-customer container isolation on Kubernetes. The goal remains the same: reduce friction for users to use their own AI assistant. The rewrite is driven by three forces:

1. **Determinism** — LLM for creativity, explicit code for everything else. Current Python/LangGraph stack leaves too much to LLM discretion (memory extraction, tool selection, behavior guardrails).
2. **Security + isolation** — One container per customer with encrypted memory. Rust's memory safety + low footprint makes per-container economics viable.
3. **Simplicity** — Drop framework dependencies (LangGraph, LangChain, Celery, Neo4j). The agent loop is just a loop. Keep it boring.

## Product Context

- **Phase 1 (0-3mo):** SaaS for execs, 20-30 paying users at 200-500 EUR/month
- **Phase 2 (3-6mo):** Multi-user, team features, first B2B pilot
- **Phase 3 (6-12mo):** White-label offering (50-200k EUR/year contracts)
- **Moat:** Memory + personalization engine that compounds per user

## Key Decisions

### 1. Language: Rust

- Production Rust experience — not a learning exercise
- Memory safety eliminates entire classes of security bugs
- Low memory footprint (~5-15 MB per container vs ~80-150 MB for Python) makes per-customer containers economically viable
- Async-native via Tokio
- Direct Anthropic HTTP API calls (no LangChain dependency)

### 2. Architecture: Per-Customer Container on Kubernetes

```
┌─────────────────────────────┐
│       Shared Postgres        │  ← auth, billing, audit, customer registry
└──────────────┬──────────────┘
               │
      ┌────────┴────────┐
      │  Routing Layer   │  ← Telegram/WhatsApp webhook → customer lookup → forward
      └────────┬────────┘
               │ (gRPC / HTTP)
    ┌──────────▼───────────┐
    │  Customer Container   │  ← one per customer
    │  ┌─────────────────┐ │
    │  │ Agent Runtime    │ │  ← Rust binary, Tokio async
    │  │ SQLite + vec     │ │  ← conversations, memory, RAG
    │  │ Persistent Vol   │ │  ← encrypted at rest
    │  └─────────────────┘ │
    └──────────────────────┘
```

**Why full isolation:** Each customer's AI state (conversations, memory, extracted facts, embeddings) lives entirely within their container's persistent volume. No cross-tenant data leakage possible. Blast radius of any failure is one customer.

### 3. Channels: Shared Bot, Per-Customer Routing

- One Mika Telegram bot, one WhatsApp Business number
- Stateless routing layer: `message in → lookup customer_id by chat_id/phone → forward to container → reply back`
- No customer data in the routing layer
- Later: BYOB (bring your own bot) as premium feature for white-label

**Why:** Zero onboarding friction. Customer signs up, gets a link, starts chatting. No BotFather, no API tokens.

### 4. Data Layer: Hybrid (SQLite per Container + Shared Postgres)

**Per-container (SQLite + sqlite-vec):**
- Conversation history
- Core memory blocks
- Structured facts (People, Commitments, Preferences, Events)
- Vector embeddings for semantic search
- Encrypted at rest (SQLCipher or application-level encryption)
- Backup = copy the volume

**Shared PostgreSQL:**
- Auth, billing, subscription state
- Customer registry (id, plan, channels, config)
- Audit logs, usage metering
- Admin dashboards, cross-tenant queries

**Drop Neo4j.** sqlite-vec handles RAG. Structured facts go in SQLite tables. No graph traversal need today. Operational burden not justified for MVP.

### 5. Agent Engine: Explicit Rust Pipeline

No framework. The agent loop is:

```rust
loop {
    let memories = retrieve_context(&db, &query).await?;
    let messages = build_prompt(system, history, memories, tool_results);
    let response = claude.messages(messages, &tools).await?;

    match response.stop_reason {
        StopReason::EndTurn => { send_reply(response.text()).await?; break; }
        StopReason::ToolUse => { tool_results = execute_tools(response.tool_calls()).await?; }
    }
}
```

Tool registry as a trait:

```rust
#[async_trait]
trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, input: Value, ctx: &AgentContext) -> Result<Value>;
}
```

The loop stays stable forever. New capabilities = new Tool implementations.

**Informed by:** OpenClaw's pi-agent-core (error recovery, timeout logic, streaming) and LettaBot's silent mode pattern.

### 6. Memory Model: Three Layers

**Layer 1 — Core Memory (always in context, agent-editable)**
- Small block (~500-1000 tokens): user summary, preferences, current goals, relationship context
- Agent tools: `update_core_memory`, `search_memory`
- Always injected into system prompt
- Creates the feeling of persistent relationship (Letta's key insight)

**Layer 2 — Structured Facts (queryable, async extraction)**
- 4 entity types (down from 7): People, Commitments, Preferences, Events
- Extracted async post-conversation via Claude structured output
- SQLite tables with proper indexes
- Enables precise queries: "what did I commit to last Tuesday?"

**Layer 3 — Vector Search (long-tail recall)**
- Embed conversation summaries + extracted facts into sqlite-vec
- Hybrid BM25 + vector search (OpenClaw's pattern)
- Catch-all when layers 1-2 don't have the answer

**Build order:** Layer 1 + 3 first (week 1, gets 80% of UX). Layer 2 added weeks 2-3.

### 7. Scheduling: Tokio Cron + Heartbeat

**Tokio cron** for deterministic tasks:
- Morning briefings (user's local 7 AM)
- Weekly summaries
- Follow-up reminders with due dates
- Schedule persisted to SQLite, survives restarts
- No Redis, no external queue

**Heartbeat** for organic proactive behavior:
- Periodic silent wake-ups (~4h with jitter)
- Agent reviews context and decides whether to reach out
- "Hey, you mentioned that interview today — how did it go?" emerges from reasoning, not scheduling
- LettaBot's silent mode pattern: suppress auto-delivery, agent uses `send_message` tool explicitly

**Cost guard:** Pre-filter before LLM call. Check if pending commitments, recent events, or >48h since last interaction. No context changes → skip heartbeat. Keeps API costs proportional to activity, not container count.

### 8. Encryption

- All SQLite data encrypted at rest (SQLCipher or application-level Fernet/AES-256-GCM)
- Per-customer encryption keys (envelope encryption via shared KMS)
- TLS for all inter-service communication
- No plaintext customer data outside the container boundary

### 9. Customer Onboarding: Invite-Only for Phase 1

- No self-serve signup, no payment integration, no provisioning UI
- White-glove: call with customer, manually provision via `kubectl apply`, send Telegram deep link
- Build a simple provisioning script (`provision.sh --name "..." --plan premium`) that templates K8s manifests, creates volumes, seeds core memory, outputs the deep link — 2 minutes, not 30
- Automate when manual provisioning becomes the bottleneck (~20+ users, month 3-4)
- The provisioning script becomes the foundation of the self-serve flow later

### 10. Container Lifecycle: Always Running (Phase 1)

- At 20-30 customers, always-on is cheap and simple
- Heartbeat and cron require a running process anyway
- No cold start latency — instant response
- Revisit scale-to-zero when customer count exceeds 200+

### 11. Google Calendar: Python Sidecar

- Keep the existing Python Calendar integration as an internal HTTP service
- Rust agent calls it as a tool: `GET /calendar/events`, `POST /calendar/create`
- Deploy as sidecar in customer pod (each customer has their own OAuth tokens)
- Don't rewrite OAuth2 in Rust — it's I/O-bound, not performance-sensitive
- Calendar awareness is table stakes for exec assistant — don't regress from v1
- When 5+ integrations exist (calendar, email, CRM), consolidate behind MCP server (v2.5)

## What We Learned From Each Source

| Source | Key Takeaway | Applied To |
|--------|-------------|------------|
| **OpenClaw** | SQLite + sqlite-vec + FTS5 hybrid search works in production | Data layer, memory layer 3 |
| **OpenClaw** | Channel plugin = composable adapter traits | Channel adapter design |
| **OpenClaw** | Skills = Markdown files injected into context | Future skills system |
| **OpenClaw** | Single-process gateway with message pipeline | Routing layer design |
| **LettaBot** | Core memory blocks (agent self-editing) create persistent relationship feel | Memory layer 1 |
| **LettaBot** | Silent mode + heartbeat for organic proactive behavior | Scheduling |
| **LettaBot** | ChannelAdapter with onMessage/onCommand callbacks (inverted control) | Channel adapter interface |
| **LettaBot** | Group message batching with debounce | Future group support |
| **Current Mika** | Structured entity extraction is valuable but 7 types was over-engineered | Memory layer 2 (simplified to 4) |
| **Current Mika** | Onboarding FSM with explicit state transitions | Keep, port to Rust |
| **Current Mika** | Proactive briefings/follow-ups with timezone awareness | Keep, move to Tokio cron |

## Architecture vs Current Mika

| Concern | Mika v1 (Python) | Mika v2 (Rust) |
|---------|-------------------|-----------------|
| Language | Python 3.12 | Rust |
| Agent framework | LangGraph + LangChain | None (explicit loop) |
| LLM client | langchain-anthropic | Direct Anthropic HTTP API |
| Relational DB | PostgreSQL (all data) | PostgreSQL (platform) + SQLite (per-customer) |
| Memory store | Neo4j knowledge graph | SQLite + sqlite-vec (embedded) |
| Memory model | LLM entity extraction → 7 node types | Core memory + 4 entity types + vector search |
| Task queue | Celery + Redis | Tokio cron + heartbeat (in-process) |
| Channel adapters | aiogram (Telegram), httpx (WhatsApp) | Rust traits, shared bot with routing |
| Isolation | Shared process, user_id filtering | Per-customer container |
| Encryption | Fernet on one field | All memory encrypted at rest |
| Orchestration | Docker Compose / Railway | Kubernetes |
| Web framework | FastAPI | axum |

## Deferred to Planning

- **Observability:** Centralized logging, metrics, tracing across 30+ containers (likely Grafana/Loki/Tempo or similar)
- **Cost modeling:** Per-customer K8s cost (compute, persistent volumes, API calls) — needs concrete numbers
- **Testing strategy:** Rust testing patterns, integration tests against SQLite, mock Claude API
- **Migration from v1:** Data migration for existing beta users (if any)

## Open Questions

None — all major architectural decisions resolved through brainstorming. Implementation details belong in the plan.

## Next Steps

1. Run `/workflows:plan` to create the implementation plan
2. Set up the Rust project structure
3. Build Layer 1 (core memory + vector search) + agent loop first
4. Add routing layer + Telegram integration
5. Get first user chatting within 2-3 weeks
