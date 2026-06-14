# A2A Protocol Implementation for Mika

**Date:** 2026-03-19
**Status:** Brainstorm complete
**Spec version:** A2A v0.3 (https://a2a-protocol.org/latest/)

## What We're Building

Full A2A (Agent-to-Agent) protocol support for the Mika platform, enabling Mika agents to both **serve** and **consume** A2A interactions:

- **Server role:** External A2A clients can discover Mika agents via Agent Cards, send messages, manage tasks, and receive streaming updates — all through the gateway as a thin proxy to per-customer agent containers.
- **Client role:** Mika agents can call out to external A2A agents during their tool loop via a built-in `a2a_call` tool, enabling task delegation and multi-agent collaboration.
- **Gateway proxy:** The existing mika-gateway gains a new A2A proxy path — authenticates clients (API key), resolves the target customer/agent, and forwards JSON-RPC requests to the agent container unchanged.

### Scope

- JSON-RPC 2.0 binding (single POST endpoint with method dispatch)
- Synchronous + SSE streaming + push notifications (full spec compliance)
- Agent Card discovery at `/.well-known/agent-card.json` (served by agent containers, proxied by gateway)
- API key authentication initially; OAuth2 declared in Agent Card for future implementation
- New `mika-a2a` crate for shared types, JSON-RPC dispatch, task state machine, and client

### What This Is NOT

- No gRPC or HTTP/REST bindings (JSON-RPC only for now)
- No OAuth2 implementation yet (declared but not enforced)
- No multi-tenant Agent Card customization (extended agent card deferred)
- No agent-to-agent communication *within* a single Mika container (internal agents already collaborate via team runs)

## Why This Approach

### JSON-RPC 2.0 as the sole binding

- Canonical A2A binding — most reference implementations and SDKs target it
- Single endpoint simplifies gateway proxying (one route to forward)
- Method dispatch maps cleanly to Rust enums
- Can add REST/gRPC bindings later as separate Axum route groups

### Thin proxy at the gateway

- Keeps A2A logic (task state machine, agent card, streaming) in the agent container where the agent state lives
- Gateway only needs to authenticate, resolve customer, and forward — no new state or storage for A2A
- Consistent with the existing Telegram proxy pattern (gateway authenticates, routes, forwards)
- Agent containers already have SQLite for task/session storage — A2A tasks fit naturally

### New mika-a2a crate

- Clean separation: A2A protocol types and logic don't pollute mika-agent or mika-gateway
- Both crates depend on mika-a2a: agent uses server + client, gateway uses types for proxying
- Can be extracted as a standalone A2A Rust library later
- Proto definitions from `a2a.proto` can drive type generation via `prost`

### Built-in tool for client role

- The agent loop already dispatches built-in tools — adding `a2a_call` follows the established pattern
- Tools have timeout and error handling built in (TOOL_TIMEOUT=30s, overridable)
- Agent can discover external agents, send messages, poll tasks — all within the existing tool loop constraints
- No skill manifest overhead, no MCP translation layer

### API key auth first, OAuth2 later

- Fits the existing per-customer model (gateway already has a `customers` table in Postgres)
- Add an `a2a_api_key` column or a separate `api_keys` table
- Agent Card declares both `apiKey` and `oauth2` security schemes; only `apiKey` is enforced initially
- OAuth2 requires an auth server decision (self-hosted, Auth0, etc.) — separate concern

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Protocol binding | JSON-RPC 2.0 | Canonical, simple to proxy, one endpoint |
| Direction | Both server + client | Full interoperability from day one |
| Gateway role | Thin proxy | Keeps A2A state with agent state in containers |
| Crate structure | New `mika-a2a` crate | Clean separation, reusable types |
| Client integration | Built-in tool (`a2a_call`) | Follows existing tool pattern in agent loop |
| Auth | API key now, OAuth2 later | Simple, fits per-customer model |
| Streaming | Full (sync + SSE + push) | Complete A2A compliance |
| Task storage | Separate SQLite tables | Clean isolation from internal tasks |
| Agent Card URL | `/a2a/{customer_id}/{agent_name}/agent.json` | Per-agent at gateway level |
| Multi-agent | Per-agent cards and endpoints | Preserves agent identity for A2A clients |
| Push egress | Through gateway | Centralized egress control |
| Skill visibility | Allow-list in agent config | Safe default: nothing exposed |

## Architecture Overview

```
External A2A Client
       │
       ▼
┌─────────────────┐    JSON-RPC     ┌─────────────────────────────┐
│  mika-gateway   │ ───────────────▶│  mika-spirit (per-customer) │
│                 │   (thin proxy)  │                             │
│ - API key auth  │                 │ - A2A JSON-RPC handler      │
│ - Customer      │                 │ - Agent Card endpoint       │
│   resolution    │                 │ - Task state machine        │
│ - Forward       │                 │ - SSE streaming             │
│   JSON-RPC      │                 │ - Push notification client  │
│                 │                 │ - Existing agent loop       │
└─────────────────┘                 └─────────────────────────────┘
                                              │
                                              │ a2a_call tool
                                              ▼
                                    ┌───────────────────┐
                                    │ External A2A Agent │
                                    └───────────────────┘
```

### Crate Dependencies

```
mika-a2a (new)
├── A2A types (AgentCard, Task, Message, Part, Artifact, etc.)
├── JSON-RPC request/response types and dispatch
├── Task state machine (9 states, transition validation)
├── A2A client (for calling external agents)
└── SSE stream types

mika-agent
├── depends on mika-a2a
├── A2A server handlers (Axum routes)
├── Agent Card builder (from agent config)
├── A2A task storage (SQLite)
├── Push notification sender
└── a2a_call built-in tool

mika-gateway
├── depends on mika-a2a (types only)
├── A2A proxy route (/a2a → agent container)
├── API key validation
└── Customer/agent resolution for A2A
```

### Key Components to Build

**mika-a2a crate:**
- `types.rs` — All A2A protocol types (consider generating from `a2a.proto` via prost)
- `jsonrpc.rs` — JSON-RPC 2.0 request/response envelope, method dispatch enum
- `task_state.rs` — Task state machine with transition validation
- `client.rs` — A2A HTTP client (reqwest-based, supports sync + SSE)
- `error.rs` — A2A error codes mapped to JSON-RPC errors

**mika-agent (server role):**
- `server/a2a.rs` — Axum handler for JSON-RPC POST endpoint
- `server/agent_card.rs` — Build AgentCard from agent config + skills
- `a2a_tasks.rs` — SQLite schema and queries for A2A task lifecycle
- `a2a_streaming.rs` — SSE stream management for task updates
- `a2a_push.rs` — Webhook push notification sender

**mika-agent (client role):**
- `tools/a2a_call.rs` — Built-in tool: discover agent, send message, poll/stream result

**mika-gateway:**
- `a2a_proxy.rs` — Proxy route: authenticate API key, resolve customer, forward JSON-RPC
- Migration: add `a2a_api_keys` table to Postgres

### A2A Task Mapping

A2A tasks map to the existing Mika task/session model:

| A2A Concept | Mika Equivalent | Notes |
|-------------|-----------------|-------|
| Task | New A2A-specific table | Separate from internal tasks — different lifecycle |
| context_id | Session | Groups related A2A messages into a conversation |
| Message (role=user) | Incoming message | Triggers agent loop like Telegram messages |
| Message (role=agent) | Agent response | Sent back via A2A instead of gateway /send |
| Artifact | New concept | Agent outputs (files, data) — store in A2A task table |
| Task states | New state machine | SUBMITTED→WORKING→COMPLETED/FAILED + interrupted states |

### Gateway Proxy Flow

```
1. Client → POST /a2a (JSON-RPC) → Gateway
2. Gateway extracts API key from Authorization header
3. Gateway looks up customer by API key in Postgres
4. Gateway resolves agent container URL: http://mika-{customer_id}:8080
5. Gateway forwards JSON-RPC body to container's /a2a endpoint
6. For SSE methods (message/stream, tasks/resubscribe):
   - Gateway upgrades to SSE passthrough
   - Streams events from container back to client
7. For sync methods: proxy response directly
```

### Agent Card Structure

Each Mika agent publishes an Agent Card derived from its configuration:

```json
{
  "name": "mika-{agent_name}",
  "description": "from agent's soul.md",
  "version": "0.3",
  "provider": {
    "organization": "Senara Solutions",
    "url": "https://senara.solutions"
  },
  "supported_interfaces": [{
    "url": "https://{gateway_domain}/a2a/{customer_id}/{agent_name}",
    "protocol_binding": "jsonrpc/2.0"
  }],
  "capabilities": {
    "streaming": true,
    "push_notifications": true
  },
  "security_schemes": {
    "apiKey": {
      "type": "apiKey",
      "in": "header",
      "name": "Authorization"
    }
  },
  "security_requirements": [{"apiKey": []}],
  "skills": ["derived from agent's skill registry"]
}
```

## Resolved Questions

1. **Agent Card URL routing** → Gateway path: `/a2a/{customer_id}/{agent_name}/agent.json`. Per-agent routing at the gateway level. No well-known path rewriting needed. The `url` field in each Agent Card points to `https://{gateway_domain}/a2a/{customer_id}/{agent_name}`.

2. **A2A task vs internal task isolation** → Separate tables (`a2a_tasks`, `a2a_messages`, `a2a_artifacts`). Clean separation with A2A-specific fields (state machine, artifacts). Internal task engine unaffected.

3. **Push notification egress** → Through the gateway. Agent container POSTs to gateway, gateway forwards to external webhook. Centralizes egress control, enables rate limiting and logging. Consistent with existing outbound message pattern.

4. **Skill exposure granularity** → Allow-list in agent config (`a2a_skills` field). Only explicitly listed skills appear in the Agent Card. Safe default: nothing exposed.

5. **Multi-agent routing** → Per-agent. Each agent gets its own Agent Card and A2A endpoint. URL structure:
   ```
   /a2a/{customer_id}/{agent_name}/agent.json   → Agent Card
   /a2a/{customer_id}/{agent_name}/              → JSON-RPC endpoint
   ```
   Future convenience: `/a2a/{customer_id}/default` alias for the active agent.

## Open Questions

None — all resolved.

## Next Steps

- `/ce:plan` to create implementation plan
- Prioritize: types crate → server handlers → gateway proxy → client tool → streaming → push notifications
