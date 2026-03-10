---
title: "Docker Build Refactoring: BuildKit Cache Mounts Replace Dummy-Source Layer Caching"
date: 2026-03-10
status: documented
category: architecture-patterns
tags:
  - docker
  - buildkit
  - cache-mounts
  - docker-compose
  - dockerfile
  - infrastructure
  - local-development
modules:
  - Dockerfile.agent
  - Dockerfile.gateway
  - docker-compose.yml
  - mika-cli (setup --mode compose)
  - mika-gateway (settings.rs error message)
severity: medium
symptoms:
  - Complex multi-step Dockerfile with dummy source files for dependency caching
  - Fragile artifact cleanup logic that could break on workspace changes
  - No docker-compose.yml for local multi-service development
  - No guided setup for compose environment variables
---

# Docker BuildKit Cache Mounts and Compose

## Problem

Both Dockerfiles used a **dummy-source dependency caching pattern** that was fragile and
verbose:

1. Copy only `Cargo.toml` manifests and lockfile
2. Create stub `lib.rs`/`main.rs` files for every workspace crate
3. Run `cargo build --release` to compile dependencies into a Docker layer
4. Delete workspace artifacts while preserving dependency cache (careful `rm` commands)
5. Copy real source code
6. Rebuild — only workspace crates recompile (in theory)

**Problems:**
- **Fragile cleanup:** The `rm` commands required exact knowledge of which fingerprints
  and artifacts to delete. Missing a pattern caused full rebuilds; deleting too much
  wiped the dependency cache.
- **Verbose:** 30+ lines per Dockerfile with dummy file creation and multi-step cleanup.
- **Maintenance burden:** Any change to workspace structure (new crate, renamed binary)
  required updating the cleanup logic.
- **Extra layers:** Multiple intermediate layers persisted in the image for orchestration.

Additionally, there was no `docker-compose.yml` in the repository and no guided way to
generate the required environment variables for multi-service deployment.

## Root Cause

The dummy-source pattern was the standard Rust Docker caching approach before BuildKit
cache mounts became widely available. It worked but fought against Docker's layer model
rather than using purpose-built caching primitives.

## Solution

### 1. BuildKit Cache Mounts

Replaced the entire dummy-source pattern with two `--mount=type=cache` directives:

```dockerfile
# Copy workspace manifests, lockfile, and source code
COPY Cargo.toml Cargo.lock ./
COPY crates/mika-common/ crates/mika-common/
COPY crates/mika-agent/ crates/mika-agent/

# Build with BuildKit cache mounts for incremental compilation
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release --bin mika-server -p mika-agent \
    && cp target/release/mika-server /usr/local/bin/mika-server
```

**Key details:**
- `--mount=type=cache,target=/app/target` — Cargo build output (incremental compilation)
- `--mount=type=cache,target=/usr/local/cargo/registry` — Downloaded crate sources
- Binary is copied to `/usr/local/bin/` *within* the RUN command because cache mounts
  are external to image layers — they're never committed
- Source code is copied upfront (no dummy files needed); cache mounts handle incremental
  compilation automatically
- Minimal stubs still needed for workspace members not being built (e.g., gateway stub
  in agent Dockerfile) to satisfy Cargo workspace resolution

### 2. docker-compose.yml

New file defining three services for local development:

| Service | Image | Port | Notes |
|---------|-------|------|-------|
| `agent` | `Dockerfile.agent` | 8081:8080 | Persistent volume at `/home/mika/.mika` |
| `gateway` | `Dockerfile.gateway` | 8080:8080 | Stateless, reads `.env` from CWD |
| `postgres` | `postgres:17-alpine` | 5433:5432 | Optional, requires `--profile db` |

Design decisions:
- Agent depends on gateway health (`condition: service_healthy`)
- Gateway's postgres dependency is optional (`required: false`) — starts without DB
- Both services use `env_file: .env` for configuration
- Named volumes (`agent-data`, `pg-data`) persist data across restarts

### 3. `mika setup --mode compose`

Interactive wizard that generates a `.env` file in the current directory:

**Prompted (masked input):**
- Anthropic API key (required)
- Telegram bot token and webhook URL (required for gateway)
- Brave Search and OpenAI API keys (optional)

**Auto-generated (64-char hex tokens):**
- `MIKA_INTERNAL_TOKEN` — gateway-to-agent auth
- `MIKA_TELEGRAM_WEBHOOK_SECRET` — webhook signature validation
- `MIKA_DASHBOARD_TOKEN` — read-only dashboard API access

**Hardcoded for compose networking:**
- `MIKA_ROUTING_URL=http://gateway:8080`
- `MIKA_DATABASE_URL=postgres://mika:mika@postgres:5432/mika`

TTY guard rejects non-interactive terminals. Overwrite confirmation if `.env` exists.

### 4. Gateway Error Message Improvement

`GatewaySettings::load()` now suggests the setup command on failure:

```
Failed to load gateway settings: {e}.
Run `mika setup --mode compose` to generate a .env file,
or set the required MIKA_* env vars directly.
```

## Key Insights

- **Use native primitives over workarounds.** BuildKit cache mounts are the Docker-native
  solution for build caching. The dummy-source pattern was a workaround that predated
  cache mounts. When the platform provides a mechanism, use it.

- **Cache mounts are not layers.** They don't appear in `docker history` and aren't
  cleared by `--no-cache`. Use `docker builder prune` to clear mount caches. This is a
  feature (builds always work cold) but can confuse debugging.

- **Copy the binary out during the build step.** Since `/app/target` is a cache mount
  (never committed to a layer), the binary must be copied to a non-mounted path within
  the same RUN command. `COPY --from=builder /app/target/...` would fail silently.

- **Stubs are still needed for unbuilt workspace members.** Cargo workspace resolution
  requires all members to be parseable. The stubs are trivial (`fn main() {}`) and don't
  participate in caching — they just satisfy the resolver.

- **Guided setup reduces friction.** `mika setup --mode compose` eliminates the most
  common deployment failure (missing or misconfigured env vars) by generating a complete
  `.env` from interactive prompts.

## Prevention Strategies

1. **Lint Dockerfiles in CI.** Add `hadolint` to catch anti-patterns (redundant layers,
   unpinned packages, missing `--no-install-recommends`).

2. **Test cold and warm builds.** After Dockerfile changes, verify: (a) cold build
   succeeds (`docker builder prune -af` first), (b) warm rebuild after touching a `.rs`
   file is significantly faster than cold.

3. **Keep `.dockerignore` in sync.** New top-level files (`.env`, `docker-compose.yml`)
   should be excluded from build context to prevent cache busting.

4. **Use distinct cache IDs if sharing base images.** If agent and gateway Dockerfiles
   share the same BuildKit cache for `/app/target` but compile different features, they'll
   thrash each other. Use `--mount=type=cache,id=mika-agent,target=/app/target`.

5. **CI runners need explicit cache persistence.** GitHub Actions ephemeral runners lose
   BuildKit caches between runs. Use `actions/cache` or the GHA cache backend
   (`cache-from`/`cache-to`) for CI speed.

## Testing Checklist

- [ ] Cold build: `docker builder prune -af && docker build -f Dockerfile.agent -t test .`
- [ ] Warm build: touch a `.rs` file, rebuild — registry cache reused
- [ ] Final image has no build tools (`rustc`, `cargo` absent)
- [ ] Image size: agent ~95MB, gateway smaller
- [ ] Non-root: `docker run --rm mika-agent:test whoami` prints `mika`
- [ ] `docker compose up` with `.env` from `mika setup --mode compose` — all healthy
- [ ] `docker compose --profile db up` starts postgres
- [ ] Missing `.env` produces clear error, not silent empty-string substitution

## Files Changed

| File | Change | Rationale |
|------|--------|-----------|
| `Dockerfile.agent` | Replaced dummy-source pattern with cache mounts | Simpler, faster, more reliable |
| `Dockerfile.gateway` | Same refactoring as agent | Consistency |
| `docker-compose.yml` | **New** — agent, gateway, postgres services | Local multi-service development |
| `crates/mika-cli/src/commands/setup.rs` | Added `run_compose_generation()` | Interactive .env wizard for compose |
| `crates/mika-cli/src/cli.rs` | Added `SetupMode::Compose` variant | CLI flag support |
| `crates/mika-gateway/src/settings.rs` | Improved error message | Guides operators to setup command |
| `.dockerignore` | Added `docker-compose.yml`, removed `config/local.toml` | Build context hygiene |

## Related Documentation

- [Simplified Config 4-Source Model](simplified-config-4-source-model.md) — Parent feature: dotenvy `.env` support that this compose work builds on
- [Deployment Guide](../../deployment.md) — BuildKit cache mounts (Section 3), docker-compose (Section 3d)
- [Configuration Reference](../../configuration.md) — `.env` secrets handling and cascade
- [Getting Started](../../getting-started.md) — `mika setup --mode` documentation
- Todo #611: Compose `.env` write lacks atomic write pattern (pending, P3)
