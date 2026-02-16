# Mika Technical Architecture Brainstorm

**Date:** 2026-02-16
**Status:** Draft
**Author:** Kani + Claude
**Topic:** Technical architecture for Mika AI executive assistant MVP

---

## What We're Building

Mika is a conversation-first AI executive assistant that lives in Telegram (WhatsApp later). It listens to natural conversation, extracts context and commitments, builds a persistent knowledge graph of each user's world, and proactively acts — drafting, researching, following up — without being asked.

The core product loop: **Listen -> Remember -> Reason -> Act -> Follow Up**

The "wow moment" is: user talks naturally, Mika spots a stuck task, and delivers a real draft within 5 minutes. No integrations needed on Day 0.

---

## Why This Approach: Agent-First

We chose the Agent-First approach: build the Telegram bot + LangGraph agent + Neo4j memory core first. Web dashboard follows once the agent is proving value with real users.

**Rationale:**
- Fastest path to the "wow moment" — real users talking to Mika in 4-6 weeks
- Validates the product thesis (conversation -> insight -> action) before investing in web UI
- Aligns with the MVP spec philosophy: "The hook is the conversation. Integrations make her indispensable."
- Solo dev (25 years experience, strong Python/AI background) — sequencing beats parallelism for one person

---

## Key Decisions

### Tech Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| **Language** | Python | Strongest language for the developer. Best AI/ML ecosystem. Fast to prototype. |
| **Agent Orchestration** | LangGraph | Developer has experience with it. State machines for the listen-reason-act loop. Tool calling, memory checkpointing built in. |
| **Memory Store** | Neo4j | Knowledge graph for entities (people, tasks, preferences) and relationships. Rich Cypher queries. Good LLM integration libraries. |
| **User/Auth DB** | PostgreSQL | Standard relational DB for users, auth, billing, session management. |
| **Background Tasks** | Celery + Redis | Celery Beat for periodic tasks (morning briefings, follow-up scans). Workers for async operations (research, drafting). Battle-tested. |
| **LLM** | Claude (Sonnet + Opus) | Sonnet for ~80% of requests (conversation, simple tasks). Opus for complex reasoning (~20%). No multi-model for MVP. |
| **Messaging** | Telegram (Phase 1) + WhatsApp (Phase 3) | Channel-agnostic message router. Telegram first (zero cost, no restrictions). WhatsApp added in Phase 3 via Cloud API (24h messaging window constraint, per-message cost). Start Meta business verification in Week 1-2. |
| **Hosting** | Railway | Managed Postgres, easy Docker services (Neo4j, Redis). Simple deploys. Good for solo dev. |
| **Web Framework** | TBD (Phase 2) | Django (batteries-included) or FastAPI (lightweight) — decide when dashboard work begins. |

### Architecture Pattern

- **Agent-First build sequence** — core agent loop before web dashboard
- **Hybrid tenant isolation** — shared API server and Neo4j instance, user-scoped subgraphs with strict query-level enforcement (`user_id` on all graph operations)
- **Claude only** — no multi-model routing for MVP. Simpler to build, margins work per business case
- **One brain per user** — isolated knowledge graph subgraph, isolated conversation history, isolated memory
- **Channel-agnostic message router** — abstract messaging behind a common interface so Telegram and WhatsApp (and future channels) plug in without touching agent logic

### Build Sequence

| Phase | Weeks | Focus | Deliverable |
|-------|-------|-------|-------------|
| 1 | 1-2 | Core agent skeleton | Telegram bot + LangGraph agent + basic Neo4j memory extraction. Can have a conversation and remember facts. |
| 2 | 3-4 | Proactive intelligence | Celery scheduler, follow-ups, commitment tracking, onboarding conversation flow. Mika spots stuck tasks and acts. |
| 3 | 5-8 | Web dashboard + WhatsApp | Signup, settings, memory viewer, conversation history. WhatsApp Cloud API integration. (Billing deferred to Month 4+.) |
| 4 | 9-12 | Integrations + polish | Google Calendar, email prep, beta launch readiness. |

---

## Technical Architecture Diagram

```
                    ┌──────────────────────────────────────────────┐
                    │                 Railway                       │
                    │                                              │
┌──────────┐       │  ┌─────────────┐     ┌────────────────────┐  │
│ Telegram │◄──┐   │  │  API Server │────►│  LangGraph Agent   │  │
│  User    │   ├──►│  │  + Message  │     │  (per-request)     │  │
└──────────┘   │   │  │   Router    │     │                    │  │
┌──────────┐   │   │  │  (FastAPI/  │     │                    │  │
│ WhatsApp │◄──┘   │  │   Django)   │     │                    │  │
│  User    │       │  │             │     │                    │  │
└──────────┘       │  │             │     │                    │  │
                    │  └──────┬──────┘     │  ┌──────────────┐ │  │
┌──────────┐       │         │             │  │ Listen Node  │ │  │
│ Web App  │◄─────►│         │             │  │ Reason Node  │ │  │
│ (Browser)│       │         │             │  │ Act Node     │ │  │
└──────────┘       │         │             │  │ Respond Node │ │  │
                    │         │             │  └──────────────┘ │  │
                    │         │             └────────┬───────────┘  │
                    │         │                      │              │
                    │  ┌──────▼──────┐    ┌──────────▼───────────┐ │
                    │  │  PostgreSQL │    │      Neo4j           │ │
                    │  │  (Users,    │    │  (Knowledge Graph)   │ │
                    │  │   Auth,     │    │  user-scoped         │ │
                    │  │   Sessions) │    │  subgraphs           │ │
                    │  └─────────────┘    └──────────────────────┘ │
                    │                                              │
                    │  ┌─────────────┐    ┌──────────────────────┐ │
                    │  │   Redis     │    │   Celery Workers     │ │
                    │  │  (Broker)   │◄──►│  + Celery Beat       │ │
                    │  └─────────────┘    │  (Follow-ups,        │ │
                    │                     │   Briefings, Cron)   │ │
                    │                     └──────────────────────┘ │
                    │                                              │
                    │  ┌──────────────────────────────────────┐    │
                    │  │         Tool Layer                    │    │
                    │  │  ├── Web Search (Tavily/SerpAPI)     │    │
                    │  │  ├── Document Drafting (Claude)      │    │
                    │  │  ├── Research (web scraping)         │    │
                    │  │  ├── Calendar API (Google OAuth)     │    │
                    │  │  └── Gmail API (post-CASA)           │    │
                    │  └──────────────────────────────────────┘    │
                    └──────────────────────────────────────────────┘
```

---

## Memory Architecture (Knowledge Graph)

### Node Types

| Node Label | Properties | Example |
|-----------|------------|---------|
| `User` | id, name, role, company, timezone, preferences | The user themselves |
| `Person` | name, relationship, context, sentiment | "Sarah — head of partnerships, responsive, likes direct comms" |
| `Commitment` | description, due_date, status, priority, source_message | "Send proposal to Sarah by Friday" |
| `Topic` | name, importance, recency | "Q2 hiring plan" |
| `Pattern` | type, description, confidence | "Procrastinates on finance-related tasks" |
| `Preference` | category, value, confidence | "Prefers bullet points over prose in drafts" |
| `Fact` | key, value, source, timestamp | "Team size: 12 people" |

### Relationship Types

| Relationship | From | To | Example |
|-------------|------|----|---------|
| `KNOWS` | User | Person | User knows Sarah |
| `COMMITTED_TO` | User | Commitment | User committed to sending proposal |
| `INVOLVES` | Commitment | Person | Proposal involves Sarah |
| `INTERESTED_IN` | User | Topic | User is tracking Q2 hiring |
| `EXHIBITS` | User | Pattern | User procrastinates on finance |
| `PREFERS` | User | Preference | User prefers bullet points |

### Memory Extraction Pipeline

Memory extraction runs as a **background Celery task** after the agent responds, not inline with the response:

1. Agent responds to the user via Telegram
2. Response + conversation context is queued as a Celery task
3. Celery worker calls Claude to identify entities, relationships, commitments, and preferences
4. Structured extraction maps to Neo4j nodes and relationships via Cypher MERGE queries (update-or-create, no duplicates)
5. Confidence scores decay over time; recent interactions boost relevance

---

## LangGraph Agent Design

### State

```python
class MikaState(TypedDict):
    user_id: str
    messages: list[BaseMessage]
    memory_context: str          # Retrieved from Neo4j before reasoning
    pending_actions: list         # Actions Mika decided to take
    personality_context: str      # Tone/style guidance
```

### Graph Nodes

1. **retrieve_memory** — Query Neo4j for relevant context (recent commitments, related people, patterns)
2. **listen_and_reason** — Process user message with full context. Identify intent, stuck tasks, opportunities to act.
3. **decide_action** — Should Mika act proactively? Draft something? Research? Follow up? Or just respond?
4. **execute_action** — Use tools (web search, drafting, calendar) to complete the action.
5. **respond** — Compose Mika's response with the right personality (warm, competent, slightly opinionated).

*Note: Memory extraction (entity/fact/commitment extraction -> Neo4j writes) happens as a background Celery task triggered after the response is sent. Not part of the LangGraph request flow.*

### Conditional Edges

- After `listen_and_reason`: if action needed -> `decide_action`; else -> `respond`
- After `decide_action`: if tool needed -> `execute_action`; else -> `respond`
- After `respond`: queue memory extraction as a Celery background task

---

## Open Questions

1. **Web framework for dashboard phase?** Django gives you admin, auth, ORM, templates out of the box — fastest for a solo dev building a full dashboard. FastAPI is lighter and pairs well with modern frontend (HTMX/React). Both work with the existing Python stack. Decide at Week 5 (start of Phase 3).

## Assumptions to Validate

- **Telegram Bot API rate limits:** Telegram allows ~30 messages/second to different users, but bot-initiated messages (proactive follow-ups, morning briefings) are limited. Validate this won't bottleneck proactive features at scale.
- **Claude API response latency:** The agent makes at least one Claude call per message (reasoning). Expected latency: 2-5 seconds. Is this acceptable for a Telegram chat UX? Users may expect faster responses.
- **Neo4j data volume:** At 20-40 messages/day/user, each producing several nodes/relationships, a user's subgraph could grow to thousands of nodes within weeks. Cypher query performance at that scale on a single self-hosted instance needs testing.
- **Phase 3 scope risk:** Phase 3 now includes web dashboard AND WhatsApp integration. 4 weeks may be tight for signup + settings + memory viewer + conversation history + WhatsApp adapter as a solo dev. May need to cut scope or extend timeline.
- **WhatsApp 24-hour window:** Proactive messages (morning briefings, follow-ups) require pre-approved message templates outside the 24h window. Need to design template messages that still feel personal, or find ways to keep the conversation window open.

---

## Resolved Decisions

| Decision | Choice | Notes |
|----------|--------|-------|
| **Stack** | Python + LangGraph + Neo4j + Postgres + Celery/Redis + Claude + Telegram + Railway | All familiar tech, strong ecosystem fit |
| **Build approach** | Agent-first | Core loop before web dashboard |
| **Tenant model** | Hybrid (shared infra, user-scoped subgraphs) | `user_id` enforced on all graph queries |
| **LLM strategy** | Claude only (Sonnet + Opus) | No multi-model for MVP |
| **Team** | Solo developer, full-stack ownership | 25 years experience, strong Python/AI background |
| **Neo4j hosting** | Self-hosted Docker on Railway | Migrate to Aura when revenue supports ~$65/month |
| **Memory extraction** | Background Celery task | Non-blocking to response, memory updates lag by seconds |
| **Privacy** | Privacy-first engineering from day one | Encrypt at rest, TLS, audit logging, data export/deletion. Transparent about Claude API processing. |
| **Billing** | Deferred to Month 4+ | Free beta for first 100 users |
| **Telegram transport** | Long polling for dev, webhooks for production | Railway provides a stable URL for webhooks |
| **WhatsApp** | Phase 3 (Weeks 5-8) via Cloud API | Start Meta business verification in Week 1-2. Channel-agnostic message router from Day 0. |
| **Personality** | Versioned prompt template, iterate with beta users | "Warm, competent, slightly opinionated — not a servant" |