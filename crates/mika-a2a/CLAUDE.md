# mika-a2a — Agent-to-Agent Protocol

A2A (Agent-to-Agent) protocol v0.3 implementation for inter-agent communication.

## Architecture

- **Protocol:** JSON-RPC types, task state machine, SSE streaming, A2A client
- **State machine:** submitted -> working -> completed/failed/canceled. Validates transitions.
- **SSE streaming** for `message/send` responses
- **`a2a_call` builtin tool** (120s timeout) sends messages to remote A2A agents

## Client transport policy (mika#2036)

`A2aClient` carries an explicit budget: `DEFAULT_TIMEOUT` = **300 s**, a measured
value (the longest generation ever delivered took 114 s; 300 s is a 2.6x margin),
not `reqwest`'s default of *no timeout at all*. `A2aClient::new` keeps its
signature and gains it; `with_timeout` overrides it, and `RECOVERY_TIMEOUT`
(30 s) bounds a recovery read. `timeout()` reports what the client actually
enforces, so an error can name the interval it really spent.

`TransportFailure` (in `error`) classifies a `reqwest` failure into unreachable /
timed out / HTTP status / unreadable / interrupted. Its load-bearing question is
`request_was_sent()`: only an **unreachable** server proves no work exists. Every
other failure happened after the bytes left, so an answer may be sitting on the
other side.

**Recovering a lost answer.** `message/send` persists the finished task *before*
serializing the HTTP response, so a response lost in transit names work that is
already on disk. The task id cannot help — the server mints it with
`Uuid::new_v4` and it travels back only in the envelope that was lost. The
caller-supplied `context_id` can: it is persisted on `a2a_task_map`. So
`tasks/get` accepts a `context_id` as its `id`, resolved **after** the task-id
lookup misses — the order means a real task id is never shadowed. `get_task`
returns `Ok(None)` for `TASK_NOT_FOUND`, which is an answer ("nothing was created
under that name"), not a failure.

## A2A Persistence (Orthogonal)

A2A tasks use `trigger_type='a2a'` in the `tasks` table with orthogonal persistence:
- `a2a_task_map` maps A2A task IDs to internal task/session IDs
- `a2a_artifacts` stores A2A-specific artifact data
- `a2a_push_notification_configs` stores push notification configurations

## Server Integration

- **mika-spirit** exposes `/a2a/{agent_name}` (JSON-RPC POST, 2MB limit) and `/a2a/{agent_name}/agent.json` (Agent Card GET), both internal-token auth. See `crates/mika-agent/CLAUDE.md` for server endpoint details.
- **mika-gateway** proxies at `/a2a/{customer_id}/{agent_name}` with API key auth (SHA-256 hashed keys in Postgres `a2a_api_keys` table, migration 003). See `crates/mika-gateway/CLAUDE.md` for gateway details.

## Convention

The A2A Protocol convention documented in root CLAUDE.md: `mika-spirit` exposes endpoints, gateway proxies with API key auth. State machine validates transitions. SSE streaming for `message/send`.
