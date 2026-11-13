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

Two GitHub Actions workflows test the code and distribute binaries. Versioning is manual: the version
and the changelog move in the release PR, and the operator pushes the tag.

### CI (`ci.yml`)

Runs on every PR and push to `main`:
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

### Release Binaries (`release.yml`)

The release path. Operator steps — dry run, launch, verification checklist, rollback — live in
[`docs/releasing.md`](releasing.md).

- **Tag push `vX.Y.Z` or `vX.Y.Z-rc.N`** (on a commit of `main`, base equal to the workspace version,
  `docs/releases/vX.Y.Z.md` present): builds, signs, then publishes a release. A `-rc.N` tag is
  published as a **prerelease** and never becomes `latest`, which is what `install.sh` installs.
- **Manual run** (`workflow_dispatch`, from `main` only): the same files as a workflow artifact,
  nothing published.
- Builds `mika` (CLI) and `mika-spirit` (HTTP server) with `--features telemetry` for x86_64-linux,
  aarch64-linux, x86_64-macos (cross-compiled on an arm64 runner) and aarch64-macos.
- Each archive comes with a `.tar.gz.sha256` checksum, a cosign `.bundle` and a build provenance
  attestation; each binary has a CycloneDX SBOM. See [`docs/release-verification.md`](release-verification.md).
- `mika-gateway` excluded — deployed via Docker to Kubernetes, not as a standalone binary.
- Dashboard assets are not embedded in release builds (empty placeholder); the `/dashboard/` endpoint shows a branded "disabled" page.

**release-please is gone (mika#2890).** `release-pr.yml` failed on every merge between 2026-06-05 and
2026-08-29 (300 runs, 300 failures): its `rust` strategy cannot handle our virtual workspace — it does
not expand `members = ["crates/*"]`, then hands the root `Cargo.toml`, which has no `[package]`, to a
package updater and throws (diagnosis mika#2047). It was disabled, the resume ticket mika#2048 was
closed not planned, and its only remaining trigger would have opened a real release PR on `main`. The
workflow and its two config files were removed. History of this failure class:
`docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`.

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

## 4b. Schema Migrations and File Rewrites (mika#2735)

### May agent processes hold the database during a migration?

**Today, yes — because no migration rewrites the file any more.** Since
mika#2735 there is no `VACUUM` anywhere on the startup path, and the
`migrate_vN_to_vM` ladder only does DDL and row updates inside a transaction.
Those are safe under concurrency: SQLite serialises writers, and other
connections see the new schema on their next statement. You do not need to
sequence process startup for a migration of that shape.

**For a migration that rewrites the file, no — and this is the order.** A
rewriting operation is one that relocates pages or changes the file header:
`VACUUM`, a full table rebuild (the v28→v29 idiom), or anything that converts
`auto_vacuum`. For those:

1. Stop every process that opens `~/.mika/data/mika.db` — `mika-spirit` first,
   then any `mika` CLI invocation still running.
2. Confirm nothing holds it: `pgrep -fa 'mika '` must return nothing, and
   `ls ~/.mika/data/mika.db-wal` should be absent or zero-length after a clean
   shutdown.
3. Run the rewriting step from a **single** connection.
4. Verify: `sqlite3 ~/.mika/data/mika.db 'PRAGMA quick_check'` must print `ok`.
5. Start `mika-spirit`. Agent processes may start in any order after that —
   they are ordinary readers and writers once the rewrite is done.

The reason the order matters is the 2026-10-16 incident: the v55→v56 migration
ran a `VACUUM` at `mika-spirit` startup while the engine tick and one connection
per agent were live. `quick_check` was `ok` at 09:05Z and reported 25 damaged
trees by 11:00Z, with `Bad ptr map entry` and `2nd reference to page`.

### Why that `VACUUM` was not just a compaction

`mika.db` was created with `auto_vacuum=NONE`. `Database::open` issues
`PRAGMA auto_vacuum = INCREMENTAL`, which on a non-empty database is
**inoperative** — it stays pending until the next `VACUUM`. So that `VACUUM` did
not only move pages; it applied the pending setting and **converted the file
format**, adding *pointer-map* pages. Both error signatures above name that
structure. Replaying v56 *without* the `VACUUM` on the v55 backup takes 6 s and
leaves `quick_check` at `ok`.

The pending `PRAGMA` is deliberately left in place — removing it is out of
mika#2735's scope. It is harmless as long as nothing runs a `VACUUM` under
concurrency, which is exactly what the guard enforces.

### Compacting the database — an operator gesture, engine stopped

Nothing compacts `mika.db` automatically any more. When the file has grown past
what its live data justifies (after a migration that dropped a column, or after
`audit_events` compaction), do it by hand:

```bash
# 1. Stop the engine and confirm nothing holds the file.
#    (systemd: systemctl stop mika-spirit — OpenRC: rc-service mika-spirit stop)
pgrep -fa 'mika '          # must return nothing

# 2. Back up first — a VACUUM rewrites the whole file.
cp ~/.mika/data/mika.db ~/.mika/data/mika.db.bak-$(date -u +%Y%m%dT%H%M%SZ)

# 3. Compact, then verify. Both lines must print `ok`.
sqlite3 ~/.mika/data/mika.db 'VACUUM;'
sqlite3 ~/.mika/data/mika.db 'PRAGMA quick_check'
sqlite3 ~/.mika/data/mika.db 'PRAGMA integrity_check'

# 4. Restart the engine.
```

If step 3 does not print `ok`, restore the backup from step 2 and escalate — do
not start the engine on a file that failed the check. `sqlite3 .recover` on a
copy is the recovery path (it produced a clean file on 2026-10-16).

A `mika db vacuum` command that takes `PRAGMA locking_mode=EXCLUSIVE` plus
`BEGIN EXCLUSIVE` and **refuses**, naming the reason, when another connection
holds the file, is tracked as flight 2 of mika#2735. Until it ships, the manual
gesture above is the only supported one.

### The size warning now insists, and that is expected

`startup_cleanup` logs `database size exceeds threshold` (field
`threshold_bytes`, constant `DB_SIZE_WARN_BYTES` in `server/mod.rs`) when the
file is larger than its threshold. Before mika#2735 a `VACUUM` could clear that
condition on its own; now nothing does. **So the warning fires on every startup
until an operator performs the gesture above.** It is a standing signal about
file size, not a transient one — reading it as "something is failing right now"
is the misreading to avoid. Its accepted cost is the operator's attention; the
alternative cost was a damaged database.

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
