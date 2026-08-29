---
title: Deployment
description: Operator guide for deploying Mika in hosted mode with Docker
---

# Mika Deployment Guide

Operator-focused documentation for deploying Mika in hosted mode.

---

## 1. Architecture Overview

Mika uses a hub-and-spoke architecture with per-customer container isolation.

```
Telegram
   |
   v
[Ingress / Load Balancer]
   |
   v
[mika-gateway]
   |   Stateless, Postgres-backed
   |   Routes by telegram_chat_id
   |
   +---> [mika-{uuid-1}]
   |        SQLite on persistent volume
   |        Axum HTTP server
   |
   +---> [mika-{uuid-2}]
   |        SQLite on persistent volume
   |        Axum HTTP server
   |
   +---> [mika-{uuid-N}] ...
```

The gateway source lives in `crates/mika-gateway/` in this repo.

---

## 2. Prerequisites

| Requirement | Details |
|-------------|---------|
| Docker | For building container images |
| Anthropic API key | For Claude API access |
| Container registry | Any Docker-compatible registry (GHCR, ECR, etc.) |

---

## 3. Building the Agent Docker Image (~95MB)

```bash
docker build -f Dockerfile.agent -t mika-agent:dev .
```

Build details:
- **Dashboard builder:** `node:22-slim` — builds the React SPA (`npm ci && npm run build --prefix dashboard`). The built `dashboard/dist/` is copied into the Rust builder for embedding via `rust-embed`.
- **Builder:** `rust:1.93-slim` with gcc, libc-dev, pkg-config (no OpenSSL — uses rustls)
- **Runtime:** `debian:bookworm-slim` with ca-certificates, wget, file, jq, gh (GitHub CLI v2.65.0), and gws (Google Workspace CLI v0.13.3) — both with SHA256 checksum verification
- **Binary:** `mika-spirit` (Axum HTTP server)
- **User:** `mika` (UID 1000), non-root
- **Port:** 8080
- **Healthcheck:** `wget -q --spider http://localhost:8080/health` (10s interval, 5s start period)
- **Config:** Serde defaults compiled-in; `~/.mika/.env` for secrets, `~/.mika/config.toml` for overrides
- **Data dir:** `/home/mika/.mika` (persistent volume mount point)

### Pushing to Registry

```bash
docker tag mika-agent:dev registry.example.com/mika-agent:v0.1.0
docker push registry.example.com/mika-agent:v0.1.0
```

### BuildKit Cache Mounts

Both Dockerfiles use BuildKit `--mount=type=cache` for the cargo registry and `target/` directory, providing incremental compilation across builds without dummy-source layers. The final binary is copied out of the cache mount to the runtime stage.

Requires Docker BuildKit (enabled by default in Docker 23+). Rebuilds after source changes are fast because compiled dependencies persist in the named cache volumes.

---

## 3b. Building the Gateway Docker Image

```bash
docker build -f Dockerfile.gateway -t mika-gateway:dev .
```

Build details:
- **Builder:** `rust:1.93-slim` (no native build deps — uses rustls, no bundled SQLite)
- **Runtime:** `debian:bookworm-slim` with ca-certificates, wget
- **Binary:** `mika-gateway` (Axum HTTP server)
- **User:** `mika` (UID 1000), non-root, no home directory (stateless)
- **Port:** 8080
- **Healthcheck:** `wget -q --spider http://localhost:8080/readyz` (10s interval, 10s start period)

The gateway image is leaner than the agent image — no GitHub CLI, no file/jq utilities, no home directory. It uses the same BuildKit cache mount strategy as the agent image.

---

## 3c. CI/CD Pipeline

Three GitHub Actions workflows automate testing, versioning, and binary distribution.

### CI (`ci.yml`)

Runs on every PR and push to `main`:
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

### Release PR (`release-pr.yml`) — **DISABLED since 2026-08-29**

**This workflow no longer runs.** The `push: main` trigger was removed (mika#2047); only
`workflow_dispatch` remains, as a manual verification path for whoever picks up the resume ticket.
There is currently **no automated release, no automated tag, and no automated changelog** in this repo.

Why: release-please failed on every merge and kept `main` permanently red. Its `rust` strategy cannot
handle our virtual workspace — it does not expand the `members = ["crates/*"]` glob, then hands the
root `Cargo.toml` (which has no `[package]` section) to a package updater and throws. 300 runs between
2026-06-05 and 2026-08-29, 300 failures, zero successes.

There is no known consumer of this release channel — last tag `v0.12.2` (2026-05-09) — and deployment
happens from `main` via `make deploy`. Release assets do still see residual downloads (38 on `v0.12.2`,
132 across all releases), and `install.sh` below pulls from GitHub Releases; but this workflow has
produced nothing since 2026-05-09, so turning it off takes away nothing those users were still getting.
They are pinned to `v0.12.2` regardless. The full reasoning lives in the workflow file's header comment.

- Diagnosis: mika#2047
- Resume ticket (what it would take to turn it back on): mika#2048
- History of this failure class: `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`

When it was live, it used `googleapis/release-please-action` to maintain a persistent Release PR with
version bumps and changelog; merging that PR created the `v{version}` tag and GitHub Release. It never
published to crates.io — all crates are `publish = false`. It required `RELEASE_PLZ_TOKEN` (a PAT with
`contents: write` and `pull-requests: write`) rather than `GITHUB_TOKEN`, so that the tag push would
trigger the release binary workflow.

### Release Binaries (`release.yml`) — dormant

Still enabled, but **dormant in practice**: nothing produces `v*` tags automatically since
`release-pr.yml` was disabled (see above). It is deliberately left in place so that a manually pushed
tag still produces binaries.

Triggered by `v*` tag push:
- Builds cross-platform binaries with `--features telemetry`: x86_64-linux, aarch64-linux (cross-compiled), x86_64-macos, aarch64-macos
- Uploads `mika` (CLI) and `mika-spirit` (HTTP server) to GitHub Releases with SHA256 checksums
- `mika-gateway` excluded — deployed via Docker to Kubernetes, not as a standalone binary
- Dashboard assets are not embedded in release builds (empty placeholder); the `/dashboard/` endpoint shows a branded "disabled" page

### Installer Script

Users can install pre-built binaries via:

```bash
curl -fsSL https://raw.githubusercontent.com/senara-solutions/mika/main/install.sh | sh
```

The script detects platform/architecture, downloads from GitHub Releases, verifies SHA256 checksum, and installs to `~/.local/bin/mika`.

---

## 3d. Docker Compose (Local Development)

A `docker-compose.yml` is provided for local multi-service development:

```bash
# Generate a .env file with all required secrets
mika setup --mode compose

# Start agent + gateway (uses .env in current directory)
docker compose up

# Include a local Postgres instance
docker compose --profile db up
```

**Services:**

| Service | Image | Port | Notes |
|---------|-------|------|-------|
| `agent` | `Dockerfile.agent` | 8081→8080 | Persistent volume at `/home/mika/.mika` |
| `gateway` | `Dockerfile.gateway` | 8080→8080 | Reads `.env` from CWD |
| `postgres` | `postgres:17-alpine` | 5433→5432 | Only with `--profile db` |

The `mika setup --mode compose` command generates a `.env` file in the current directory with all required variables (API keys, tokens, database URL, Telegram config). It prompts interactively for secrets and auto-generates the internal token.

---

## 4. Container Deployment

Each customer gets an agent container with a persistent volume for SQLite storage, plus a shared gateway container.

### Required Environment Variables (Agent Container)

| Variable | Description |
|----------|-------------|
| `MIKA_ANTHROPIC_API_KEY` | Anthropic API key (default provider). See docs for other provider keys. |
| `MIKA_ROUTING_URL` | Gateway URL for outbound message delivery |
| `MIKA_INTERNAL_TOKEN` | Shared 64-char hex bearer token |

### Required Environment Variables (Gateway Container)

| Variable | Description |
|----------|-------------|
| `MIKA_DATABASE_URL` | Postgres connection string |
| `MIKA_TELEGRAM_BOT_TOKEN` | Telegram Bot API token |
| `MIKA_TELEGRAM_WEBHOOK_SECRET` | 64-char hex secret for webhook validation |
| `MIKA_TELEGRAM_WEBHOOK_URL` | Public HTTPS URL for Telegram webhook delivery |
| `MIKA_INTERNAL_TOKEN` | Shared 64-char hex bearer token (same as agent) |

### Container Security

Run containers as non-root (the Docker images use UID 1000). Recommended security settings:
- Drop all Linux capabilities
- Disable privilege escalation
- Use a read-only root filesystem with writable mounts only for `/home/mika/.mika` and `/tmp`

### Storage

- Agent containers need a persistent volume mounted at `/home/mika/.mika` for SQLite data
- Mount `/tmp` as a small tmpfs or writable volume (read-only root filesystem)
- Only one container should access each volume at a time (SQLite is single-writer)
- No CPU limit recommended — the agent is I/O-bound (Claude API calls), CPU limits cause throttling

---

## 5. Security

### Token Generation

All tokens are 64-character hex strings (32 bytes of randomness), generated with:

```bash
openssl rand -hex 32
```

### Constant-Time Comparison

Token comparisons use the `subtle` crate's `ConstantTimeEq` trait to prevent timing attacks. Token length is validated at startup (must be exactly 64 hex chars).

### Non-Root Containers

The agent image runs as user `mika` (UID 1000). The recommended container security context enforces non-root execution, drops all capabilities, disables privilege escalation, and enables a read-only root filesystem.

### Read-Only Root Filesystem

The agent container uses a read-only root filesystem. Writable paths:
- `/home/mika/.mika` (persistent volume for SQLite)
- `/tmp` (small writable mount)

### Encrypted Volumes

SQLite databases are stored as plaintext on encrypted volumes. Mika does not implement application-level encryption. Use your infrastructure's volume encryption (e.g., encrypted block storage) to protect data at rest.

---

## 6. Troubleshooting

### 429 Agent Busy

**Symptom:** Container returns HTTP 429. User sees "I'm having trouble right now."

**Cause:** The agent container serializes agent loop execution with a `tokio::sync::Mutex`. If one request is being processed, subsequent requests are rejected with 429 (non-blocking `try_lock`).

**Resolution:** Expected behavior during long Claude API calls. If persistent, check container logs:

```bash
docker logs mika-<uuid> --tail 100
```

### Agent Offline

**Symptom:** User receives "Your Mika assistant is currently offline. Please contact your administrator or check your subscription status at console.getmika.ai."

**Cause:** The gateway could not establish a TCP connection to the agent container. This happens when the container is scaled to zero, deprovisioned, or the DNS name does not resolve (e.g., pod not yet scheduled).

**Resolution:**
- Verify the agent pod is running: `kubectl get pods -n <agents-namespace> -l app=mika-<customer-id>`
- Check if the Service exists: `kubectl get svc mika-<customer-id> -n <agents-namespace>`
- If the container was intentionally scaled down, scale it back up
- For DNS failures, verify the namespace matches `MIKA_AGENTS_NAMESPACE`

### Container Crash Loop

**Symptom:** Customer container restarts repeatedly.

**Common causes:**
- Missing or invalid `MIKA_ANTHROPIC_API_KEY`
- Corrupted SQLite database
- Missing persistent volume mount

**Resolution:**

```bash
# Check container logs
docker logs mika-<uuid>

# Check container logs (previous instance if using orchestration)
docker logs mika-<uuid> --tail 200
```

If the SQLite database is corrupted, remove the persistent volume data and let the container bootstrap a fresh database. This loses all conversation history and memory.

### Failed Sends Accumulating

**Symptom:** The container's `failed_sends` table grows.

**Cause:** The routing endpoint was unreachable when the container tried to send.

**Resolution:** Failed sends are automatically flushed (up to 5 at a time) before each inbound message. If the routing endpoint is restored, the next inbound message triggers delivery of pending messages.
