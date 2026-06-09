# mika-gateway — Webhook Router

Telegram and GitHub webhook router with Postgres customer registry. Handles text messages, images, and GitHub App events. Env-var-only config.

## Endpoints

| Endpoint | Method | Auth | Purpose |
|----------|--------|------|---------|
| `/webhook/telegram` | POST | Webhook secret | Inbound Telegram messages (single-bot mode) |
| `/webhook/telegram/{customer_id}` | POST | Per-customer webhook secret | Inbound Telegram messages (per-customer bot) |
| `/webhook/github` | POST | HMAC-SHA256 | Inbound GitHub App events |
| `/send` | POST | Internal token | Outbound relay with `agent_name` identification |
| `/health`, `/readyz`, `/livez` | GET | None | Health probes |
| `/version` | GET | None | Returns `{"version":"<semver>","git_hash":"<short-hash>"}` |
| `/webhook/dlq` | GET | Internal token | List DLQ entries (pending + dead) |
| `/webhook/dlq/{delivery_id}/replay` | POST | Internal token | Replay a single DLQ entry |
| `/webhook/dlq/replay-all` | POST | Internal token | Replay all dead DLQ entries |
| `/a2a/{customer_id}/{agent_name}` | POST | API key | A2A protocol proxy (2MB limit) |
| `/a2a/{customer_id}/{agent_name}/agent.json` | GET | None | Agent Card proxy |
| `/orchestrator/inbox/{orchestrator_id}/message` | POST | Internal token | Persist spawn→orchestrator message (256KB limit, mika#1189) |
| `/orchestrator/inbox/{orchestrator_id}/stream` | GET | Internal token | SSE stream of inbox messages with cursor replay (mika#1189) |

## GitHub Webhook Integration

HMAC-SHA256 signature validation via `X-Hub-Signature-256`. Event routing:
- `issues.assigned` and `issues.labeled` and `issue_comment.created` and `pull_request_review.submitted` and `pull_request.closed` and `check_suite.completed(failure/timed_out/success)` -> mika-dev
- `pull_request.opened/synchronize/review_requested` -> mika-qa
- Delivery UUID dedup via 10k-entry LRU cache
- 256KB body limit
- Multi-tenant routing via `github_repos` table lookup with `agent_base_url` fallback for single-tenant mode
- Machine user assignee filtering: `MIKA_GITHUB_APP_LOGIN` in per-agent `.env` (e.g., `~/.mika/agents/mika-dev/.env`) should match the machine user login (e.g., `mika-platform-dev`). Filtering logic lives in the self-dev skill prompt, not in gateway code.
- **Webhook skill denylist (#845):** `WEBHOOK_SKILL_DENYLIST` const in `github.rs` blocks operator-only skills from being triggered via webhook events. For `issues.labeled` events, the label name is checked against the denylist (case-insensitive). Denylisted events are dropped with `StatusCode::OK` (prevents GitHub retries) and a `warn!` log. Currently contains `"dev-groom"`. This is Layer 3 defense-in-depth — Layer 1 (`well_known_agents.rs` `disabled_skills`) is the primary check at the agent level.
- **Synchronize no-diff guard (#886):** For `pull_request.synchronize` events, the gateway compares `before` and `after` commit SHAs (from the webhook payload) via the GitHub Compare API (`GET /repos/{repo}/compare/{before}...{after}`). If zero files changed (trailer-only amend, commit-message-only push), the event is suppressed — no mika-qa session is created. Prevents cross-session duplicate APPROVED reviews. Requires `MIKA_GITHUB_APP_ID`, `MIKA_GITHUB_APP_PRIVATE_KEY`, and `MIKA_GITHUB_APP_INSTALLATION_ID` to be set. Fail-open on all error paths: API timeout (5s), HTTP errors, missing credentials, token refresh failure, malformed response. Guard runs between the skill denylist check (step 9b) and semaphore acquisition (step 10). Structured log event: `webhook_synchronize_no_diff_change`.
- **Body truncation caps (#911):** `format_event_text` truncates webhook body fields per event type via `truncate_body()`. Two named constants: `DEFAULT_GITHUB_BODY_TRUNCATION_CHARS = 2_000` (issue, PR, comment) and `GITHUB_REVIEW_BODY_TRUNCATION_CHARS = 16_000` (pull_request_review only). The review cap is higher because mika-qa review bodies are structured-and-long (3–5 KB typical: DIFF ANALYSIS + PLAN-AC VERIFICATION + BUILD VERIFICATION + VERDICT) and the engine's verdict parser depends on the VERDICT token surviving transport. See `docs/solutions/best-practices/gateway-truncation-cap-per-event-type-calibration-2026-05-01.md` for the per-event-type calibration principle.

### Inbound delivery retry (#589)

The spawned forwarding task retries on HTTP 429/5xx or request timeouts using the fixed schedule `[2s, 5s, 15s, 60s, 300s]` with ±25% per-attempt jitter (prevents synchronized retry bursts on the same agent). Permanent failures (HTTP 4xx other than 429, non-localhost connection errors, or unresolvable route) stop retries immediately. Localhost connection errors are retryable (#1293) — the agent may be restarting during a deploy. Route resolution (`github_repos` lookup + `agent_mapping`) is cached across retries — a single Postgres query per event regardless of retry count.

Semaphore lifecycle during retry: the 30-permit `webhook_semaphore` (shared with Telegram) is released during each retry sleep and re-acquired via `try_acquire_owned` before the next attempt. If the semaphore is full on re-acquire, the retry is abandoned with a dedicated ERROR log (`semaphore at capacity during retry`), distinct from the `retry budget exhausted` ERROR emitted when all 6 attempts return a retryable failure.

The delivery LRU cache has no TTL (size-based eviction only). Under extreme webhook volume (>10k deliveries during a single 300s retry sleep), the `X-GitHub-Delivery` entry may be evicted and a GitHub redelivery would bypass gateway dedup. Agent-side idempotency (task unique index on `reference_url`) mitigates double-processing.

### Dead-letter queue (#590)

Events that exhaust the retry budget or are abandoned due to semaphore pressure are persisted in the `webhook_deliveries` Postgres table (migration 006) with `status='pending'`. A background tokio task wakes every 30s, selects pending rows past their exponential backoff window (`30s * 2^attempts`, capped at 1h), and re-attempts delivery via the same `forward_to_resolved_route()` path. Route is re-resolved on each worker attempt (container URLs may change). After 10 worker attempts, status transitions to `'dead'`.

The DLQ respects the shared 30-permit webhook semaphore — if all permits are held, the worker skips forwarding for that tick. Manual replay via `POST /webhook/dlq/{id}/replay` and `POST /webhook/dlq/replay-all` follows the same semaphore-gated delivery path. CLI: `mika webhook list-dead`, `mika webhook replay <id>`, `mika webhook replay-all`.

## Agent Identification & Reply Routing

- Outbound messages carry `agent_name` in the `/send` payload; gateway prepends `[agent_name]` to Telegram text and stores `(telegram_message_id, chat_id, agent_name)` in `outbound_messages` Postgres table
- Parses `reply_to_message` from Telegram updates; looks up the originating agent via `outbound_messages` and forwards the inbound message with `"agent": "<name>"` to the correct agent in the container
- Periodic cleanup: purges `outbound_messages` older than 7 days (batched, every ~100 webhooks)

## A2A Auth

API keys are SHA-256 hashed and stored in Postgres `a2a_api_keys` table (migration 003); validated via `validate_a2a_api_key()` with expiry and revocation checks. See `crates/mika-a2a/CLAUDE.md` for A2A protocol details.

## Request Logging

`tower_http::trace::TraceLayer` middleware logs method, path, status code, and latency for every request. `inject_request_meta` middleware (inner to TraceLayer) copies method+path from request into response extensions so `on_response` emits them as top-level JSON event fields (not just nested in the `spans` array). Health probe paths (`/health`, `/readyz`, `/livez`, `/version`) are logged at DEBUG level to reduce noise from Kubernetes checks; all other routes log at INFO level. 5xx responses are logged at WARN. Connection-level failures (timeouts, stream errors) are logged at ERROR with classification.

## Postgres Migrations

- Migration 002: creates `outbound_messages` table
- Migration 003: creates `a2a_api_keys` table
- Migration 004: creates `github_repos` table (maps `repo_full_name` -> `customer_id` for multi-tenant GitHub webhook routing)
- Migration 005: adds `agent_mapping JSONB NOT NULL DEFAULT '{}'` to `github_repos` for per-repo agent name overrides (keys are default agent names from `route_event()`, values are customer's replacement names; `apply_agent_mapping()` validates names via `is_valid_agent_name()` and falls back to defaults for invalid values)
- Migration 006: creates `webhook_deliveries` table for dead-letter queue (delivery_id PK, event_type, target_agent, repo_full_name, payload, request_id, status CHECK IN pending/delivered/dead, attempts, last_attempt_at, last_error). Partial indexes on `(status, last_attempt_at) WHERE status='pending'` and `(created_at DESC) WHERE status='dead'`.
- Migration 007: creates `orchestrator_inbox_messages` table for the orchestrator inbox v2 channel (mika#1189). Columns: `id BIGSERIAL PK`, `orchestrator_id TEXT NOT NULL`, `spawn_id TEXT` (nullable), `kind TEXT NOT NULL CHECK IN ('handoff','update','ack')`, `body JSONB NOT NULL`, `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`, `delivered_at TIMESTAMPTZ`. Indexes: `(orchestrator_id, id)` for cursor replay; partial `(orchestrator_id, created_at) WHERE delivered_at IS NULL` for diagnostic queries on undelivered rows (retention sweeps purge by `created_at` alone and don't use this partial index — see migration comment).
- Migration 008: adds `bot_token`, `bot_username`, `webhook_secret` columns to `customers` table (per-customer Telegram bot support, mika#1454). All nullable — NULL means single-bot mode fallback.

## build.rs

- `cargo::rerun-if-changed=migrations` so new migration files invalidate the incremental compilation cache (SQLx `migrate!()` is a compile-time proc macro)
- Captures short git hash via `git rev-parse --short HEAD` into `GIT_HASH` env var for the `/version` endpoint (falls back to `"unknown"` when `.git` is absent); watches `.git/HEAD` and `.git/refs` for rebuild on new commits

## Gateway Environment Variables

- `MIKA_DATABASE_URL` — Postgres connection string
- `MIKA_TELEGRAM_BOT_TOKEN` — Telegram Bot API token. Required only in single-bot mode.
- `MIKA_TELEGRAM_WEBHOOK_SECRET` — 64-char hex secret for webhook validation. Required only in single-bot mode.
- `MIKA_TELEGRAM_WEBHOOK_URL` — Public HTTPS URL for Telegram webhook delivery. Required only in single-bot mode.
- `MIKA_TELEGRAM_SINGLE_BOT_MODE` — When `1` or `true`, the gateway uses the global `MIKA_TELEGRAM_BOT_TOKEN` for all customers (legacy single-bot behavior). Requires `MIKA_TELEGRAM_BOT_TOKEN`, `MIKA_TELEGRAM_WEBHOOK_SECRET`, and `MIKA_TELEGRAM_WEBHOOK_URL` to be set. Default: off (per-customer bot mode).
- `MIKA_INTERNAL_TOKEN` — Shared 64-char hex bearer token
- `MIKA_AGENTS_NAMESPACE` — K8s namespace where agent pods run (default: `mika-agents`). Used for FQDN construction in cross-namespace DNS resolution (`http://mika-{id}.{ns}.svc.cluster.local:8080`). Override for environment-scoped namespaces (e.g. `mika-agents-prd`).
- `MIKA_GITHUB_WEBHOOK_SECRET` — Secret for validating inbound GitHub App webhooks via HMAC-SHA256. Arbitrary string (not hex-constrained like Telegram). When absent, `POST /webhook/github` returns 404.
- `MIKA_GITHUB_APP_ID` — GitHub App ID (u64). Required for the synchronize no-diff guard (#886).
- `MIKA_GITHUB_APP_PRIVATE_KEY` — GitHub App private key (base64-encoded PEM). Required for the synchronize no-diff guard (#886). Encode with: `base64 -w0 < your-app.pem`.
- `MIKA_GITHUB_APP_INSTALLATION_ID` — GitHub App installation ID (u64). Required for the synchronize no-diff guard (#886). All 3 GitHub App vars must be set; when incomplete, the no-diff guard is disabled (fail-open).
- `MIKA_ORCHESTRATOR_INBOX_ENABLED` — Orchestrator inbox feature flag (mika#1189). Default off — `/orchestrator/inbox/*` endpoints return 404. Set `1` (or `true`, case-insensitive) to enable dual-write with the mika-platform#100 filesystem inbox. `2` (gateway-only cutover) is reserved for a future ticket and currently treated as disabled. Note: bearer auth gates the endpoints by token; orchestrator/spawn distinction is carried by path (`orchestrator_id`) and `spawn_id` field, not by separate tokens — multi-operator deployments will need per-operator scoping before exposure beyond a solo operator.

## Orchestrator Inbox (mika#1189)

Real-time spawn→orchestrator coordination channel. Spawned Claude Code tenants POST handoff messages addressed to their parent orchestrator's id; the orchestrator subscribes via SSE and surfaces messages in-session. Replaces the operator-mediated paste cycle of mika-platform#100 (filesystem-inbox protocol remains operational during dual-write migration).

- Persistence: Postgres table `orchestrator_inbox_messages` (migration 007). `id BIGSERIAL` is the SSE cursor — clients reconnect with `Last-Event-Id: <last-seen>` and the server replays rows with `id > cursor`. `delivered_at` is observational; cursor replay is authoritative.
- Wire shape: POST returns `201 {"id": <bigserial>}`. SSE events emit `id: <n>` + `event: message` + `data: <JSON envelope>`. KeepAlive pings every 30s. Long-poll loop reads rows every `ORCHESTRATOR_INBOX_POLL_INTERVAL` (1.5s by default).
- Retention: background tokio task (spawned in `main.rs`) purges rows older than `ORCHESTRATOR_INBOX_RETENTION_DAYS` (7 days) once an hour. Constants live at the top of `orchestrator_inbox.rs` — code-edit-tunable per plan NF3.
- Validation: `orchestrator_id` path segment must be 1-128 chars `[A-Za-z0-9_-]`; `kind` must be one of `handoff`, `update`, `ack` (mirrors the migration CHECK constraint).
- Structured log events: `orchestrator_inbox_message_received`, `orchestrator_inbox_subscriber_connected`, `orchestrator_inbox_retention_purged`.
