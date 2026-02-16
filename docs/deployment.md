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
- **Builder:** `rust:1.85-slim` with gcc, libc-dev, pkg-config, libssl-dev
- **Runtime:** `debian:bookworm-slim` with ca-certificates, wget, file, jq, and gh (GitHub CLI v2.65.0 with SHA256 checksum verification)
- **Binary:** `mika-server` (Axum HTTP server)
- **User:** `mika` (UID 1000), non-root
- **Port:** 8080
- **Healthcheck:** `wget -q --spider http://localhost:8080/health` (10s interval, 5s start period)
- **Config:** Default config copied to `/app/config/default.toml`
- **Data dir:** `/home/mika/.mika` (persistent volume mount point)

### Pushing to Registry

```bash
docker tag mika-agent:dev registry.example.com/mika-agent:v0.1.0
docker push registry.example.com/mika-agent:v0.1.0
```

### Dependency Caching

The Dockerfile uses a dependency caching strategy:

1. Copy only `Cargo.toml`, `Cargo.lock`, and crate manifests.
2. Create dummy source files and build dependencies.
3. Remove workspace crate artifacts but keep dependency cache.
4. Copy real source code and rebuild (only workspace crates recompile).

Rebuilds after source changes are fast because dependency compilation is cached in the Docker layer.

---

## 3b. Building the Gateway Docker Image

```bash
docker build -f Dockerfile.gateway -t mika-gateway:dev .
```

Build details:
- **Builder:** `rust:1.85-slim` with pkg-config, libssl-dev (no gcc — no bundled SQLite)
- **Runtime:** `debian:bookworm-slim` with ca-certificates, wget
- **Binary:** `mika-gateway` (Axum HTTP server)
- **User:** `mika` (UID 1000), non-root, no home directory (stateless)
- **Port:** 8080
- **Healthcheck:** `wget -q --spider http://localhost:8080/readyz` (10s interval, 10s start period)

The gateway image is leaner than the agent image — no GitHub CLI, no file/jq utilities, no home directory. It uses the same dependency caching strategy as the agent image.

---

## 4. Container Deployment

Each customer gets an agent container with a persistent volume for SQLite storage, plus a shared gateway container.

### Required Environment Variables (Agent Container)

| Variable | Description |
|----------|-------------|
| `MIKA_ANTHROPIC_API_KEY` | Anthropic API key or OAuth subscription token |
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
