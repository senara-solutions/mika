# mika-gateway — Webhook Router

Telegram and GitHub webhook router with Postgres customer registry. Handles text messages, images, and GitHub App events. Env-var-only config.

## Endpoints

| Endpoint | Method | Auth | Purpose |
|----------|--------|------|---------|
| `/webhook/telegram` | POST | Webhook secret | Inbound Telegram messages |
| `/webhook/github` | POST | HMAC-SHA256 | Inbound GitHub App events |
| `/send` | POST | Internal token | Outbound relay with `agent_name` identification |
| `/health`, `/readyz`, `/livez` | GET | None | Health probes |
| `/version` | GET | None | Returns `{"version":"<semver>","git_hash":"<short-hash>"}` |
| `/a2a/{customer_id}/{agent_name}` | POST | API key | A2A protocol proxy (2MB limit) |
| `/a2a/{customer_id}/{agent_name}/agent.json` | GET | None | Agent Card proxy |

## GitHub Webhook Integration

HMAC-SHA256 signature validation via `X-Hub-Signature-256`. Event routing:
- `issues.assigned` and `issue_comment.created` and `pull_request_review.submitted` and `pull_request.closed` and `check_suite.completed(failure)` -> mika-dev
- `pull_request.opened/synchronize` -> mika-qa
- Delivery UUID dedup via 10k-entry LRU cache
- 256KB body limit
- Multi-tenant routing via `github_repos` table lookup with `agent_base_url` fallback for single-tenant mode

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

## build.rs

- `cargo::rerun-if-changed=migrations` so new migration files invalidate the incremental compilation cache (SQLx `migrate!()` is a compile-time proc macro)
- Captures short git hash via `git rev-parse --short HEAD` into `GIT_HASH` env var for the `/version` endpoint (falls back to `"unknown"` when `.git` is absent); watches `.git/HEAD` and `.git/refs` for rebuild on new commits

## Gateway Environment Variables

- `MIKA_DATABASE_URL` — Postgres connection string
- `MIKA_TELEGRAM_BOT_TOKEN` — Telegram Bot API token
- `MIKA_TELEGRAM_WEBHOOK_SECRET` — 64-char hex secret for webhook validation
- `MIKA_TELEGRAM_WEBHOOK_URL` — Public HTTPS URL for Telegram webhook delivery
- `MIKA_INTERNAL_TOKEN` — Shared 64-char hex bearer token
- `MIKA_AGENTS_NAMESPACE` — K8s namespace where agent pods run (default: `mika-agents`). Used for FQDN construction in cross-namespace DNS resolution (`http://mika-{id}.{ns}.svc.cluster.local:8080`). Override for environment-scoped namespaces (e.g. `mika-agents-prd`).
- `MIKA_GITHUB_WEBHOOK_SECRET` — Secret for validating inbound GitHub App webhooks via HMAC-SHA256. Arbitrary string (not hex-constrained like Telegram). When absent, `POST /webhook/github` returns 404.
- `MIKA_GITHUB_APP_ID` — GitHub App ID (u64). Used by the gateway for GitHub App identification.
