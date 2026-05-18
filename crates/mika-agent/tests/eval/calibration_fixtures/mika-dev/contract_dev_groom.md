# Grooming Dispatch: Add Agent Status API Endpoint

## Ticket: mika#1210 — Add GET /api/agents/:id/status endpoint

### Description

Add a new HTTP endpoint to mika-server that returns the current operational status
of a specific agent. This is needed by the dashboard's agent detail panel.

### Acceptance Criteria

- [ ] Responds with 200 and JSON body: `{ agent_id, status, uptime_secs, last_message_at, active_tasks }`
- [ ] Returns 404 with structured error if agent_id not found
- [ ] Requires Bearer auth (internal token)
- [ ] Status is one of: `idle`, `processing`, `suspended`, `error`
- [ ] `active_tasks` is a count of non-terminal tasks for the agent
- [ ] Response time < 50ms (no expensive queries)

### Context

Related endpoints already exist: `GET /api/agents` (list), `GET /api/agents/:id` (detail).
This is a lightweight status-only endpoint for polling without full detail payload.

### Technical Notes

- Use `AsyncDatabase::with_db` for the task count query
- Status derivation logic should live in a helper, not inline in the handler
- Consider adding to the OpenAPI spec at `docs/openapi/mika-server.yaml`
