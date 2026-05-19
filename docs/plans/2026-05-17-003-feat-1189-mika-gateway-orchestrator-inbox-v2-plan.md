---
ticket: mika#1189
type: feat
status: draft
date: 2026-05-17
---

# Plan: mika-gateway orchestrator-inbox v2 (bidirectional channel, real-time path)

## Context

mika-platform#100 (CLOSED) shipped a file-based orchestrator inbox at `~/.mika/orchestrator/inbox/<spawn_id>.json`. Spawned Claude Code tenants write JSON on `/mika-handsoff` Phase 6; orchestrator-Claude reads on its own Phase 0. Batch, single-host, no real-time visibility.

mika#1189 asks for an HTTP/SSE channel on `mika-gateway` to eliminate operator-mediated paste cycles between spawn and orchestrator, and to enable multi-operator coordination (Mac + Linux operators sharing context).

This plan scopes a **conservative first cut** that delivers real-time spawn-to-orchestrator notification, surfaces the architectural questions that must be settled before later phases can land, and explicitly defers the speculative parts of the ticket (bidirectional dispatch, hook-driven surfacing, cross-machine spawn launch).

## Phase 0 — Pin (base SHA + verbatim slices)

**Base SHA:** `72021b78482f1c313156e7630d626865415dede3` (origin/main, fetched 2026-05-17).

Implementer must verify these slices match the worktree HEAD before editing. If any drift, refresh from current HEAD and re-pin.

### Slice 1 — SSE pass-through pattern (the shape `orchestrator_inbox.rs` mirrors)

File: `crates/mika-gateway/src/a2a_routes.rs`, lines 130–142.

```rust
    let status = resp.status();

    if is_streaming && status.is_success() {
        // Stream SSE response through
        let byte_stream = resp.bytes_stream();
        let body = Body::from_stream(byte_stream);
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .header("connection", "keep-alive")
            .body(body)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
```

Note: this is a **pass-through** from an upstream agent container. The orchestrator inbox SSE handler does NOT proxy — it reads rows from Postgres in a loop and emits SSE events directly. The headers and `Response::builder()` shape are what we mirror; the body source differs.

### Slice 2 — Bearer-auth middleware signature

File: `crates/mika-gateway/src/routes.rs`, lines 797–817.

```rust
// -- Bearer auth middleware --

/// Middleware: validates `Authorization: Bearer <token>` using constant-time comparison.
async fn require_bearer_token(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> impl IntoResponse {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match token {
        Some(t) if constant_time_eq(t, state.internal_token.expose_secret()) => {
            next.run(req).await.into_response()
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}
```

The new routes are layered with `require_bearer_token` identically to `/send` (`crates/mika-gateway/src/routes.rs:138-145`):

```rust
        .route(
            "/send",
            post(handle_send)
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    require_bearer_token,
                ))
                .layer(RequestBodyLimitLayer::new(256 * 1024)),
        )
```

### Slice 3 — Migration 006 (`webhook_deliveries`) as the precedent migration 007 follows

File: `crates/mika-gateway/migrations/006_webhook_deliveries.sql`.

```sql
-- Dead-letter queue for GitHub webhook deliveries that exhausted retries.
-- Tracks delivery attempts and allows manual/automatic replay.
CREATE TABLE webhook_deliveries (
    delivery_id     TEXT PRIMARY KEY,
    event_type      TEXT NOT NULL,
    target_agent    TEXT NOT NULL,
    repo_full_name  TEXT,
    payload         TEXT NOT NULL,
    request_id      TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'delivered', 'dead')),
    last_error      TEXT
);

-- Index for background worker: find pending entries eligible for retry.
CREATE INDEX idx_webhook_deliveries_pending
    ON webhook_deliveries (status, last_attempt_at)
    WHERE status = 'pending';

-- Index for CLI: list dead entries.
CREATE INDEX idx_webhook_deliveries_dead
    ON webhook_deliveries (created_at DESC)
    WHERE status = 'dead';
```

Migration 007 follows the same shape: a single `CREATE TABLE` plus partial indexes for the access patterns the SSE handler needs.

### Slice 4 — claude-pilot-py session-end call site

File: `claude-pilot-py/src/claude_pilot/agent.py`, around line 180 in the success-path emit.

```python
                    result = ResultJson(
                        status=status,
                        subtype=subtype,
                        task_id=task_id,
                        session_id=session_id or getattr(message, "session_id", None),
                        turns=message.num_turns,
                        cost_usd=message.total_cost_usd or 0.0,
                        duration_ms=message.duration_ms,
                        errors=errors,
                        termination_reason=termination_reason,
                    )
                    _emit_result(result)

                    if status == "success":
                        log_done(message.num_turns, result.cost_usd, message.duration_ms)
```

The new `inbox_writer.post_handoff(result)` call goes **after** `_emit_result(result)` and gated on `status == "success"` plus the `MIKA_ORCHESTRATOR_INBOX_ENABLED` + `MIKA_ORCHESTRATOR_ID` env vars. Failures in `inbox_writer.post_handoff()` are logged but do NOT change exit code — the inbox write is a side-channel, not the canonical output. Canonical output is `_emit_result(result)` (stdout `ResultJson` per `claude-pilot-py/CLAUDE.md` "Architecture" notes).

## Premise corrections (read first)

Two specifics in the ticket as written disagree with the codebase. Both must be settled before grooming continues.

### 1. Persistence target — `mika.db` is the wrong location

> Ticket §1: "Messages persisted in mika.db (new `orchestrator_inbox_messages` table)"

`mika.db` is the **per-customer agent SQLite** at `~/.mika/data/mika.db` inside an agent container. Each Mika customer gets their own container with their own SQLite (`mika/CLAUDE.md` → Stack → Database; `crates/mika-agent/CLAUDE.md` schema v35). There is no shared `mika.db` on the gateway.

`mika-gateway` uses **Postgres** for its persistent state: `outbound_messages` (002), `a2a_api_keys` (003), `github_repos` (004), `webhook_deliveries` (006) (`crates/mika-gateway/CLAUDE.md` → Postgres Migrations). The orchestrator inbox, if hosted on the gateway, lands in Postgres as migration **007**.

This plan assumes Postgres. If the architect prefers SQLite-on-operator-box instead (see open question Q1), the persistence-layer phase changes shape — see "Alternative path" below.

### 2. Gateway placement — RESOLVED to Option A (gateway)

`mika-gateway` exists to route **customer-facing traffic** (Telegram + GitHub webhooks) into per-customer agent containers, plus the A2A proxy. Today every Postgres table on the gateway is about routing customer messages to customer containers. Orchestrator-spawn coordination is **operator developer-workflow infrastructure** — Vincent (and potentially a Mac contributor) running Claude Code tenants on their own boxes.

Architect (mika-arch session `9d81e315-4ba6-4995-9991-e941866bd3b2`, first pass) ratified **Option A (mika-gateway)** for v1: YAGNI applies; a new `mika-coord` service is premature; messages live durably in Postgres so a gateway pod restart causes a reconnect gap, not data loss. `mika-coord` extraction trigger: "if multi-tenant coordination semantics require row-level isolation or the retention/eviction policies diverge meaningfully."

Options considered (kept for audit):

| Option | Where it lives | Pros | Cons |
|---|---|---|---|
| **A. mika-gateway** ✅ ratified | Existing K8s service, Postgres | One service to operate; multi-operator works for free; reuses gateway auth pattern | Couples dev infra to customer infra; gateway pod restart affects developer workflow (mitigated by durable Postgres + cursor replay) |
| **B. mika-coord (new service)** | New K8s service or local daemon | Clean SoC; can be operator-machine-local | New service to operate; deferred multi-operator until deployed remotely |
| **C. mika-server (agent HTTP server)** of a "personal" agent | Existing per-customer Axum server | Reuses agent infra | Still per-customer scoping; orchestrator is not a customer |

## Orchestrator-id discovery (F3 resolution — option (a'))

Operator (Vincent, 2026-05-17) ratified option (a'): the orchestrator generates its id once at session start, caches it at `~/.mika/orchestrator/id`, and `scripts/mika-platform-spawn` exports `MIKA_ORCHESTRATOR_ID` to every child tenant alongside the existing `MIKA_SPAWN_ID`. Reasoning lives in the architect-retro exchange on session `9d81e315-4ba6-4995-9991-e941866bd3b2`.

Mechanism:

1. **Id generation + cache.** A new helper script `scripts/mika-orchestrator-id` (in `mika-platform`) does:
   ```bash
   mkdir -p ~/.mika/orchestrator
   if [ ! -s ~/.mika/orchestrator/id ]; then
       uuidgen > ~/.mika/orchestrator/id   # uuidgen preferred; fallback printf '%s-%s' $(date -u +%Y%m%dT%H%M%SZ) $$
   fi
   cat ~/.mika/orchestrator/id
   ```
   Idempotent — first invocation writes; subsequent invocations just print. Survives orchestrator restarts, tmux session crashes, and laptop reboots because it's disk-backed.

2. **Spawn-side export.** `scripts/mika-platform-spawn` reads the cached id and exports `MIKA_ORCHESTRATOR_ID="$(scripts/mika-orchestrator-id)"` into the spawned tenant's env, alongside the existing `MIKA_SPAWN_ID` it already exports. Both correlation handles flow to the child via the same script — symmetric with mika-platform#100's spawn-id mechanism.

3. **Doc surface.** `.claude/commands/mika-spawn.md` documents the new env var in the "Related" section.

4. **Consumer side.** `claude-pilot-py/inbox_writer.py` reads `MIKA_ORCHESTRATOR_ID` from env and uses it as the URL path segment for the POST. When unset (operator running outside the spawn-chain), the writer skips silently — same skip-silently semantics as the filesystem-inbox write when `MIKA_SPAWN_ID` is unset.

This locks the file scope for this PR (see "Files to modify" below — three new files added under the F3 resolution).

## Scope (this ticket)

**In:** Persistence schema; HTTP write endpoint (spawn → inbox); SSE read endpoint (orchestrator subscribes); spawn-side writer in claude-pilot-py session end; orchestrator-side polling-only consumer; orchestrator-id generation + spawn-side export (F3 (a') resolution above); feature flag for dual-write with filesystem inbox; smoke-test path.

**Out (deferred to follow-up tickets):**

- Bidirectional dispatch (orchestrator → spawn "iterate on X"). Spawned Claude Code tenants are stateful interactive sessions; the Claude Code Agent SDK does not expose an "inject user-turn" hook for an active conversation today. Verifying this is a separate piece of work.
- Real-time push to orchestrator via Claude Code hooks (Stop / PreToolUse / message-injection). Hook capability is unverified per the ticket itself. Polling-only is the safe first cut.
- Multi-operator coordination protocol (Mac + Linux operators sharing state). The endpoints enable this but the cross-operator coordination semantics (who sees what, conflict resolution) are a separate design.

## Files to modify

### mika-gateway (Rust crate)

- `crates/mika-gateway/migrations/007_orchestrator_inbox_messages.sql` — new migration. Table shape below.
- `crates/mika-gateway/src/routes.rs` — register 2 new routes under `build_router()` (lines 130–187):
  - `POST /orchestrator/inbox/{orchestrator_id}/message` (bearer auth, body limit 64 KB)
  - `GET  /orchestrator/inbox/{orchestrator_id}/stream` (bearer auth, SSE)
- `crates/mika-gateway/src/orchestrator_inbox.rs` — new module. Handlers + persistence ops, mirroring the structure of `a2a_routes.rs`. SSE shape follows `crates/mika-gateway/src/a2a_routes.rs:130-142` (pass-through `text/event-stream`).
- `crates/mika-gateway/CLAUDE.md` — append the two new endpoints to the table and document migration 007. Append `MIKA_ORCHESTRATOR_INBOX_ENABLED` to the env-var section.
- `docs/openapi/gateway.yaml` — add the two operations with schemas.

### claude-pilot-py

- `claude_pilot/inbox_writer.py` (new) — thin POST client. Reads `MIKA_GATEWAY_URL` + `MIKA_INTERNAL_TOKEN` (already known to claude-pilot per `mika/CLAUDE.md` env-var section) and `MIKA_ORCHESTRATOR_ID` if set. POSTs a `handoff` message on successful session end. No SSE consumer in this ticket (spawn → orchestrator only, one direction).
- `claude_pilot/session_lifecycle.py` or equivalent (whichever module owns session-end finalization) — call `inbox_writer.post_handoff()` on success path. Gate behind `MIKA_ORCHESTRATOR_INBOX_ENABLED=1`. When flag off OR `MIKA_ORCHESTRATOR_ID` unset, skip silently — preserves current filesystem-inbox path as the only side effect.
- `claude-pilot-py/CLAUDE.md` — document the new env vars and the dual-write semantics.

### Orchestrator-side consumer (operator's Claude Code tenant)

- New polling client invoked from `.claude/commands/mika-handsoff.md` Phase 0 OR as a standalone helper script: `scripts/mika-orchestrator-poll` (preferred — standalone, runs in a tmux pane, surfaces new messages to operator).
- Choice between the two is operator UX, not engineering — defer to operator preference, surface in the smoke-test section.

### Spawn-side correlation (F3 (a') resolution — mika-platform repo)

Cross-repo touches required for orchestrator-id discovery. These land in a companion PR on `mika-platform` (the meta-repo, where these files live):

- `scripts/mika-orchestrator-id` (new) — idempotent id-generate-and-cache script. See "Orchestrator-id discovery" section above for the exact body.
- `scripts/mika-platform-spawn` (edit) — export `MIKA_ORCHESTRATOR_ID="$(scripts/mika-orchestrator-id)"` to the spawned tenant's env alongside the existing `MIKA_SPAWN_ID` export.
- `.claude/commands/mika-spawn.md` (edit) — add `MIKA_ORCHESTRATOR_ID` to the "Related" env-var bullet list.

Branch naming convention for the cross-repo touches: same branch name `feat/1189/mika-gateway-bidirectional-sse-channel` on the `mika-platform` repo, per `mika-platform/CLAUDE.md` → Cross-Repo Development → Branch naming.

Primary repo: `mika` (this plan's branch). Secondary (`mika-platform`) follows the "Primary + direct" pattern from the cross-repo strategy table — the secondary three-file change is small and lands directly on a branch in `mika-platform` without a separate `/mika` dispatch.

## Schema (migration 007)

```sql
-- 007_orchestrator_inbox_messages.sql
CREATE TABLE orchestrator_inbox_messages (
  id              BIGSERIAL PRIMARY KEY,
  orchestrator_id TEXT NOT NULL,
  spawn_id        TEXT,                          -- nullable for orchestrator-originated messages (future)
  kind            TEXT NOT NULL CHECK (kind IN ('handoff', 'update', 'ack')),
  body            JSONB NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  delivered_at    TIMESTAMPTZ                    -- set when SSE stream emits this row to a subscriber
);

CREATE INDEX orchestrator_inbox_messages_recv_idx
  ON orchestrator_inbox_messages (orchestrator_id, id);

CREATE INDEX orchestrator_inbox_messages_undelivered_idx
  ON orchestrator_inbox_messages (orchestrator_id, created_at)
  WHERE delivered_at IS NULL;
```

The `dispatch` kind from the ticket sketch is dropped from v1 because dispatch is deferred. `id` (monotonic) is the SSE cursor — clients resume by `Last-Event-Id: <id>`. `delivered_at` is observational (lets us track lag); it does NOT gate redelivery — replay-from-cursor is authoritative.

Retention: a periodic cleanup mirroring the existing `outbound_messages` 7-day batched purge (`crates/mika-gateway/CLAUDE.md` → Agent Identification & Reply Routing). Cap details in Q4.

## Wire protocol

Two endpoints in v1:

### `POST /orchestrator/inbox/{orchestrator_id}/message`

Auth: `Authorization: Bearer <MIKA_INTERNAL_TOKEN>` (reuse the existing gateway shared-secret pattern; see `crates/mika-gateway/src/routes.rs` `require_bearer_token` middleware around lines 138–145).

Body (256 KB max — same as `/send`):

```json
{
  "spawn_id": "string|null",
  "kind": "handoff" | "update" | "ack",
  "body": { /* free-form JSON; for `handoff`, schema mirrors mika-platform#100 inbox file */ }
}
```

Response: `201 Created` with `{"id": <bigserial>}` on success; `400` on schema violation; `401` on bad bearer.

### `GET /orchestrator/inbox/{orchestrator_id}/stream`

Auth: same bearer pattern. Returns `text/event-stream`. Each event:

```
id: <bigserial>
event: message
data: {"id":<n>,"spawn_id":"...","kind":"handoff","body":{...},"created_at":"..."}
```

Client resumes with `Last-Event-Id: <last-seen-id>` header on reconnect. Server emits all rows with `id > last-seen-id` for the given `orchestrator_id`, then keep-alive pings every 30s. On pod restart, clients reconnect and replay from cursor.

Initial v1 implementation can be a **long-poll loop reading rows** rather than `LISTEN/NOTIFY` — the persistence is the source of truth and a 1–2 second poll cadence is enough for the "5+ paste-cycles → real-time" win. `LISTEN/NOTIFY` is a follow-up optimization once volume justifies it.

## Auth model

Reuse `MIKA_INTERNAL_TOKEN` for v1. The token is already known to claude-pilot (per env-var docs), and the gateway already validates it on `/send` via `require_bearer_token`. No new auth surface introduced in this ticket.

This means orchestrator and spawn tenants share an auth identity at the bearer layer; orchestrator/spawn distinction is carried by path (`orchestrator_id` segment) and `spawn_id` field, not by auth. That is acceptable for v1 because both tenants are operator-controlled developer infrastructure. If multi-operator (Q2 follow-up) introduces a need to scope tokens per-operator, that work introduces a separate auth surface.

## Backward compatibility & migration

`MIKA_ORCHESTRATOR_INBOX_ENABLED` (gateway env + claude-pilot env):

- **unset / `0`**: filesystem inbox is the only path. Gateway endpoints return 404. Default-off, zero behavioral change.
- **`1` (dual-write)**: spawn writes to BOTH filesystem inbox AND the new HTTP endpoint. Filesystem readers (`mika-handsoff.md` Phase 0) keep working. Orchestrator polling client (when running) sees real-time pushes. Both paths read each spawn's completion exactly once.
- **`2` (gateway-only)** — out of scope for this ticket. Cutover is a separate decision after dogfooding.

When `MIKA_ORCHESTRATOR_ID` is unset on the spawn side, the HTTP write is skipped silently — same skip-silently semantics as mika-platform#100's filesystem write when `MIKA_SPAWN_ID` is unset.

## Phases

### Phase 1a — Orchestrator-id mechanism (mika-platform repo, ships first)

Lands as a tiny PR on `mika-platform` so the env var is in place before claude-pilot-py wires up to it.

1. Add `scripts/mika-orchestrator-id` per the body above. Make executable.
2. Edit `scripts/mika-platform-spawn` to export `MIKA_ORCHESTRATOR_ID="$(scripts/mika-orchestrator-id)"` in the same export block that already produces `MIKA_SPAWN_ID`.
3. Edit `.claude/commands/mika-spawn.md` "Related" section to mention `MIKA_ORCHESTRATOR_ID`.
4. Smoke-test by spawning a no-op tenant and confirming `env | grep MIKA_ORCHESTRATOR_ID` inside the spawn matches the cached id from `~/.mika/orchestrator/id`.

Acceptance: spawning any non-bare tenant exports a stable `MIKA_ORCHESTRATOR_ID` that survives orchestrator-side restart. PR cross-references this plan + mika#1189.

### Phase 1 — Schema + endpoints (gateway)

1. Write migration 007.
2. Implement `orchestrator_inbox.rs` with the POST handler + persistence ops.
3. Implement the SSE GET handler (long-poll loop reading rows by cursor). Define poll interval as a named constant `ORCHESTRATOR_INBOX_POLL_INTERVAL: Duration = Duration::from_millis(1500)` at module top, NOT a hardcoded literal — keeps it trivially tunable per architect NF3.
4. Wire both into `build_router()` with `require_bearer_token` middleware.
5. Add OpenAPI spec entries.
6. Unit tests: POST happy path, POST schema rejection, SSE cursor replay, SSE keep-alive emission.

Acceptance: `cargo test -p mika-gateway` green. Manual `curl` POST then `curl -N` SSE on a local gateway demonstrates persistence and replay.

### Phase 2 — claude-pilot-py session-end writer

1. Add `inbox_writer.py` with a single `post_handoff()` function.
2. Hook into session-end finalization on success path.
3. Gate behind `MIKA_ORCHESTRATOR_INBOX_ENABLED` and presence of `MIKA_ORCHESTRATOR_ID`.
4. Tests for the new module (mock HTTP).
5. Update `claude-pilot-py/CLAUDE.md`.

Acceptance: claude-pilot test suite green. Manual run of claude-pilot with the env vars set emits a row in the gateway Postgres table.

### Phase 3 — Orchestrator polling client

1. Add `scripts/mika-orchestrator-poll`. Reads `MIKA_GATEWAY_URL`, `MIKA_INTERNAL_TOKEN`, `MIKA_ORCHESTRATOR_ID`. Long-runs an SSE client, prints each new message to stdout with a clear "INBOX:" prefix.
2. Document operator UX in `.claude/commands/mika-handsoff.md` (a "before invoking, check the inbox-poll pane" note) and/or `docs/operator/`.
3. Smoke-test instructions for the cross-tenant loop.

Acceptance: operator can run the poll script in a tmux pane and see spawn handoffs appear within the poll cadence (1–2s). No `/mika-handsoff` Phase 0 invocation required.

### Phase 4 — Retention + observability

1. **Cleanup mechanism** (per architect NF2 — must be named, not implicit): a tokio background task in `crates/mika-gateway/src/main.rs` runs every ~100 webhook-counter ticks (mirroring the existing `outbound_messages` cleanup pattern documented in `crates/mika-gateway/CLAUDE.md` → "Agent Identification & Reply Routing"). The task executes `DELETE FROM orchestrator_inbox_messages WHERE created_at < now() - interval '7 days'`. TTL is a named constant `ORCHESTRATOR_INBOX_RETENTION_DAYS: i64 = 7` so future tuning is one edit.
2. Structured log events: `orchestrator_inbox_message_received` (POST handler), `orchestrator_inbox_subscriber_connected` (SSE handler), `orchestrator_inbox_retention_purged` (cleanup task — log row count purged).
3. Optional: `mika webhook` CLI parity for inbox inspection (`mika orchestrator inbox list`, etc.). Deferred unless operator asks for it.

Acceptance: 7-day-old test rows are purged on cleanup tick. Logs surface in `mika-gateway` log stream.

## Acceptance criteria (plan-level, complementing the ticket body's AC list)

These ACs are the implementation-side view; they ladder up to the ticket body's AC1–AC5 + Phase 1a smoke-test.

- **PAC1.** Migration 007 lands and the `orchestrator_inbox_messages` table exists in the gateway Postgres with the two indexes specified.
- **PAC2.** `POST /orchestrator/inbox/{orchestrator_id}/message` accepts a `handoff` message and persists it. Bearer auth required.
- **PAC3.** `GET /orchestrator/inbox/{orchestrator_id}/stream` returns `text/event-stream`, replays rows from `Last-Event-Id` cursor, and emits keep-alives every 30s.
- **PAC4.** claude-pilot-py session end POSTs a `handoff` message when `MIKA_ORCHESTRATOR_INBOX_ENABLED=1` and `MIKA_ORCHESTRATOR_ID` is set; skips silently otherwise. The filesystem-inbox path from mika-platform#100 remains operational unchanged.
- **PAC5.** `scripts/mika-orchestrator-id` is idempotent — first invocation generates + caches; later invocations print the cached id. `scripts/mika-platform-spawn` exports `MIKA_ORCHESTRATOR_ID` from this script into every non-bare spawn.
- **PAC6.** Smoke test: with the poll script running in a separate pane, dispatch a no-op spawn that reaches `/mika-handsoff` Phase 6 on the success path. Confirm the poll script prints the handoff line within 2s of the spawned tenant exiting.
- **PAC7.** Retention task purges rows older than 7 days; constants `ORCHESTRATOR_INBOX_POLL_INTERVAL` and `ORCHESTRATOR_INBOX_RETENTION_DAYS` are named, not hardcoded literals.
- **PAC8.** Gateway and claude-pilot-py CLAUDE.md files are updated to document the new env var, endpoints, and dual-write semantics. `.claude/commands/mika-spawn.md` documents the new `MIKA_ORCHESTRATOR_ID` env var.

## Open questions

Q1 and Q2 are resolved (see "Premise corrections → 2. Gateway placement" and "Orchestrator-id discovery (F3 resolution)" above). Remaining open items are non-blocking ratifications:

| ID | Question | Why it's load-bearing |
|---|---|---|
| Q3 | **Bidirectional dispatch primitive**: is "orchestrator → spawn 'iterate on X'" achievable at all without an SDK hook for stateful Claude Code sessions? Or must it route through a fresh spawn (which already works via `/mika-spawn`)? | Determines whether the deferred follow-up is "extend this ticket's protocol" or "this is a non-goal." Architect (first pass NF1) called this permanently a non-goal until SDK hook is confirmed; ticket "Out of scope" already disclaims it. |
| Q4 | **Retention TTL**: 7 days mirrors `outbound_messages`. Is that the right window for developer-workflow messages, or shorter (1 day)? | Affects retention task design. Architect (first pass NF2) ratified 7 days as acceptable but flagged that the cleanup mechanism should be named in the plan (see Phase 4 below). |
| Q5 | **SSE persistence semantics on pod restart**: long-poll-from-cursor handles K8s rolling restarts but adds latency. Acceptable? Or is `LISTEN/NOTIFY` worth the complexity in v1? | Affects Phase 1 implementation shape. Architect (first pass NF3) ratified polling as the correct v1 call; named-constant for the poll interval is the implementation guidance. |

## Grooming history

| Date | Pass | Outcome |
|---|---|---|
| 2026-05-17 | First-pass (mika-arch session `9d81e315-4ba6-4995-9991-e941866bd3b2`) | **ESCALATE** — 3 blocking findings: F1 (Phase 0 Pin absent), F2 (spec divergence — ticket AC1 4-endpoint vs plan 2-endpoint), F3 (orchestrator-id discovery load-bearing). Q1 (gateway placement) ratified inline. |
| 2026-05-17 | Operator resolution (Vincent) | F2: edited ticket body — AC1 rewritten with Rust+markdown+YAML-manifest grounding, "Out of scope" entry added for the orchestrator→spawn endpoints. AC2/AC3 also tightened (dual-write + 1–2 s polling cadence). F3: ratified option (a') — orchestrator-id generation + cache + spawn-side export. F1: absorbed into this plan revision (Phase 0 Pin section above). |
| 2026-05-17 | Architect retro (same session) | F2 acknowledged on the body edit, no callback flagged. Standing by for second-pass brief. |

## Risks

- **Hook unavailability for orchestrator surfacing** — polling is the v1 fallback. If polling UX is unacceptable, real-time push needs a verified hook surface. Mitigation: explicit polling-only scope; defer hook integration.
- **Auth identity sharing in v1** — both tenants use the same internal token. If a spawn is compromised, it can impersonate the orchestrator. Acceptable for operator-controlled dev infra in v1; revisit if cloud-hosted multi-operator lands.
- **Orchestrator-id isolation gap** (per architect NF4) — `MIKA_INTERNAL_TOKEN` bearer auth lets any valid bearer caller read or write to any `orchestrator_id` inbox. For v1 solo operator this is fine — there's only one `orchestrator_id`. The multi-operator follow-up MUST address row-level authorization before exposing to multiple operators. Document this gap explicitly in `crates/mika-gateway/CLAUDE.md` when the endpoints land.
- **Dual-write race** — filesystem-inbox path AND HTTP path can both fire on a successful handoff. Both readers must tolerate the same event being seen twice (orchestrator Phase 0 + poll-pane print). Mitigation: idempotency by `spawn_id` on the consumer side; documented in operator docs.

## Alternative path (if architect picks Option B in Q1)

If the architect prefers a new local daemon over gateway:

- Phase 1 shifts from "migration 007 + gateway endpoints" to "new `mika-coord` crate with embedded SQLite + bind 127.0.0.1".
- claude-pilot-py inbox writer URL changes from gateway to `127.0.0.1:<port>`.
- Multi-operator becomes a future deployment exercise rather than working out of the box.

Schema, wire protocol, and AC list stay structurally identical — only the host changes.

## Related

- mika-platform#100 (closed) — Pattern 3 filesystem inbox. Supersedes via dual-write.
- `mika-platform/.claude/commands/mika-spawn.md` — spawn-id env exporter.
- `mika-platform/.claude/commands/mika-handsoff.md` — Phase 0 reader, Phase 6 writer.
- `crates/mika-gateway/src/a2a_routes.rs:130-142` — SSE pass-through reference.
- `crates/mika-gateway/src/routes.rs:138-145` — bearer-auth middleware reference.
- `crates/mika-gateway/CLAUDE.md` — Postgres migrations 002–006 (007 adds here).
- `mika/docs/architecture/review-guide.md` — SoC / orthogonality principles relevant to Q1.
