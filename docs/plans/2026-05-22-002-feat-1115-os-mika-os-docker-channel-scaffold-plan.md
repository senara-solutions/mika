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

- `Dockerfile.agent` — Debian bookworm-slim, builds `mika-spirit` only, non-root `mika` user (uid 1000), port 8080.
- `Dockerfile.gateway` — Debian bookworm-slim, builds `mika-gateway`, port 8080.
- `docker-compose.yml` — agent + gateway + optional postgres, env from `.env`.
- `skills/bundled/deploy-mika/handlers/run.sh` — expects OpenRC init scripts at `/etc/init.d/mika-spirit` and `/etc/init.d/mika-gateway`, restarts via `sudo rc-service`. **No init scripts are shipped in the repo today** — they're manually provisioned on Vincent's host.
- Runtime: `mika-spirit` reads config from `~/.mika/`, SQLite at `~/.mika/data/mika.db`, logs at `~/.mika/logs/`.

## Deliverables

### D1: `os/Dockerfile` — Functional Gentoo + OpenRC placeholder

Multi-stage build:
1. **Builder stage:** Reuse `rust:1.93-slim` pattern from `Dockerfile.agent`. Build `mika`, `mika-spirit`, `mika-gateway` binaries with `--release --features telemetry`.
2. **Dashboard stage:** Reuse `node:24-slim` pattern from `Dockerfile.agent` for dashboard build.
3. **Runtime stage:** `gentoo/stage3:20250505` base (pinned; see F3 resolution below).
   - Ensure OpenRC is functional (`/sbin/openrc` + `/etc/init.d/`).
   - Install runtime deps: `ca-certificates`, `wget`, `jq`, `sqlite` (for debugging).
   - Install `gh` CLI (same pattern as `Dockerfile.agent` lines 39-46).
   - Create `mika` user (uid 1000) with home directory.
   - Copy binaries from builder.
   - Copy OpenRC service definitions from `os/openrc/`.
   - Copy default configs from `os/config/`.
   - Install `ollama` binary via official install script pinned to v0.6.0 (binary only; no models pulled at build time — see F4 resolution below).
   - Set `MIKA_HOME=/home/mika/.mika`.
   - `ENTRYPOINT ["/usr/local/bin/mika-os-init.sh"]` — wrapper script that boots OpenRC and supervises (see "F2 — Entrypoint lifecycle contract" below).

**F1 resolution — process supervision strategy:** **Full OpenRC** (operator decision 2026-05-22).

- Aligns with Vincent's Gentoo desktop service model (host parity).
- `deploy-mika` skill's `run.sh` already expects `/etc/init.d/mika-spirit` + `/etc/init.d/mika-gateway` via `sudo rc-service`. OpenRC inside the image closes the gap directly.
- Trade-off accepted: container runtime requires `--cap-add=SYS_ADMIN` (preferred over full `--privileged`) and `--security-opt apparmor=unconfined` on hosts with AppArmor. Document required runtime flags in `os/README.md` (D2).

**F2 resolution — Entrypoint lifecycle contract:**

`os/init/mika-os-init.sh` (entrypoint wrapper) provides:

1. **Boot order**: `openrc-init` boots → invokes runlevel via `rc default` → OpenRC dependency graph orchestrates: `net` → `mika-spirit` → `mika-gateway` (gateway depends on mika-spirit per `os/openrc/init.d/mika-gateway`).
2. **SIGTERM propagation**: entrypoint installs `trap` handler that runs `rc-service mika-gateway stop` then `rc-service mika-spirit stop` (reverse dependency order), then `exit 0`. Graceful drain via OpenRC's `supervise-daemon` `--retry SIGTERM/10/SIGKILL/15` policy.
3. **Child-exit policy**: `supervise-daemon` in `os/openrc/init.d/mika-spirit` + `mika-gateway` configured with `respawn_delay=5 respawn_max=10 respawn_period=60` (matches Vincent's host pattern per memory `project_gentoo_openrc_services`). If respawn-max trips, supervise-daemon exits → container exits → Docker restart policy applies.
4. **Healthcheck**: `HEALTHCHECK CMD rc-service mika-spirit status` in Dockerfile. OpenRC `rc-service status` returns 0 on running, non-zero otherwise — straight Docker-compatible.
5. **Container stays alive**: after `rc default` completes, entrypoint runs `tail -F /home/mika/.mika/logs/mika-spirit.log /home/mika/.mika/logs/mika-gateway.log &` (background) then `wait $!` so the trap remains active. Do NOT use `exec tail -F` — `exec` replaces the shell and discards the trap, breaking SIGTERM propagation (architect catch, second-pass review). Alternative considered + rejected for the placeholder: hand PID 1 to `openrc-init` via `exec /sbin/openrc-init` — cleaner but defers SIGTERM-to-rc-service translation to openrc-init's own handler, which is less audit-friendly for the placeholder; revisit in audit-derived recipe.

**Key decisions:**
- **gentoo/stage3 pinned (F3):** `gentoo/stage3:20250505` (recent stable tag). The audit-derived recipe in the followup ticket will revisit pinning policy (digest pin vs tag pin).
- **No model baking:** GGUF weights are NOT baked into the image. They're either volume-mounted or pulled by ollama on first start. This keeps the image under control (~200-300MB without models vs multi-GB with).
- **All three binaries:** Unlike `Dockerfile.agent` (mika-spirit only), Mika OS ships `mika` (TUI), `mika-spirit`, and `mika-gateway` — it's a complete runtime.
- **Pre-audit marker:** Add `# PRE-AUDIT PLACEHOLDER` header comment and a `LABEL mika.audit-status="pre-audit"` to make the placeholder status machine-readable.
- **Ollama stub shape (F4):** placeholder ollama install via official script, version-pinned to `v0.6.0`. NO `serve` daemon registered as OpenRC service in the placeholder — defer to audit-derived recipe. `os/README.md` documents `docker run -v ollama-models:/root/.ollama mika-os ollama pull <model>` as the runtime model-pull path.

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

**`os/openrc/init.d/mika-spirit`** — OpenRC init script:
- `command="/usr/local/bin/mika-spirit"`
- `command_background="yes"` + `pidfile`
- `supervise_daemon_args` for automatic restart
- Depends on: `net` (network must be up)
- Start-stop-daemon with `--user mika`
- Logging: stdout/stderr to `/home/mika/.mika/logs/mika-spirit.log`
- Environment sourced from `/etc/conf.d/mika-spirit`

**`os/openrc/conf.d/mika-spirit`** — Configuration:
- `MIKA_HOME="/home/mika/.mika"`
- `MIKA_LOG_FORMAT="json"`
- `MIKA_SPIRIT_LOG_FILE="/home/mika/.mika/logs/mika-spirit.log"`
- Commented-out provider keys (runtime-only, never baked)
- `MIKA_DEV_MODE="false"` (production default)

**`os/openrc/init.d/mika-gateway`** — OpenRC init script:
- Same pattern as mika-spirit
- Depends on: `net`, `mika-spirit` (gateway routes to agent)
- `command="/usr/local/bin/mika-gateway"`

**`os/openrc/conf.d/mika-gateway`** — Configuration:
- `MIKA_DATABASE_URL` (Postgres connection string, commented out — requires external DB or embedded)
- `MIKA_INTERNAL_TOKEN` (shared secret, must match mika-spirit's)
- Commented-out Telegram bot token

**Design note:** These init scripts also close the gap identified in `deploy-mika` skill — `run.sh` expects `/etc/init.d/mika-spirit` and `/etc/init.d/mika-gateway` but nothing in the repo provides them. The Dockerfile copies these into place, and they could also be installed on bare-metal Gentoo hosts.

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

4. Create `os/openrc/init.d/mika-spirit` — supervise-daemon init script.
5. Create `os/openrc/conf.d/mika-spirit` — environment configuration.
6. Create `os/openrc/init.d/mika-gateway` — supervise-daemon init script.
7. Create `os/openrc/conf.d/mika-gateway` — environment configuration.

**Files created:** 4 files in `os/openrc/`

### Phase 3: Default config (D4)

8. Create `os/config/config.toml` — server config defaults.
9. Create `os/config/mika.env` — template env file.

**Files created:** 2 files in `os/config/`

### Phase 4: Dockerfile + entrypoint (D1 + F2)

10. Create `os/init/mika-os-init.sh` — entrypoint wrapper per F2 contract (trap → `openrc-init` → `rc default` → `exec tail -F`). Executable mode 0755.
11. Create `os/Dockerfile` — multi-stage Gentoo + OpenRC build. `ENTRYPOINT ["/usr/local/bin/mika-os-init.sh"]`. `HEALTHCHECK CMD rc-service mika-spirit status`.
12. Verify it builds: `docker build -t mika-os:dev -f os/Dockerfile .` (from repo root).

**Files created:** `os/Dockerfile`, `os/init/mika-os-init.sh`

### Phase 5: Validation

12. Verify `docker build` succeeds (image builds without errors).
13. Verify `docker run --rm mika-os:dev mika-spirit --version` prints version.
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
2. **OpenRC runtime requirement (resolved per F1):** Container requires `--cap-add=SYS_ADMIN` (preferred over `--privileged`) and AppArmor unconfined on hosts where AppArmor is enforcing. `os/README.md` documents the required `docker run` flags. Helm chart cutover (future ticket) must set equivalent `securityContext.capabilities.add: ["SYS_ADMIN"]`.
3. **Builder stage compatibility:** The Rust builder stage uses Debian-based `rust:1.93-slim`. Cross-compiling for Gentoo runtime should work (both are glibc Linux), but static linking via `RUSTFLAGS="-C target-feature=+crt-static"` would be safer. However, SQLite bundled compilation should handle this — the existing `Dockerfile.agent` pattern works.
4. **PID 1 in container under OpenRC:** Docker's default expectation is PID 1 = the entrypoint binary. With OpenRC, `openrc-init` becomes PID 1 (via the entrypoint script's exec). Zombie reaping is OpenRC's responsibility. Validated by F2 entrypoint contract.

## Acceptance Criteria (from issue)

- AC1: `os/Dockerfile` exists, is marked pre-audit, and is functional enough to build and run.
- AC2: `os/README.md` documents what Mika OS is, the three audiences, and Apache 2.0 license.
- AC3: `os/openrc/` contains supervise-daemon + conf.d service definitions for mika-spirit and mika-gateway.
- AC4: `os/config/` contains opinionated default configs.
- AC5: No secrets baked in — all secrets are runtime/deploy-time only.
- AC6: All files carry Apache 2.0 license headers.
