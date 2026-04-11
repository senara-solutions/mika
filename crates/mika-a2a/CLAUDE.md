# mika-a2a — Agent-to-Agent Protocol

A2A (Agent-to-Agent) protocol v0.3 implementation for inter-agent communication.

## Architecture

- **Protocol:** JSON-RPC types, task state machine, SSE streaming, A2A client
- **State machine:** submitted -> working -> completed/failed/canceled. Validates transitions.
- **SSE streaming** for `message/send` responses
- **`a2a_call` builtin tool** (120s timeout) sends messages to remote A2A agents

## A2A Persistence (Orthogonal)

A2A tasks use `trigger_type='a2a'` in the `tasks` table with orthogonal persistence:
- `a2a_task_map` maps A2A task IDs to internal task/session IDs
- `a2a_artifacts` stores A2A-specific artifact data
- `a2a_push_notification_configs` stores push notification configurations

## Server Integration

- **mika-server** exposes `/a2a/{agent_name}` (JSON-RPC POST, 2MB limit) and `/a2a/{agent_name}/agent.json` (Agent Card GET), both internal-token auth. See `crates/mika-agent/CLAUDE.md` for server endpoint details.
- **mika-gateway** proxies at `/a2a/{customer_id}/{agent_name}` with API key auth (SHA-256 hashed keys in Postgres `a2a_api_keys` table, migration 003). See `crates/mika-gateway/CLAUDE.md` for gateway details.

## Convention

The A2A Protocol convention documented in root CLAUDE.md: `mika-server` exposes endpoints, gateway proxies with API key auth. State machine validates transitions. SSE streaming for `message/send`.
