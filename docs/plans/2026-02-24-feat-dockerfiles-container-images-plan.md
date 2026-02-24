---
title: "feat: Dockerfiles for mika-server and mika-gateway container images"
type: feat
status: completed
date: 2026-02-24
parent: docs/plans/2026-02-24-feat-platform-systems-gateway-provisioning-heartbeat-plan.md
brainstorm: docs/brainstorms/2026-02-24-platform-systems-brainstorm.md
---

# Dockerfiles for mika-server and mika-gateway

## Overview

Create Docker container images for both Mika binaries so they can be built, tested locally, and deployed to AWS EKS. This is a prerequisite for the Helm charts and provisioning pipeline (next step).

**Two images:**
- `mika-agent` — runs `mika-server` binary (per-customer container with SQLite)
- `mika-gateway` — runs `mika-gateway` binary (shared Telegram router with Postgres)

## Problem Statement

All code phases (0-3) are complete with 147 tests passing and 167 review findings resolved. The binaries work locally but cannot be deployed — no container images exist. Without Dockerfiles, the Helm charts and provisioning pipeline have nothing to deploy.

## Proposed Solution

Two separate Dockerfiles with multi-stage builds, dependency layer caching, and optimized release binaries. Plus a `.dockerignore` and release profile optimization.

## Technical Approach

### Key Findings from Research

1. **`-p` flag isolates builds:** `cargo build --release --bin mika-server -p mika-agent` skips sqlx compilation entirely. `cargo build --release --bin mika-gateway -p mika-gateway` skips rusqlite. Each build takes ~22-27s locally.
2. **Binary sizes:** mika-server = 14 MB, mika-gateway = 8.6 MB (without strip/LTO).
3. **rusqlite `bundled` feature** compiles C SQLite source — needs `gcc` + `libc-dev` in the builder stage.
4. **sqlx `migrate!()` macro** embeds SQL files at compile time (reads from disk only) — no `DATABASE_URL` needed at build time. All queries use runtime strings, not compile-time checked macros.
5. **`config/default.toml`** loaded as `required(false)` relative to working directory — include in agent image for visibility.
6. **Home directory bootstrap** (`home.rs`) creates `~/.mika/` structure on first startup — needs writable home dir for the non-root user.

### Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| One or two Dockerfiles | Two separate (`Dockerfile.agent`, `Dockerfile.gateway`) | Different runtime needs (volume vs. no volume, config file vs. none). Clearer than build args. |
| Dependency caching | Workspace Cargo.toml + Cargo.lock copied first, dummy source files, then real source | Avoids recompiling all deps on code changes. ~2min cached vs. ~25min clean. No external tool (cargo-chef) needed. |
| Runtime base image | `debian:bookworm-slim` | Debuggable (can exec into container), includes glibc. ~80MB. Switch to distroless later if needed. |
| Rust version | `rust:1.85-slim` | Minimum for edition 2024. Pin in `rust-toolchain.toml` to match. |
| Non-root user | `mika` (UID 1000) with home directory | Required for security. `useradd -m` creates proper `/home/mika` for `dirs::home_dir()`. |
| MIKA_HOME | `ENV MIKA_HOME=/home/mika/.mika` | Explicit — no reliance on `dirs::home_dir()` resolution. Volume mount at `/home/mika/.mika` persists all state. |
| Volume mount | At `$MIKA_HOME` (whole home dir) | Persists database + soul.md + identity.toml + config. These represent per-customer state that should survive restarts. |
| Release profile | `lto = true`, `codegen-units = 1`, `strip = true` | Reduces binary size (~50%), improves runtime performance. Build time increases ~30% but only affects release builds. |
| Multi-platform | `linux/amd64` only for now | EKS uses x86_64. Add arm64 later if needed (Graviton). |
| HEALTHCHECK | Include in both Dockerfiles | Free, helpful for local `docker run` testing. K8s ignores it (uses its own probes). |
| `rust-toolchain.toml` | Create, pinning `channel = "1.85"` | Ensures local dev and Docker build use the same Rust version. |

### Implementation Phases

---

#### Phase 1: Release Profile + Rust Toolchain Pin

Add optimized release profile to reduce binary size and pin Rust version.

**Edit: `Cargo.toml` (root)**
```toml
[profile.release]
lto = true
codegen-units = 1
strip = true
```

**New: `rust-toolchain.toml`**
```toml
[toolchain]
channel = "1.85"
```

**Verification:**
```bash
cargo build --release --bin mika-server -p mika-agent
cargo build --release --bin mika-gateway -p mika-gateway
ls -lh target/release/mika-server target/release/mika-gateway
# Expect smaller binaries than current 14MB / 8.6MB
```

**Files:**
- Edit: `Cargo.toml`
- New: `rust-toolchain.toml`

---

#### Phase 2: .dockerignore

Prevent secrets, build artifacts, and unnecessary files from entering the build context.

**New: `.dockerignore`**
```
target/
.git/
.env
.env.*
config/local.toml
docs/
todos/
*.md
!crates/**/*.md
.claude/
.github/
```

Note: `!crates/**/*.md` re-includes any `.md` files inside crates that might be needed (none currently, but defensive). The `config/default.toml` is NOT excluded — it's needed by the agent image.

**Files:**
- New: `.dockerignore`

---

#### Phase 3: Dockerfile.agent (mika-server)

Multi-stage build with dependency caching for the per-customer agent container.

**New: `Dockerfile.agent`**
```dockerfile
# === Builder stage ===
FROM rust:1.85-slim AS builder

# Install C compiler for rusqlite bundled SQLite
RUN apt-get update && apt-get install -y gcc libc-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 1. Copy workspace manifests and lockfile (dependency cache layer)
COPY Cargo.toml Cargo.lock ./
COPY crates/mika-common/Cargo.toml crates/mika-common/Cargo.toml
COPY crates/mika-agent/Cargo.toml crates/mika-agent/Cargo.toml
COPY crates/mika-gateway/Cargo.toml crates/mika-gateway/Cargo.toml

# 2. Create dummy source files to compile dependencies
RUN mkdir -p crates/mika-common/src && echo "pub fn _dummy() {}" > crates/mika-common/src/lib.rs \
    && mkdir -p crates/mika-agent/src && echo "fn main() {}" > crates/mika-agent/src/cli.rs \
    && mkdir -p crates/mika-agent/src/bin && echo "fn main() {}" > crates/mika-agent/src/bin/mika-server.rs \
    && echo "" > crates/mika-agent/src/lib.rs \
    && mkdir -p crates/mika-gateway/src && echo "fn main() {}" > crates/mika-gateway/src/main.rs \
    && mkdir -p crates/mika-gateway/migrations && touch crates/mika-gateway/migrations/.keep

# 3. Build dependencies only (this layer is cached until Cargo.toml/lock changes)
RUN cargo build --release --bin mika-server -p mika-agent 2>/dev/null || true
# Clean dummy artifacts but keep compiled dependencies
RUN rm -rf crates/

# 4. Copy real source code
COPY crates/ crates/

# 5. Build the actual binary
RUN cargo build --release --bin mika-server -p mika-agent

# === Runtime stage ===
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user with home directory
RUN useradd -m -u 1000 -s /bin/false mika

# Copy binary
COPY --from=builder /app/target/release/mika-server /usr/local/bin/mika-server

# Copy default config (loaded as optional by Settings::load)
COPY config/default.toml /app/config/default.toml

# Set up home directory structure (bootstrap creates subdirs on first run)
ENV MIKA_HOME=/home/mika/.mika
RUN mkdir -p /home/mika/.mika/data /home/mika/.mika/logs \
    && chown -R mika:mika /home/mika/.mika

WORKDIR /app
USER mika
EXPOSE 8080

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD ["wget", "-q", "--spider", "http://localhost:8080/health"]

CMD ["mika-server"]
```

**Key points:**
- `gcc` + `libc-dev` installed for rusqlite's bundled SQLite C compilation
- Dependency cache layer: Cargo files copied first, dummy sources built, then real source copied
- `-p mika-agent` flag avoids compiling sqlx (gateway's Postgres dependency)
- `wget` included for HEALTHCHECK (smaller than curl, available in bookworm-slim)
- `MIKA_HOME` set explicitly — volume mount at `/home/mika/.mika` in K8s
- `config/default.toml` copied to `/app/config/` since `Settings::load()` reads from `config/default` relative to CWD

**Files:**
- New: `Dockerfile.agent`

---

#### Phase 4: Dockerfile.gateway (mika-gateway)

Multi-stage build for the shared Telegram routing gateway.

**New: `Dockerfile.gateway`**
```dockerfile
# === Builder stage ===
FROM rust:1.85-slim AS builder

WORKDIR /app

# 1. Copy workspace manifests and lockfile (dependency cache layer)
COPY Cargo.toml Cargo.lock ./
COPY crates/mika-common/Cargo.toml crates/mika-common/Cargo.toml
COPY crates/mika-agent/Cargo.toml crates/mika-agent/Cargo.toml
COPY crates/mika-gateway/Cargo.toml crates/mika-gateway/Cargo.toml

# 2. Create dummy source files to compile dependencies
RUN mkdir -p crates/mika-common/src && echo "pub fn _dummy() {}" > crates/mika-common/src/lib.rs \
    && mkdir -p crates/mika-agent/src && echo "fn main() {}" > crates/mika-agent/src/cli.rs \
    && mkdir -p crates/mika-agent/src/bin && echo "fn main() {}" > crates/mika-agent/src/bin/mika-server.rs \
    && echo "" > crates/mika-agent/src/lib.rs \
    && mkdir -p crates/mika-gateway/src && echo "fn main() {}" > crates/mika-gateway/src/main.rs \
    && mkdir -p crates/mika-gateway/migrations && touch crates/mika-gateway/migrations/.keep

# 3. Build dependencies only (cached layer)
RUN cargo build --release --bin mika-gateway -p mika-gateway 2>/dev/null || true
# Clean dummy artifacts but keep compiled dependencies
RUN rm -rf crates/

# 4. Copy real source code
COPY crates/ crates/

# 5. Build the actual binary
RUN cargo build --release --bin mika-gateway -p mika-gateway

# === Runtime stage ===
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user (no home directory needed — gateway is stateless)
RUN useradd -u 1000 -s /bin/false mika

# Copy binary
COPY --from=builder /app/target/release/mika-gateway /usr/local/bin/mika-gateway

USER mika
EXPOSE 8080

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD ["wget", "-q", "--spider", "http://localhost:8080/readyz"]

CMD ["mika-gateway"]
```

**Key points:**
- No `gcc`/`libc-dev` needed — `-p mika-gateway` skips rusqlite compilation
- No config files or volume mounts — gateway is stateless, env-var-only config
- No home directory created — gateway doesn't use `MIKA_HOME`
- HEALTHCHECK hits `/readyz` (the gateway's readiness endpoint, checks Postgres connectivity)
- `start-period=10s` gives time for Postgres connection + migration + webhook registration

**Files:**
- New: `Dockerfile.gateway`

---

#### Phase 5: Local Build + Test Verification

Verify both images build and start correctly.

**Build both images:**
```bash
docker build -f Dockerfile.agent -t mika-agent:dev .
docker build -f Dockerfile.gateway -t mika-gateway:dev .
```

**Verify image sizes:**
```bash
docker images | grep mika
# Expected: ~90-120MB each (debian-slim base + binary + ca-certs)
```

**Test mika-agent locally (smoke test):**
```bash
docker run --rm \
  -e MIKA_ANTHROPIC_API_KEY=sk-ant-test-fake \
  -e MIKA_ROUTING_URL=http://localhost:9999 \
  -e MIKA_INTERNAL_TOKEN=$(openssl rand -hex 32) \
  -p 8080:8080 \
  mika-agent:dev

# In another terminal:
curl http://localhost:8080/health
# Expected: 200 OK
```

**Test mika-gateway locally (requires Postgres):**
```bash
# Start Postgres
docker run -d --name mika-pg -p 5432:5432 \
  -e POSTGRES_PASSWORD=dev -e POSTGRES_DB=mika \
  postgres:16

# Start gateway (will fail at setWebhook without valid Telegram token, but tests DB connection)
docker run --rm --network host \
  -e MIKA_DATABASE_URL=postgres://postgres:dev@localhost:5432/mika \
  -e MIKA_TELEGRAM_BOT_TOKEN=fake:token \
  -e MIKA_TELEGRAM_WEBHOOK_SECRET=$(openssl rand -hex 32) \
  -e MIKA_TELEGRAM_WEBHOOK_URL=https://example.com/webhook/telegram \
  -e MIKA_INTERNAL_TOKEN=$(openssl rand -hex 32) \
  mika-gateway:dev
# Expected: Postgres connects, migrations run, then fails at setWebhook (expected with fake token)
```

**Files:** None (manual verification steps)

---

## Acceptance Criteria

### Functional Requirements

- [x] `docker build -f Dockerfile.agent -t mika-agent:dev .` succeeds
- [x] `docker build -f Dockerfile.gateway -t mika-gateway:dev .` succeeds
- [x] mika-agent container starts, creates `~/.mika/` structure, serves `/health`
- [ ] mika-gateway container starts, connects to Postgres, runs migrations
- [x] Both containers run as non-root user (UID 1000)
- [x] Dependency cache layer works: changing Rust source triggers only app recompile, not full dep rebuild

### Non-Functional Requirements

- [x] `.dockerignore` excludes `.env`, `target/`, `.git/`, `docs/`, `todos/`
- [x] No secrets in any image layer
- [x] Release binaries optimized with LTO + strip
- [x] Image size < 150MB each (agent: 95MB, gateway: 90MB)
- [x] HEALTHCHECK instruction present in both Dockerfiles

### Quality Gates

- [x] `cargo test` still passes after `[profile.release]` and `rust-toolchain.toml` changes
- [x] `cargo clippy` clean
- [x] Both images build from a clean Docker cache

---

## Dependencies & Prerequisites

- Docker installed locally (for building and testing)
- Postgres available for gateway smoke test (`docker run postgres:16`)
- No external infrastructure required (ECR push is a follow-up)

---

## Risk Analysis

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Dependency cache invalidated by Cargo.toml format changes | Low | Medium | Cache layer is mechanical — easy to debug |
| `rust:1.85-slim` missing gcc | Very Low | High | Verified: rust:slim images include gcc/make |
| rusqlite bundled compilation fails in Docker | Low | High | Same as local build — `bundled` feature handles everything |
| `dirs::home_dir()` returns None for non-root user | Low | High | Mitigated: `MIKA_HOME` env var set explicitly, bypasses `dirs::home_dir()` |
| sqlx compile-time check added later breaks gateway build | Low | Medium | Documented: current code uses runtime queries only. Add `.sqlx/` offline cache if compile-time macros are introduced. |

---

## Future Considerations

- **ECR push:** Add `docker push` to ECR as part of CI/CD pipeline
- **Multi-platform:** Add `--platform linux/arm64` via `docker buildx` for Graviton nodes
- **Distroless runtime:** Switch from `debian-slim` to `gcr.io/distroless/cc-debian12` for smaller attack surface
- **GitHub Actions workflow:** `.github/workflows/docker.yml` for automated builds on push
- **`cargo-chef`:** Replace dummy-source technique if workspace grows significantly

---

## References

### Internal
- Parent plan Phase 4: `docs/plans/2026-02-24-feat-platform-systems-gateway-provisioning-heartbeat-plan.md` (lines 1124-1337)
- Brainstorm: `docs/brainstorms/2026-02-24-platform-systems-brainstorm.md`
- Gateway settings: `crates/mika-gateway/src/settings.rs`
- Agent config: `crates/mika-common/src/config.rs`
- Home directory bootstrap: `crates/mika-common/src/home.rs`
- Server entry point: `crates/mika-agent/src/bin/mika-server.rs`
- Gateway entry point: `crates/mika-gateway/src/main.rs`
