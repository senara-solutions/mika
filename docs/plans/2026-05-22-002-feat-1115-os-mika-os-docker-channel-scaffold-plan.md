# Plan: Scaffold mika/os/ — Mika OS Docker Channel (Pre-Audit Placeholder)

**Issue:** mika#1115
**Type:** feat
**Branch:** `feat/1115/os-mika-os-docker-channel-scaffold-mika`
**Date:** 2026-05-22

## Context

Mika OS is the Docker distribution channel — a complete runtime image (Gentoo base + OpenRC + mika binaries + ollama) serving three audiences:

1. Per-customer container base for mika-cloud Helm deployments (replaces current `Dockerfile.agent` Debian-based image long-term).
2. Single-box self-host path (`docker run mika-os`).
3. Forkable reference environment for developers building on Mika.

This ticket scaffolds the `mika/os/` directory with a **functional pre-audit placeholder** — enough to build and run, but the final recipe will be derived from Vincent's audited Gentoo desktop configuration in a follow-up ticket.

## Existing Infrastructure

- `Dockerfile.agent` — Debian bookworm-slim, builds `mika-server` only, non-root `mika` user (uid 1000), port 8080.
- `Dockerfile.gateway` — Debian bookworm-slim, builds `mika-gateway`, port 8080.
- `docker-compose.yml` — agent + gateway + optional postgres, env from `.env`.
- `skills/bundled/deploy-mika/handlers/run.sh` — expects OpenRC init scripts at `/etc/init.d/mika-server` and `/etc/init.d/mika-gateway`, restarts via `sudo rc-service`. **No init scripts are shipped in the repo today** — they're manually provisioned on Vincent's host.
- Runtime: `mika-server` reads config from `~/.mika/`, SQLite at `~/.mika/data/mika.db`, logs at `~/.mika/logs/`.

## Deliverables

### D1: `os/Dockerfile` — Functional Gentoo + OpenRC placeholder

Multi-stage build:
1. **Builder stage:** Reuse `rust:1.93-slim` pattern from `Dockerfile.agent`. Build `mika`, `mika-server`, `mika-gateway` binaries with `--release --features telemetry`.
2. **Dashboard stage:** Reuse `node:24-slim` pattern from `Dockerfile.agent` for dashboard build.
3. **Runtime stage:** `gentoo/stage3:latest` base.
   - Install OpenRC (already in stage3, just ensure `/sbin/openrc` is functional).
   - Install runtime deps: `ca-certificates`, `wget`, `jq`, `sqlite` (for debugging).
   - Install `gh` CLI (same pattern as `Dockerfile.agent` lines 39-46).
   - Create `mika` user (uid 1000) with home directory.
   - Copy binaries from builder.
   - Copy OpenRC service definitions from `os/openrc/`.
   - Copy default configs from `os/config/`.
   - Install ollama binary (stub: download script placeholder, not a full model pull — models are runtime-mounted or pulled on first start).
   - Set `MIKA_HOME=/home/mika/.mika`.
   - `ENTRYPOINT` runs OpenRC init to start services, then tails logs (container stays alive via OpenRC supervision).

**Key decisions:**
- **gentoo/stage3 vs custom stage3:** Use official `gentoo/stage3:latest` for the placeholder. The audit-derived recipe will likely pin a specific stage3 variant.
- **No model baking:** GGUF weights are NOT baked into the image. They're either volume-mounted or pulled by ollama on first start. This keeps the image under control (~200-300MB without models vs multi-GB with).
- **All three binaries:** Unlike `Dockerfile.agent` (mika-server only), Mika OS ships `mika` (TUI), `mika-server`, and `mika-gateway` — it's a complete runtime.
- **Pre-audit marker:** Add `# PRE-AUDIT PLACEHOLDER` header comment and a `LABEL mika.audit-status="pre-audit"` to make the placeholder status machine-readable.

### D2: `os/README.md` — Documentation

Contents:
- What Mika OS is (one paragraph).
- Three audiences (mika-cloud base, self-host, developer reference).
- Quick start: `docker build -t mika-os -f os/Dockerfile .` and `docker run -v mika-data:/home/mika/.mika mika-os`.
- Configuration: env vars, volume mounts, config file locations.
- Pre-audit status notice: this is a placeholder, final recipe pending audit.
- License: Apache 2.0.
- Pointer to strategic frame (the OSS decision).

### D3: `os/openrc/` — Service definitions

Three files per service pattern (following Gentoo OpenRC conventions):

**`os/openrc/init.d/mika-server`** — OpenRC init script:
- `command="/usr/local/bin/mika-server"`
- `command_background="yes"` + `pidfile`
- `supervise_daemon_args` for automatic restart
- Depends on: `net` (network must be up)
- Start-stop-daemon with `--user mika`
- Logging: stdout/stderr to `/home/mika/.mika/logs/mika-server.log`
- Environment sourced from `/etc/conf.d/mika-server`

**`os/openrc/conf.d/mika-server`** — Configuration:
- `MIKA_HOME="/home/mika/.mika"`
- `MIKA_LOG_FORMAT="json"`
- `MIKA_SERVER_LOG_FILE="/home/mika/.mika/logs/mika-server.log"`
- Commented-out provider keys (runtime-only, never baked)
- `MIKA_DEV_MODE="false"` (production default)

**`os/openrc/init.d/mika-gateway`** — OpenRC init script:
- Same pattern as mika-server
- Depends on: `net`, `mika-server` (gateway routes to agent)
- `command="/usr/local/bin/mika-gateway"`

**`os/openrc/conf.d/mika-gateway`** — Configuration:
- `MIKA_DATABASE_URL` (Postgres connection string, commented out — requires external DB or embedded)
- `MIKA_INTERNAL_TOKEN` (shared secret, must match mika-server's)
- Commented-out Telegram bot token

**Design note:** These init scripts also close the gap identified in `deploy-mika` skill — `run.sh` expects `/etc/init.d/mika-server` and `/etc/init.d/mika-gateway` but nothing in the repo provides them. The Dockerfile copies these into place, and they could also be installed on bare-metal Gentoo hosts.

### D4: `os/config/` — Default configuration

**`os/config/config.toml`** — Mika server config defaults:
- Provider: not set (user must configure at runtime)
- Model: not set
- Log format: json
- Server port: 8080
- KG docs root: `/app/docs/solutions` (container working directory)

**`os/config/mika.env`** — Template `.env` file for Mika OS:
- All `MIKA_*` env vars from `.env.example`, with comments
- Clearly marked: "copy to /home/mika/.mika/.env and edit"
- No secrets populated

### D5: License header

All files in `os/` include Apache 2.0 SPDX header: `# SPDX-License-Identifier: Apache-2.0`

## Implementation Phases

### Phase 1: Directory structure + README (D2, D5)

1. Create `os/` directory.
2. Write `os/README.md` with all sections from D2.
3. Apache 2.0 headers on all files.

**Files created:** `os/README.md`

### Phase 2: OpenRC service definitions (D3)

4. Create `os/openrc/init.d/mika-server` — supervise-daemon init script.
5. Create `os/openrc/conf.d/mika-server` — environment configuration.
6. Create `os/openrc/init.d/mika-gateway` — supervise-daemon init script.
7. Create `os/openrc/conf.d/mika-gateway` — environment configuration.

**Files created:** 4 files in `os/openrc/`

### Phase 3: Default config (D4)

8. Create `os/config/config.toml` — server config defaults.
9. Create `os/config/mika.env` — template env file.

**Files created:** 2 files in `os/config/`

### Phase 4: Dockerfile (D1)

10. Create `os/Dockerfile` — multi-stage Gentoo + OpenRC build.
11. Verify it builds: `docker build -t mika-os:dev -f os/Dockerfile .` (from repo root).

**Files created:** `os/Dockerfile`

### Phase 5: Validation

12. Verify `docker build` succeeds (image builds without errors).
13. Verify `docker run --rm mika-os:dev mika-server --version` prints version.
14. Verify OpenRC service files have correct permissions (executable init scripts).

## Out of Scope

Per issue body — these are separate follow-up tickets:
- Audit-derived final Dockerfile (replaces the placeholder).
- Multi-arch build matrix + registry publish CI.
- mika-cloud Helm chart cutover to Mika OS base image.
- Ollama model pulling/serving configuration.
- Docker Compose integration for Mika OS (separate from existing `docker-compose.yml`).

## Risks

1. **gentoo/stage3 image size:** Stage3 images are ~700MB+. The placeholder will be significantly larger than the current ~95MB Debian agent image. The audit-derived recipe will slim this down via selective package removal.
2. **OpenRC in Docker:** Running OpenRC inside Docker requires `--privileged` or specific capabilities (`SYS_ADMIN`). The placeholder should document this requirement. Alternative: use a simpler supervisor (e.g., `s6-overlay` or a shell entrypoint that starts services directly) and reserve full OpenRC for the audited recipe.
3. **Builder stage compatibility:** The Rust builder stage uses Debian-based `rust:1.93-slim`. Cross-compiling for Gentoo runtime should work (both are glibc Linux), but static linking via `RUSTFLAGS="-C target-feature=+crt-static"` would be safer. However, SQLite bundled compilation should handle this — the existing `Dockerfile.agent` pattern works.

## Acceptance Criteria (from issue)

- AC1: `os/Dockerfile` exists, is marked pre-audit, and is functional enough to build and run.
- AC2: `os/README.md` documents what Mika OS is, the three audiences, and Apache 2.0 license.
- AC3: `os/openrc/` contains supervise-daemon + conf.d service definitions for mika-server and mika-gateway.
- AC4: `os/config/` contains opinionated default configs.
- AC5: No secrets baked in — all secrets are runtime/deploy-time only.
- AC6: All files carry Apache 2.0 license headers.
