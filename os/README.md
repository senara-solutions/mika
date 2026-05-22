<!-- SPDX-License-Identifier: Apache-2.0 -->

# Mika OS — Docker Distribution Channel

> **⚠️ PRE-AUDIT PLACEHOLDER** — This is a functional scaffold. The final recipe will be derived from an audited Gentoo configuration in a follow-up ticket.

Mika OS is a complete runtime image (Gentoo base + OpenRC + mika binaries + ollama) that packages the entire Mika AI executive assistant into a single Docker image.

## Audiences

1. **mika-cloud base image** — Per-customer container base for Helm deployments (replaces Debian-based `Dockerfile.agent` long-term).
2. **Single-box self-host** — `docker run mika-os` for operators who want a complete Mika instance.
3. **Developer reference** — Forkable environment for developers building on Mika.

## Quick Start

### Build

```bash
# From the mika repo root
docker build -t mika-os:dev -f os/Dockerfile .
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
  mika-os:dev
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

### Environment Variables

All `MIKA_*` environment variables are supported. Pass them via `docker run -e` or mount an env file:

```bash
docker run -d \
  --cap-add=SYS_ADMIN \
  --security-opt apparmor=unconfined \
  -v mika-data:/home/mika/.mika \
  --env-file /path/to/your/mika.env \
  mika-os:dev
```

See `os/config/mika.env` for a template with all available variables.

### Volume Mounts

| Mount point | Purpose |
|-------------|---------|
| `/home/mika/.mika` | Mika home — SQLite DB, logs, agent configs, skills |

### Config Files

- `/home/mika/.mika/config.toml` — Server configuration (provider, model, ports)
- `/home/mika/.mika/.env` — Runtime secrets (API keys, tokens)

Default configs are installed from `os/config/` at image build time. Override by mounting your own or editing via `docker exec`.

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
docker run -v ollama-models:/root/.ollama mika-os:dev ollama pull llama3
```

Or from inside a running container:

```bash
docker exec mika ollama pull llama3
```

## What's Inside

- **Gentoo stage3** base (glibc, OpenRC)
- **mika** — TUI CLI
- **mika-server** — HTTP agent server (Axum)
- **mika-gateway** — Telegram + GitHub webhook router
- **ollama** — Local LLM inference (v0.6.0, no models pre-loaded)
- **OpenRC** process supervision with automatic restart
- **gh** CLI for GitHub operations

## Pre-Audit Status

This image is a **functional placeholder**. It works, but the final production recipe will be derived from an audited Gentoo configuration that includes:

- Hardened package selection
- Optimized image size (current stage3 base is ~700MB+)
- Multi-arch build matrix
- Registry publish CI
- Helm chart integration

## License

Apache 2.0 — see [LICENSE](../LICENSE) for details.
