---
title: GitHub App JWT Authentication Module
category: architecture-patterns
date: 2026-04-02
tags: [github-app, jwt, authentication, token-caching, rwlock, security]
---

# GitHub App JWT Authentication Module

## Problem

Mika's GitHub integration relied solely on Personal Access Tokens (PATs) via `MIKA_GITHUB_TOKEN`. PATs are tied to human accounts, have broad scopes, require manual rotation, and share rate limits across all uses. GitHub App installation tokens are org-scoped, short-lived (1 hour), separately rate-limited, and auditable.

## Root Cause

No GitHub App authentication support existed. Adding it required a new module for JWT signing + installation token exchange, config fields for the 3 required env vars, threading through the entire AppState → AgentParams → ToolContext chain, and a doctor check.

## Solution

### Module: `mika-common/src/github_app.rs`

Follows the `OAuthTokenManager` pattern from `oauth.rs`:

- **`GitHubApp` struct** holds `app_id`, `EncodingKey` (pre-parsed RSA key), `installation_id`, `RwLock<Option<CachedToken>>` cache, and `reqwest::Client`
- **`from_settings()`** returns `Option<Arc<Self>>` — `None` if any of 3 config fields missing. Eagerly decodes base64 PEM and parses RSA key (fail-fast)
- **`installation_token()`** uses double-checked locking: read lock fast path → write lock slow path with JWT generation + HTTP exchange → cache result
- **Expiry buffer:** 5 minutes before the 1-hour GitHub expiry (conservative)

### Eager Token Resolution Pattern

Token is resolved **once per agent turn** via `Settings::resolve_github_token()`, not lazily in each tool. This means:
- No structural changes to `ToolContext` (stays `github_token: Option<&'a str>`)
- Zero changes to downstream tools (`run_gh`, `check_task`, `fetch_pr_diff`)
- Trade-off: one `RwLock::read()` per turn even without GitHub tool calls (nanoseconds)

### Key Design Decisions

1. **No disk persistence** — Installation tokens are short-lived; regeneration is cheap (unlike OAuth refresh tokens)
2. **Single shared `Arc<GitHubApp>`** — Created once per agent/server init, cloned to all consumers (TaskDispatcher, AgentState, TeamEngine, DelegateTaskTool). Prevents cache fragmentation
3. **Base64 PEM in env var** — Avoids volume mount complexity in containers. Encode with `base64 -w0 < your-app.pem`
4. **Warn + fallback to PAT** — Never silently mask a broken App config

## Prevention / Best Practices

### When adding token caching with RwLock

- Use double-checked locking: read lock → check validity → drop → write lock → re-check → refresh
- `tokio::sync::RwLock` (not `std::sync`) — async-aware, no poisoning
- Always add an expiry buffer (don't wait for exact expiry)
- Use `try_into()` not `as u64` for timestamp conversions (prevents silent wrapping)

### When threading a new shared resource through the stack

- Create once, share via `Arc::clone()` — don't call `from_settings()` at each construction site
- Follow the existing chain: AppState/AgentState → AgentParams (borrowed ref) → resolved value in ToolContext
- Update ALL agent entry points: `run_agent`, `run_silent_agent`, `run_team_agents`, orchestrator path

### Secret handling for new credentials

- `SecretString` for the raw secret in `Settings`
- Manual `Debug` impl with `[REDACTED]`
- `get_effective_value()` returns `"[SET]"` not the actual value
- `ConfigKeyInfo` with `secret: true`
- Truncate error bodies from external APIs before they propagate to `warn!` logs

## Related

- `docs/solutions/architecture-patterns/dedicated-github-token-agent-operations.md` — Token architecture
- `docs/solutions/architecture-patterns/config-key-rename-across-layers.md` — 9-layer checklist
- `docs/solutions/integration-issues/oauth-pkce-token-exchange-for-anthropic-subscriptions.md` — OAuthTokenManager pattern
- `docs/solutions/security-issues/gh-token-identity-collision-dotenv-leak.md` — GH_TOKEN scrubbing
- GitHub issue: #381
