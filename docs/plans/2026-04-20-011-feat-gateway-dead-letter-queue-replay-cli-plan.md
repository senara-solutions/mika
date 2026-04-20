---
title: "feat: Gateway dead-letter queue + replay CLI for exhausted webhook retries"
type: feat
status: active
date: 2026-04-20
issue: 590
---

# Gateway Dead-Letter Queue + Replay CLI

## Overview

Add a dead-letter queue (DLQ) to the gateway for GitHub webhook events that exhaust their retry budget or are abandoned due to semaphore pressure. A background worker periodically retries pending entries, and new gateway HTTP endpoints + CLI commands allow operators to inspect and manually replay dead entries.

## Problem Frame

Issue #589 added retry-with-backoff to the gateway's inbound webhook delivery. This covers transient failures (short agent downtime, network blips). It does **not** cover pathological cases: agent containers down for 10+ minutes, gateway restart while retries are in-flight, or sustained backpressure exhausting the retry budget. When retries are exhausted, the event is logged as an ERROR and permanently lost. There is no recovery path.

This feature adds persistence for failed deliveries, automatic background retry, and operator tooling for manual intervention.

## Requirements Trace

- R1. Exhausted retries persist the event in a Postgres `webhook_deliveries` table with `status='pending'`
- R2. Background worker retries pending entries every 30s with exponential backoff per entry
- R3. After 10 worker attempts, entries transition to `status='dead'`
- R4. `mika webhook list-dead` shows DLQ entries (dead + pending)
- R5. `mika webhook replay <delivery_id>` re-delivers a single dead entry
- R6. `mika webhook replay-all` re-delivers all dead entries
- R7. Entries survive gateway restart (Postgres-backed)
- R8. Semaphore-abandoned events are also captured in the DLQ

## Scope Boundaries

- Scope is webhook delivery to agent containers only — not outbound, not Telegram
- One table, one background task, one CLI subcommand group
- No web UI, no metrics endpoint, no dashboard integration
- No generic "reliable delivery framework" abstraction

### Deferred to Separate Tasks

- DLQ for non-webhook channels (Telegram, future channels): separate ticket if needed
- Web UI for DLQ inspection: separate ticket
- DLQ metrics/alerting endpoint: separate ticket
- Cross-gateway HA / replication: out of scope

## Context & Research

### Relevant Code and Patterns

- `crates/mika-gateway/src/github.rs` — `deliver_with_retry_inner()` (lines 771-893) is the retry loop; `forward_to_resolved_route()` (lines 650-714) handles single delivery attempts; `resolve_github_container_url()` (lines 581-644) does Postgres route lookup
- `crates/mika-gateway/src/routes.rs` — `AppState` struct with `pool: PgPool`, `webhook_semaphore`, `http_client`, `internal_token`
- `crates/mika-gateway/src/main.rs` — Server startup, state construction, `shutdown_signal()`
- `crates/mika-gateway/migrations/` — 5 existing Postgres migrations
- `crates/mika-cli/src/cli.rs` — `Commands` enum with clap subcommands
- `crates/mika-cli/src/commands/mod.rs` — 18 existing command modules
- CLI commands that talk to remote services use `MIKA_SERVER_URL` (dashboard) or direct HTTP calls

### Institutional Learnings

- Gateway uses Postgres (not SQLite) — the issue template mentioned SQLite but the gateway's actual storage is Postgres
- The `failed_sends` table in the agent container DB is a precedent for persisting delivery failures — this is the symmetric pattern on the gateway side
- Route resolution is cached per-event in the retry loop — for DLQ replays, route must be re-resolved since container URLs may have changed

## Key Technical Decisions

- **CLI talks to gateway via HTTP, not direct Postgres access**: The CLI runs on user machines; the gateway runs in K8s. New gateway REST endpoints (`GET /webhook/dlq`, `POST /webhook/dlq/{id}/replay`, `POST /webhook/dlq/replay-all`) expose DLQ operations. CLI uses `MIKA_GATEWAY_URL` (new env var) to reach the gateway. This follows the same pattern as the dashboard CLI using `MIKA_SERVER_URL` for the agent server.
- **Store formatted `text` + metadata, not raw GitHub body**: The DLQ stores the already-formatted message text, target agent, request ID, and repo name — the same values passed to `deliver_with_retry()`. This avoids re-parsing the raw GitHub webhook body on replay and keeps the storage compact. The raw body could be 256KB; the formatted text is typically <1KB.
- **Replay re-resolves routes**: When replaying, the worker/CLI re-resolves the container URL via `resolve_github_container_url()` rather than caching the original URL. This handles the case where containers moved or were recreated.
- **Background worker uses the same `forward_to_resolved_route()` path**: The worker calls the same forwarding function as the original retry loop, ensuring consistent behavior and semaphore respect.
- **DLQ endpoints use internal token auth**: Same Bearer token auth as the `/send` endpoint, since DLQ operations are administrative.

## Open Questions

### Resolved During Planning

- **SQLite vs Postgres?** Postgres — the gateway already has a `PgPool` and all state is in Postgres. The issue mentioned SQLite but was written before the gateway's actual storage was finalized.
- **How does the CLI access the DLQ?** Via HTTP endpoints on the gateway, not direct DB access. New `MIKA_GATEWAY_URL` env var.
- **What payload to store?** The formatted text + metadata (target_agent, request_id, repo_full_name, event_type), not the raw GitHub body.

### Deferred to Implementation

- Exact exponential backoff formula for the background worker (likely `min(30s * 2^attempts, 1h)`)
- Whether `replay-all` should have a batch size limit or rate limiting

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
                    GitHub Webhook
                         │
                    ┌────▼────┐
                    │ handler  │── returns 200 immediately
                    └────┬────┘
                         │ tokio::spawn
                    ┌────▼──────────┐
                    │ deliver_with  │
                    │ _retry_inner  │
                    └───┬───────┬───┘
                   ok   │       │ exhausted / abandoned
                        │  ┌────▼──────────────┐
                        │  │ INSERT INTO        │
                        │  │ webhook_deliveries │
                        │  │ status='pending'   │
                        │  └────────────────────┘
                        │           │
                        │  ┌────────▼───────────┐
                        │  │ Background Worker   │ ← wakes every 30s
                        │  │ SELECT pending rows │
                        │  │ forward each        │
                        │  │ success → delivered  │
                        │  │ fail → attempts++    │
                        │  │ attempts≥10 → dead   │
                        │  └────────────────────┘
                        │           │
                    ┌───▼───────────▼──┐
                    │  CLI (via HTTP)   │
                    │  list-dead        │
                    │  replay <id>      │
                    │  replay-all       │
                    └──────────────────┘
```

## Implementation Units

- [ ] **Unit 1: Postgres migration + DLQ data types**

**Goal:** Create the `webhook_deliveries` table and Rust types for DLQ rows.

**Requirements:** R1, R7

**Dependencies:** None

**Files:**
- Create: `crates/mika-gateway/migrations/006_webhook_deliveries.sql`
- Create: `crates/mika-gateway/src/dlq.rs`
- Modify: `crates/mika-gateway/src/main.rs` (add `mod dlq`)

**Approach:**
- Postgres table with columns: `delivery_id TEXT PRIMARY KEY`, `event_type TEXT NOT NULL`, `target_agent TEXT NOT NULL`, `repo_full_name TEXT`, `payload TEXT NOT NULL` (formatted text), `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`, `attempts INTEGER NOT NULL DEFAULT 0`, `last_attempt_at TIMESTAMPTZ`, `status TEXT NOT NULL DEFAULT 'pending'` (CHECK IN 'pending','delivered','dead'), `last_error TEXT`, `request_id TEXT NOT NULL`
- Rust struct `WebhookDelivery` with `sqlx::FromRow` derive
- Status enum type with string serialization
- Index on `(status, last_attempt_at)` for worker queries

**Patterns to follow:**
- Existing migrations in `crates/mika-gateway/migrations/` (plain SQL, numbered)
- `build.rs` already has `cargo::rerun-if-changed=migrations`

**Test scenarios:**
- Happy path: Migration runs cleanly on an empty database
- Edge case: Inserting a delivery with all fields populated, verify round-trip via SELECT

**Verification:**
- `sqlx::migrate!()` compiles without errors
- Rust types match the table schema

---

- [ ] **Unit 2: DLQ write path — capture exhausted/abandoned deliveries**

**Goal:** When `deliver_with_retry_inner()` exhausts retries or abandons due to semaphore pressure, insert a row into `webhook_deliveries`.

**Requirements:** R1, R8

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-gateway/src/github.rs`
- Modify: `crates/mika-gateway/src/dlq.rs`

**Approach:**
- Add `pub async fn insert_delivery(pool, delivery)` function in `dlq.rs`
- At the two terminal failure points in `deliver_with_retry_inner()` (retry budget exhausted, semaphore capacity), call `insert_delivery()` with the event metadata
- The `delivery_id` comes from the `request_id` parameter (which is the `X-GitHub-Delivery` UUID or a generated UUID)
- Fire-and-forget insert (log error if DB write fails, don't block the caller)
- Pass `pool` through to `deliver_with_retry` — `AppState` already carries `pool`

**Patterns to follow:**
- Existing error logging patterns at the two terminal points in `deliver_with_retry_inner()`
- The function already has access to `state: &AppState` which contains `pool`

**Test scenarios:**
- Happy path: Mock a delivery that exhausts all retries, verify row appears in `webhook_deliveries` with `status='pending'` and `attempts` matching retry count
- Happy path: Mock a delivery abandoned due to semaphore pressure, verify row appears with `status='pending'`
- Error path: If the DB insert fails, the ERROR log still fires and the function returns normally (no panic)
- Edge case: Permanent failure (4xx) does NOT insert into DLQ — only retryable exhaustion and semaphore abandonment

**Verification:**
- The two ERROR log lines in `deliver_with_retry_inner()` are now preceded by DLQ inserts
- Existing retry tests still pass

---

- [ ] **Unit 3: Background DLQ worker**

**Goal:** A tokio task that periodically retries pending DLQ entries and transitions them to `delivered` or `dead`.

**Requirements:** R2, R3, R7

**Dependencies:** Unit 1, Unit 2

**Files:**
- Modify: `crates/mika-gateway/src/dlq.rs`
- Modify: `crates/mika-gateway/src/main.rs`

**Approach:**
- `pub async fn run_dlq_worker(state: AppState)` — infinite loop with `tokio::time::sleep(Duration::from_secs(30))`
- Each tick: `SELECT * FROM webhook_deliveries WHERE status = 'pending' AND (last_attempt_at IS NULL OR last_attempt_at < now() - interval * backoff(attempts)) ORDER BY created_at ASC LIMIT 50`
- For each entry: re-resolve route via `resolve_github_container_url()`, call `forward_to_resolved_route()`, update row based on result
- Success → `status = 'delivered'`
- Retryable/Permanent failure → `attempts += 1`, `last_attempt_at = now()`, `last_error = reason`
- `attempts >= 10` → `status = 'dead'`
- Respect the webhook semaphore (acquire permit before forwarding, release after)
- Spawn in `main.rs` before `axum::serve()`: `tokio::spawn(dlq::run_dlq_worker(state.clone()))`
- Worker logs at INFO level on transitions and WARN on failures

**Patterns to follow:**
- The periodic cleanup in `telegram.rs` (counter-based, different cadence but same spawn pattern)
- `forward_to_resolved_route()` and `resolve_github_container_url()` from `github.rs`

**Test scenarios:**
- Happy path: Insert a pending delivery, run one worker tick, verify it gets forwarded and transitions to `delivered`
- Happy path: Insert a pending delivery with `attempts=9`, fail the forward, verify it transitions to `dead` after the 10th attempt
- Edge case: No pending deliveries — worker tick completes quickly without errors
- Edge case: Backoff respected — entry with `last_attempt_at` within its backoff window is skipped
- Error path: Forward returns `Permanent` — still increments attempts and records error (permanent is a worker failure, not a routing error; the initial routing already resolved)
- Integration: Worker respects semaphore — if all 30 permits are held, worker skips forwarding for that tick

**Verification:**
- Worker starts with the gateway and survives across ticks
- Pending entries are retried, delivered entries are not retried, dead entries are not retried

---

- [ ] **Unit 4: Gateway HTTP endpoints for DLQ operations**

**Goal:** REST endpoints on the gateway for listing, replaying single entries, and replaying all dead entries.

**Requirements:** R4, R5, R6

**Dependencies:** Unit 1, Unit 3

**Files:**
- Modify: `crates/mika-gateway/src/dlq.rs`
- Modify: `crates/mika-gateway/src/routes.rs`

**Approach:**
- `GET /webhook/dlq` — returns JSON array of deliveries with `status IN ('pending', 'dead')`. Optional query params: `?status=dead&limit=50`
- `POST /webhook/dlq/{delivery_id}/replay` — re-resolves route, forwards, updates status. Returns the delivery row with updated status.
- `POST /webhook/dlq/replay-all` — selects all `status='dead'` rows, replays each, returns summary (count succeeded, count failed)
- All endpoints use the same internal token Bearer auth as `/send`
- Register routes in `build_router()` under the internal auth middleware

**Patterns to follow:**
- Existing route registration in `routes.rs` `build_router()`
- Internal token auth middleware already in place for `/send`
- `utoipa` annotations for OpenAPI docs (optional, other webhook endpoints skip it)

**Test scenarios:**
- Happy path: `GET /webhook/dlq` returns empty array when no deliveries exist
- Happy path: `GET /webhook/dlq` returns deliveries filtered by status
- Happy path: `POST /webhook/dlq/{id}/replay` with valid ID re-delivers and returns updated row
- Error path: `POST /webhook/dlq/{id}/replay` with non-existent ID returns 404
- Error path: Unauthenticated request returns 401
- Happy path: `POST /webhook/dlq/replay-all` replays dead entries and returns summary
- Edge case: `replay-all` with no dead entries returns success with count=0

**Verification:**
- Endpoints accessible via curl with correct auth
- Status transitions reflected in DB after replay

---

- [ ] **Unit 5: CLI `mika webhook` subcommand**

**Goal:** CLI commands that call the gateway HTTP endpoints to list and replay DLQ entries.

**Requirements:** R4, R5, R6

**Dependencies:** Unit 4

**Files:**
- Create: `crates/mika-cli/src/commands/webhook.rs`
- Modify: `crates/mika-cli/src/commands/mod.rs`
- Modify: `crates/mika-cli/src/cli.rs`

**Approach:**
- New `Webhook` variant in `Commands` enum with subcommands: `ListDead`, `Replay { delivery_id: String }`, `ReplayAll`
- `MIKA_GATEWAY_URL` env var (default: `http://localhost:3001`) for gateway base URL
- `MIKA_INTERNAL_TOKEN` env var for Bearer auth (same token the gateway uses)
- `list-dead` prints a table: delivery_id, event_type, target_agent, attempts, created_at, last_error (truncated)
- `replay` and `replay-all` print results and exit codes
- Support `--format text|json` like other CLI commands

**Patterns to follow:**
- `crates/mika-cli/src/commands/dashboard.rs` — uses `MIKA_SERVER_URL` for remote API calls
- `crates/mika-cli/src/commands/tasks.rs` — table output formatting
- Clap subcommand registration pattern in `cli.rs`

**Test scenarios:**
- Happy path: `mika webhook list-dead` with gateway running shows table of dead entries
- Happy path: `mika webhook replay <id>` with valid ID prints success message
- Error path: Gateway unreachable prints connection error with helpful message
- Error path: Missing `MIKA_GATEWAY_URL` uses default and warns if connection fails
- Happy path: `--format json` outputs JSON array/object

**Verification:**
- `mika webhook --help` shows subcommands
- Commands execute against a running gateway and display results

---

- [ ] **Unit 6: Tests and documentation**

**Goal:** Integration tests for the DLQ write path and worker, plus documentation updates.

**Requirements:** R1-R8

**Dependencies:** Units 1-5

**Files:**
- Modify: `crates/mika-gateway/src/github.rs` (add DLQ-related test cases to existing test module)
- Modify: `crates/mika-gateway/src/dlq.rs` (unit tests for worker logic)
- Modify: `docs/deployment.md` or relevant docs (mention DLQ, `MIKA_GATEWAY_URL`)
- Modify: `crates/mika-gateway/CLAUDE.md` (document DLQ architecture)
- Modify: `crates/mika-cli/CLAUDE.md` (document `webhook` subcommand)

**Approach:**
- Test the DLQ insert path by extending existing `deliver_with_retry_inner` tests to verify DB writes
- Test the worker tick function in isolation with a test Postgres instance (if available) or with mock queries
- Document the new `MIKA_GATEWAY_URL` env var in gateway docs
- Add DLQ section to gateway CLAUDE.md

**Test scenarios:**
- Integration: Full flow — deliver_with_retry exhausts budget → row in DB → worker picks up → forward succeeds → status='delivered'
- Integration: Worker respects backoff — entry just attempted is not re-attempted immediately
- Edge case: Multiple pending entries processed in created_at order
- Edge case: Worker handles DB connection failure gracefully (logs, continues next tick)

**Verification:**
- `cargo test -p mika-gateway` passes with new tests
- CLAUDE.md files updated with DLQ documentation

## System-Wide Impact

- **Interaction graph:** `handle_github_webhook()` → `deliver_with_retry_inner()` → `dlq::insert_delivery()` (new write path). `dlq::run_dlq_worker()` → `github::resolve_github_container_url()` + `github::forward_to_resolved_route()` (reuse existing functions). Gateway HTTP endpoints → `dlq` module functions.
- **Error propagation:** DLQ insert failures are logged but never propagate to the webhook handler (fire-and-forget). Worker failures are logged per-entry and don't halt the worker loop.
- **State lifecycle risks:** Duplicate `delivery_id` entries — handled by PRIMARY KEY constraint (UPSERT or conflict-ignore). Gateway restart mid-worker-tick — pending entries remain in DB, picked up on next tick after restart.
- **API surface parity:** New gateway endpoints need internal token auth consistent with `/send`. CLI commands need `MIKA_GATEWAY_URL` — a new env var.
- **Unchanged invariants:** The retry loop's behavior for successful deliveries and permanent failures is unchanged. The LRU dedup cache is unchanged. Semaphore backpressure capacity (30 permits) is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Worker contends with webhook handler for semaphore permits | Worker uses `try_acquire_owned` and skips entries if semaphore is full — same pattern as retry loop |
| `replay-all` floods agent with many events at once | Process sequentially with semaphore gating; log progress |
| DLQ table grows unbounded | Add a CRON-like cleanup for delivered entries older than 30 days (deferred — manual cleanup via SQL is acceptable initially) |
| `forward_to_resolved_route` and `resolve_github_container_url` are currently `pub(crate)` — may need visibility changes for `dlq.rs` | Both are in the same crate, `pub(crate)` is sufficient |

## Documentation / Operational Notes

- New env var: `MIKA_GATEWAY_URL` — base URL for CLI commands that talk to the gateway (default: `http://localhost:3001`)
- CLI requires `MIKA_INTERNAL_TOKEN` to authenticate with gateway DLQ endpoints
- Gateway CLAUDE.md needs DLQ section documenting the background worker and endpoints
- CLI CLAUDE.md needs `webhook` subcommand documentation

## Sources & References

- Related issue: #590 (this feature)
- Depends on: #589 (retry with backoff — already merged)
- Related: #583 / PR #586 (engine-side dispatch guards)
- Precedent: `failed_sends` table in agent container DB (`docs/deployment.md`)
- Code: `crates/mika-gateway/src/github.rs` — `deliver_with_retry_inner()`, `forward_to_resolved_route()`, `resolve_github_container_url()`
