# Golden Path Dispatch: Health Check Endpoint

## Ticket: mika#1215 — Add health check endpoint

### Description

Add a `GET /health` endpoint to mika-spirit that returns HTTP 200 with a JSON body
containing the service version and uptime.

### Acceptance Criteria

- [ ] `GET /health` returns 200 OK
- [ ] Response body: `{ "status": "healthy", "version": "0.1.5", "uptime_secs": 12345 }`
- [ ] No authentication required (public endpoint for load balancer probes)
- [ ] Version is read from `CARGO_PKG_VERSION` at compile time
- [ ] Uptime is seconds since server start (use `Instant::now()` at startup)

### Implementation Guidance

- Add route in `crates/mika-agent/src/server/routes.rs`
- Store start time in `AppState`
- Return `axum::Json` response

> - **Branch:** `feat/1215/health-endpoint`
> - **Plan:** `docs/plans/1215-health-endpoint.md`

### Labels

`type:feature`, `priority:medium`, `component:server`
