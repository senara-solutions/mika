<!-- SPDX-License-Identifier: Apache-2.0 -->

# Mika OS — Docker Distribution Channel

Mika OS is a complete runtime image (Gentoo base + OpenRC + mika binaries + ollama) that packages the entire Mika AI executive assistant into a single Docker image.

## Build Targets

Six named multi-stage targets serve different audiences:

| Target | Binary set | Tooling | OpenRC services | Audience |
|--------|------------|---------|-----------------|----------|
| `mika-os` | All three + full toolchain | gh, gws, ollama + portage tree | mika-server, mika-gateway | Forkable reference, dev environment |
| `mika-runtime-base` | None (shared base) | None | None | Internal — not pushed standalone |
| `mika-runtime-server` | mika-server | gh, ollama | mika-server | mika-cloud per-customer agent container |
| `mika-runtime-gateway` | mika-gateway | None | mika-gateway | mika-cloud gateway deployment |
| `mika-runtime-cli` | mika (TUI) | None | None | Operator desktop install via Docker |
| `mika-runtime-all` | All three binaries | gh, gws, ollama | mika-server, mika-gateway | Single-box self-host |

### Stage hierarchy

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
│  mika-runtime-base  (gentoo/stage3) │
│  Runtime deps only (jq, sqlite)     │
│  No compilers, no portage tree      │
│  No role-specific binaries          │
└──────┬──────┬──────┬──────┬─────────┘
       │      │      │      │
       ▼      ▼      ▼      ▼
   server  gateway  cli    all
```

## Quick Start

### Build

```bash
# From the mika repo root (requires BuildKit for per-Dockerfile .dockerignore)

# Per-customer agent container (recommended for mika-cloud)
docker build --target mika-runtime-server -f os/Dockerfile -t mika-runtime-server:dev .

# Gateway deployment
docker build --target mika-runtime-gateway -f os/Dockerfile -t mika-runtime-gateway:dev .

# Interactive TUI
docker build --target mika-runtime-cli -f os/Dockerfile -t mika-runtime-cli:dev .

# Single-box self-host (all binaries)
docker build --target mika-runtime-all -f os/Dockerfile -t mika-runtime-all:dev .

# Full developer image
docker build --target mika-os -f os/Dockerfile -t mika-os:dev .
```

### Run

```bash
# Daemon targets (server, gateway, all) — OpenRC supervision
docker run -d \
  --name mika \
  --cap-add=SYS_ADMIN \
  --security-opt apparmor=unconfined \
  -v mika-data:/home/mika/.mika \
  -p 8080:8080 \
  -e MIKA_ANTHROPIC_API_KEY=sk-ant-... \
  mika-runtime-server:dev

# CLI target — interactive TUI
docker run -it --rm \
  -v mika-data:/home/mika/.mika \
  mika-runtime-cli:dev
```

**Required runtime flags (daemon targets only):**

- `--cap-add=SYS_ADMIN` — Required for OpenRC process supervision inside the container. Preferred over `--privileged`.
- `--security-opt apparmor=unconfined` — Required on hosts with AppArmor enforcing (Ubuntu, Debian). Not needed on hosts without AppArmor (Gentoo, Alpine, most minimal container hosts).

### Verify

```bash
# Check services are running (server or all targets)
docker exec mika rc-service mika-server status

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
  mika-runtime-server:dev
```

See `os/config/mika.env` for a template with all available variables.

### Volume Mounts

| Mount point | Purpose |
|-------------|---------|
| `/home/mika/.mika` | Mika home — SQLite DB, logs, agent configs, skills |

### OpenRC Services

Daemon targets run services under OpenRC supervision:

| Service | Binary | Port | Config | Present in |
|---------|--------|------|--------|------------|
| `mika-server` | `/usr/local/bin/mika-server` | 8080 | `/etc/conf.d/mika-server` | server, all |
| `mika-gateway` | `/usr/local/bin/mika-gateway` | 3001 | `/etc/conf.d/mika-gateway` | gateway, all |

Manage services inside the container:

```bash
docker exec mika rc-service mika-server restart
docker exec mika rc-service mika-gateway stop
```

### Ollama

Ollama is included in `mika-runtime-server` and `mika-runtime-all` targets. No models are pulled at build time. Pull models at runtime:

```bash
docker exec mika ollama pull llama3
```

## What's Inside

- **Gentoo stage3** base (glibc, OpenRC) — handbook-style `emerge` for all portage-available packages
- **mika** — TUI CLI (cli, all targets)
- **mika-server** — HTTP agent server (Axum, with telemetry) (server, all targets)
- **mika-gateway** — Telegram + GitHub webhook router (gateway, all targets)
- **ollama** — Local LLM inference (v0.6.0, sha256-verified, no models pre-loaded) (server, all targets)
- **gh** CLI — GitHub operations (v2.65.0, sha256-verified) (server, all targets)
- **gws** CLI — Google Workspace operations (v0.13.3, sha256-verified) (all target only)
- **OpenRC** process supervision with automatic restart (`supervise-daemon`) (server, gateway, all targets)
- **Dashboard** — Built-in React observability dashboard (embedded in mika-server)

## License

Apache 2.0 — see [LICENSE](../LICENSE) for details.
