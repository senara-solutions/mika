<!-- SPDX-License-Identifier: Apache-2.0 -->

# Mika OS — Docker Distribution Channel

Mika OS is a complete runtime image (Gentoo base + OpenRC + mika binaries + ollama) that packages the entire Mika AI executive assistant into a single Docker image.

## Build Targets

Two named multi-stage targets serve different audiences:

| Target | Purpose | Includes portage tree | Size |
|--------|---------|----------------------|------|
| `mika-os` | Developer reference, single-box self-host | Yes (can `emerge` post-build) | ~1.5GB+ |
| `mika-runtime` | Production deployment (mika-cloud Helm) | No (minimal attack surface) | ~800MB+ |

## Audiences

1. **mika-cloud base image** — Use `mika-runtime` as per-customer container base for Helm deployments.
2. **Single-box self-host** — Use either target. `mika-runtime` is smaller; `mika-os` has developer tools.
3. **Developer reference** — Use `mika-os` for a forkable environment with full Gentoo toolchain (Rust, Node.js, gcc, portage tree).

## Quick Start

### Build

```bash
# From the mika repo root (requires BuildKit for per-Dockerfile .dockerignore)

# Production runtime (recommended)
docker build --target mika-runtime -f os/Dockerfile -t mika-runtime:dev .

# Full developer image
docker build --target mika-os -f os/Dockerfile -t mika-os:dev .
```

### Run

```bash
docker run -d \
  --name mika \
  --cap-add=SYS_ADMIN \
  --security-opt apparmor=unconfined \
  -v mika-data:/home/mika/.mika \
  -p 8080:8080 \
  -e MIKA_ANTHROPIC_API_KEY=sk-ant-... \
  mika-runtime:dev
```

**Required runtime flags:**

- `--cap-add=SYS_ADMIN` — Required for OpenRC process supervision inside the container. Preferred over `--privileged`.
- `--security-opt apparmor=unconfined` — Required on hosts with AppArmor enforcing (Ubuntu, Debian). Not needed on hosts without AppArmor (Gentoo, Alpine, most minimal container hosts).

### Verify

```bash
# Check services are running
docker exec mika rc-service mika-server status
docker exec mika rc-service mika-gateway status

# Check version
docker exec mika mika-server --version
```

## Configuration

### Default Configs

Default configuration files are installed to `/etc/mika/` (FHS-compliant) at build time. On first boot, the entrypoint copies them to `$MIKA_HOME` if not already present — this preserves operator customizations on volume restarts.

| Source (image) | Destination (runtime) | Purpose |
|----------------|----------------------|---------|
| `/etc/mika/config.toml` | `$MIKA_HOME/config.toml` | Server configuration |
| `/etc/mika/mika.env.template` | `$MIKA_HOME/.env.template` | Env var template (copy to `.env` and edit) |

### Environment Variables

All `MIKA_*` environment variables are supported. Pass them via `docker run -e` or mount an env file:

```bash
docker run -d \
  --cap-add=SYS_ADMIN \
  --security-opt apparmor=unconfined \
  -v mika-data:/home/mika/.mika \
  --env-file /path/to/your/mika.env \
  mika-runtime:dev
```

See `os/config/mika.env` for a template with all available variables.

### Volume Mounts

| Mount point | Purpose |
|-------------|---------|
| `/home/mika/.mika` | Mika home — SQLite DB, logs, agent configs, skills |

### OpenRC Services

The image runs two services under OpenRC supervision:

| Service | Binary | Port | Config |
|---------|--------|------|--------|
| `mika-server` | `/usr/local/bin/mika-server` | 8080 | `/etc/conf.d/mika-server` |
| `mika-gateway` | `/usr/local/bin/mika-gateway` | 3001 | `/etc/conf.d/mika-gateway` |

Manage services inside the container:

```bash
docker exec mika rc-service mika-server restart
docker exec mika rc-service mika-gateway stop
```

### Ollama

Ollama is installed but no models are pulled at build time. Pull models at runtime:

```bash
docker exec mika ollama pull llama3
```

## What's Inside

- **Gentoo stage3** base (glibc, OpenRC) — handbook-style `emerge` for all portage-available packages
- **mika** — TUI CLI
- **mika-server** — HTTP agent server (Axum, with telemetry)
- **mika-gateway** — Telegram + GitHub webhook router
- **ollama** — Local LLM inference (v0.6.0, sha256-verified, no models pre-loaded)
- **gh** CLI — GitHub operations (v2.65.0, sha256-verified)
- **gws** CLI — Google Workspace operations (v0.13.3, sha256-verified)
- **OpenRC** process supervision with automatic restart (`supervise-daemon`)
- **Dashboard** — Built-in React observability dashboard (embedded in mika-server)

## Architecture

```
┌─────────────────────────────────────┐
│  mika-os  (gentoo/stage3, pinned)   │
│  Full toolchain + portage tree      │
│  Builds dashboard, Rust binaries,   │
│  installs non-portage CLIs          │
└──────────────┬──────────────────────┘
               │ COPY binaries + configs
               ▼
┌─────────────────────────────────────┐
│  mika-runtime  (gentoo/stage3)      │
│  Runtime deps only (jq, sqlite)     │
│  No compilers, no portage tree      │
└─────────────────────────────────────┘
```

## License

Apache 2.0 — see [LICENSE](../LICENSE) for details.
