# Platform Systems Brainstorm

**Date:** 2026-02-24
**Status:** Decided
**Scope:** Gateway, provisioning, heartbeat, conversation compaction, onboarding

## What We're Building

Three systems that turn Mika from a CLI agent into a deployed platform where executives get their own AI assistant by clicking a Telegram link:

1. **mika-gateway** — Thin Axum service that receives Telegram webhooks and routes messages to per-customer containers
2. **Provisioning pipeline** — Helm chart + shell script that spins up a customer's Mika container on EKS
3. **Agent features** — Heartbeat (proactive check-ins), conversation compaction, and silent mode for background tasks

## Why This Approach

- **White-glove onboarding** — 20-30 users don't justify self-serve. Manual provisioning + Telegram deep link. Automate later when it's the bottleneck.
- **Separate gateway** — One Telegram bot token = one webhook endpoint. That endpoint must exist outside customer containers. Gateway is a router, not a brain.
- **Helm for K8s** — Right abstraction for per-customer workloads. `helm install` per customer, `helm upgrade` to ship new agent versions, `helm rollback` when something breaks.
- **Silent mode** (from Letta) — Background tasks (heartbeat, scheduled reminders) don't auto-deliver output. Agent must explicitly call `send_message` to contact the user. Prevents accidental spam.
- **Async compaction** (hybrid of OpenClaw + Letta) — Keep last N messages in full context. Summarize older ones asynchronously after the conversation turn, batched with structured fact extraction. Keeps response path fast.

## Key Decisions

### 1. Gateway Architecture

**Decision:** Separate crate (`mika-gateway`), stateless Axum HTTP service, shared Postgres for customer registry.

```
Telegram webhook → mika-gateway (one instance)
  → validate webhook signature
  → lookup chat_id → customer_id (Postgres)
  → forward message to customer container (HTTP)
  → relay response back to Telegram API
```

- ~500 lines of Rust. Handles routing, webhook validation, customer pairing, and outbound message relay. No agent logic or memory access.
- Shared Postgres stores: `customers` table (channel mappings deferred to a separate table when WhatsApp is added).
- Gateway deployed as its own K8s Deployment (1-2 replicas).
- Customer containers expose an internal HTTP endpoint for the gateway to call.
- **Async webhook pattern:** Gateway immediately returns 200 to Telegram (acknowledging receipt), then forwards to the container asynchronously. Container replies via the gateway's `/send` endpoint. All responses are outbound — eliminates Telegram's 60-second webhook timeout issue.

### 2. Provisioning Pipeline

**Decision:** `provision.sh` + Helm chart + AWS Secrets Manager + External Secrets Operator.

**Flow:**
```bash
./provision.sh "John Doe" premium
```
1. Generate UUID (`customer_id`)
2. Create secret in AWS Secrets Manager (`mika/customers/<id>`)
3. `helm install mika-<id> ./helm/mika-customer` with customer values
4. Insert customer row in shared Postgres
5. Output Telegram deep link: `https://t.me/mika_bot?start=<customer_id>`

**Customer registry (shared Postgres):**
```sql
CREATE TABLE customers (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    plan TEXT NOT NULL DEFAULT 'standard',
    status TEXT NOT NULL DEFAULT 'provisioned',  -- provisioned → active → suspended
    telegram_chat_id BIGINT UNIQUE,              -- NULL until paired
    timezone TEXT NOT NULL DEFAULT 'UTC',         -- IANA timezone (e.g., 'Europe/Berlin')
    service_url TEXT,                             -- K8s service URL, set by provision.sh
    paired_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

**Timezone:** Set during provisioning (`provision.sh --timezone Europe/Berlin`). Confirmed during conversational onboarding. Stored in Postgres (gateway needs it for heartbeat scheduling) and mirrored in per-customer SQLite (agent needs it for time-aware responses).

**EKS infrastructure:** Managed separately with Terraform. Helm manages per-customer workloads.

### 3. Telegram Onboarding (Deep Link Auto-Pair)

**Decision:** Single-use UUID deep link. No pairing codes for now.

**Flow:**
1. You provision customer, get `https://t.me/mika_bot?start=<customer_id>`
2. You send link to the customer directly (email, call, etc.)
3. Customer clicks link → Telegram sends `/start <customer_id>` to webhook
4. Gateway validates UUID exists in Postgres
5. Maps `chat_id → customer_id` (single-use — if already paired, reject)
6. Forwards to customer container → Mika starts conversational onboarding

**Single-use enforcement:**
```sql
UPDATE customers
SET telegram_chat_id = $1, paired_at = now(), status = 'active'
WHERE id = $2 AND telegram_chat_id IS NULL;
-- Returns 0 rows if already paired → reject
```

**Conversational onboarding** (already designed): Mika asks 2-3 questions, uses `update_core_memory` to seed persona/user_summary/priorities/key_people blocks. Onboarding flag set in DB after first session.

### 4. Conversation Compaction

**Decision:** Sliding window + async LLM summarization, batched with structured fact extraction.

**How it works:**
- After each conversation turn (async, non-blocking to user):
  - Check if total message count > `COMPACTION_THRESHOLD` (e.g., 50 messages)
  - If yes: take old messages (everything beyond the `CONTEXT_WINDOW` of last 20), ask Claude to summarize
  - Store summary as a special row in `conversations` table (role = `summary`, replacing the compacted messages)
  - In the same async call: sweep for structured facts the agent missed during inline extraction
- Context assembly: summary (if exists) + last 20 messages + core memory + relevant facts

**Data model:** Add a `role TEXT` column to `conversations` (values: `user`, `assistant`, `summary`). Summary rows replace compacted message ranges. A `compacted_through_id INTEGER` column on summary rows tracks the highest original message ID covered.

**Inline vs async fact extraction:** The agent continues to use `store_fact`/`update_fact` tools during conversation for high-signal facts it recognizes in real-time. The async post-turn sweep is a safety net that catches facts the agent missed. The two systems are complementary — inline is primary, async is supplementary.

**Why async:** Synchronous compaction when context nears the limit adds latency at the worst moment. Async keeps the response path fast.

**Why keep last N full:** Executives often say "remember what we discussed about X?" — the full recent messages preserve conversational coherence. Memory layers help for older context but shouldn't be the only fallback.

### 5. Heartbeat System

**Decision:** Silent mode. Tokio cron for deterministic scheduled tasks. Infrastructure-triggered heartbeat for organic check-ins.

**Two modes:**

**a) Scheduled reminders (deterministic):**
- User says "remind me tomorrow at 3pm about the board deck"
- Agent parses natural language → creates a Tokio scheduled task
- When timer fires: run agent loop in silent mode with reminder context
- Agent calls `send_message` tool to deliver the reminder
- No full cron expression system — just natural language time parsing

**NLP time parsing:** Claude extracts the structured datetime during the conversation turn (returns ISO 8601). Rust code schedules the Tokio timer from the parsed timestamp. This aligns with "LLM for creativity, explicit code for everything else."

**Persistence and restart recovery:** Reminders are persisted to a `reminders` table in per-customer SQLite (columns: `id`, `fire_at TIMESTAMP`, `message TEXT`, `status TEXT`, `created_at`). On container startup, query for future reminders and re-register Tokio timers. Past-due reminders missed during downtime fire immediately on startup.

**b) Organic heartbeat (proactive):**
- K8s CronJob or AWS EventBridge fires on interval (e.g., every 30 min)
- Sends wake-up call to customer container
- Pre-filter before running Claude: only wake if pending commitments, recent events, or >48h since last interaction
- If warranted: run agent loop in silent mode with heartbeat prompt
- Agent evaluates context, decides whether to reach out
- Calls `send_message` if action warranted; no call = no delivery

**Silent mode contract:**
- Background agent turns produce NO auto-delivered output
- Agent must explicitly use `send_message` tool to contact user
- `send_message` supports urgency levels: `high` (immediate), `normal` (during active hours), `low` (batch into next briefing). Deferred: for Phase 2 all messages deliver immediately during active hours. Urgency routing added when morning briefings are built.
- Timezone-aware: never fire during inactive hours (default 8AM-9PM user's TZ, stored in `customers.timezone`)
- Rate-limited: max 1 organic heartbeat message per hour, 3 per calendar day (user's TZ). Counter stored in per-customer SQLite (`heartbeat_sends` table with timestamps). Scheduled reminders (user-requested) do NOT count against this limit.

### 6. Gateway ↔ Container Communication

**Decision:** Internal HTTP, fully async (fire-and-forget inbound, callback-based outbound).

```
Gateway                          Customer Container
  │                                    │
  │  POST /message                     │
  │  { "text": "...", "chat_id": ... } │
  │ ──────────────────────────────────► │
  │  200 OK (accepted)                 │  (gateway returns immediately)
  │ ◄────────────────────────────────── │
  │                                    │  → agent loop runs
  │                                    │  → tool calls, memory updates
  │         POST /send                 │  → agent calls send_message
  │  { "chat_id": ..., "text": "..." } │
  │ ◄────────────────────────────────── │
  │                                    │
  │  (gateway relays to Telegram API)  │
```

- K8s Service per customer container (ClusterIP, internal only)
- Gateway discovers containers via `customers.service_url` in Postgres (set by `provision.sh`)
- All responses are outbound — container calls gateway's `/send` endpoint, gateway relays to Telegram API
- Same `/send` path for both conversation replies and heartbeat/reminder messages — unified outbound flow

## Prerequisites

1. **Async SQLite (todo #027)** — Wrap sync rusqlite in `spawn_blocking` for async. Required before the customer container HTTP endpoint (Axum) can serve requests without blocking the Tokio runtime. Must be completed first.
2. **Container HTTP endpoint** — Each mika-agent container needs an Axum server (currently CLI-only). This is the Phase 2 HTTP server already planned.

## Patterns Adopted from Reference Codebases

| Pattern | Source | Adaptation for Mika |
|---------|--------|---------------------|
| Silent mode | Letta | Background tasks don't auto-deliver. Agent uses `send_message` tool explicitly. |
| Async summarization | OpenClaw | Compaction happens post-response, batched with fact extraction. |
| Channel adapter interface | OpenClaw | Gateway normalizes Telegram messages before forwarding to container. |
| Heartbeat skip logic | Letta | Skip heartbeat if user messaged recently (configurable window). |
| Single-use deep link | Mika original | UUID-based, single-use pairing. No device codes needed for invite-only. |
| Hub-and-spoke gateway | OpenClaw | One gateway, many containers. Gateway is stateless router. |

## Patterns NOT Adopted

| Pattern | Source | Why Not |
|---------|--------|---------|
| Full cron expression system | OpenClaw | Users say "remind me Tuesday at 3pm", not `0 15 * * 2`. NLP parsing is enough. |
| Agent-managed job scheduling | OpenClaw | Too complex for Phase 2. Fixed interval heartbeat + NLP reminders cover the use cases. |
| Context delegation to SDK | Letta | Letta delegates to their SDK. Mika owns its own context — explicit compaction needed. |
| Memory block .mdx files | Letta | Mika uses SQLite core_memory table. Already built, works well. |
| Multi-agent gateway | Letta | Mika is one agent per customer. No multi-agent orchestration needed. |
| File-watcher cron reload | Letta | K8s pods don't edit their own job files. Tokio in-process scheduling is cleaner. |

## System Boundaries

```
┌─────────────────────────────────────────────────────────────┐
│                        AWS EKS Cluster                       │
│                                                              │
│  ┌──────────────┐    ┌──────────────┐                       │
│  │ mika-gateway │    │  Postgres    │                       │
│  │ (Axum, 1-2   │───▶│  (shared)    │                       │
│  │  replicas)   │    │  customers   │                       │
│  └──────┬───────┘    │  chan_maps   │                       │
│         │            └──────────────┘                       │
│         │                                                    │
│         ├─── POST /message ──▶ ┌─────────────────────┐      │
│         │                      │ mika-customer-abc   │      │
│         │                      │ (Axum + agent loop) │      │
│         │                      │ SQLite (PVC)        │      │
│         │                      └─────────────────────┘      │
│         │                                                    │
│         ├─── POST /message ──▶ ┌─────────────────────┐      │
│         │                      │ mika-customer-def   │      │
│         │                      │ (Axum + agent loop) │      │
│         │                      │ SQLite (PVC)        │      │
│         │                      └─────────────────────┘      │
│         │                                                    │
│  ┌──────┴───────┐                                           │
│  │  Telegram    │  (webhook from Telegram API)              │
│  │  Webhook     │                                           │
│  └──────────────┘                                           │
│                                                              │
│  ┌──────────────┐                                           │
│  │ AWS Secrets  │  (synced via External Secrets Operator)   │
│  │ Manager      │                                           │
│  └──────────────┘                                           │
└─────────────────────────────────────────────────────────────┘
```

## Open Questions

_None — all questions resolved during brainstorm and review._

## Resolved Questions

1. **Onboarding entry point?** → White-glove only. Manual provisioning + Telegram deep link.
2. **K8s management?** → Helm + provision.sh. No operator, no Terraform for workloads.
3. **Feature scope?** → Core set: heartbeat, gateway, compaction. No WhatsApp, no skills system, no morning briefings yet.
4. **Gateway architecture?** → Separate Axum service. Handles routing, pairing, webhook validation, outbound relay. No agent logic.
5. **Compaction strategy?** → Async sliding window + LLM summarization. Summary stored as `role=summary` row. Inline fact extraction is primary; async sweep is supplementary.
6. **Heartbeat approach?** → Tokio cron for reminders + infrastructure-triggered organic heartbeat. Silent mode.
7. **Provisioning tooling?** → Helm install + AWS Secrets Manager + External Secrets Operator. Postgres for customer registry.
8. **Silent mode?** → Yes, adopted from Letta. Background tasks must use send_message explicitly.
9. **Telegram pairing?** → Single-use UUID deep link. Auto-pair on /start. No pairing codes until self-serve.
10. **Webhook timeout?** → Async pattern. Gateway acks Telegram immediately (200), container replies via `/send` callback.
11. **NLP time parsing?** → Claude extracts ISO 8601 during conversation. Rust schedules from parsed timestamp.
12. **Reminder persistence?** → SQLite `reminders` table. Re-register Tokio timers on startup. Past-due fire immediately.
13. **Timezone?** → Stored in `customers.timezone` (Postgres) and mirrored in per-customer SQLite. Set during provisioning, confirmed during onboarding.
14. **Inline vs async facts?** → Complementary. Agent tools are primary (real-time). Async sweep catches misses.
15. **Urgency model?** → `send_message` accepts urgency param. Deferred: Phase 2 delivers all during active hours. Urgency routing added with morning briefings.
16. **Rate limits?** → Per calendar day (user's TZ). Organic heartbeats only. Stored in SQLite. Reminders exempt.
17. **Async SQLite prerequisite?** → Todo #027 must complete before container HTTP endpoint.
