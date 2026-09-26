# Plan: Audit Dockerfile.agent + Rewrite mika/os/Dockerfile (mika#1243)

- **Issue:** mika#1243
- **Type:** chore (audit + rewrite)
- **Branch:** `chore/1243/docker-audit-dockerfile-agent-rewrite`
- **Date:** 2026-05-22
- **Supersedes:** mika#1242 (subsumed by Surface 1 audit)

## Context

Three bugs shipped in `Dockerfile.agent` within 24h (mika#1237, mika#1240, mika#1242) — all download-rename / sha256sum / path-resolution issues. Separately, `os/Dockerfile` shipped as a pre-audit placeholder (mika#1115) using Debian-style patterns on a Gentoo base. Vincent specified handbook-style Gentoo (real `emerge`, portage configuration — not `wget` + binary download patterns).

## Codebase Analysis

### Current State: Dockerfile.agent (78 lines)

Three-stage build: dashboard-builder (node:24-slim) → builder (rust:1.93-slim) → runtime (debian:bookworm-slim). Produces `mika-spirit` + `gh` + `gws` CLIs.

**Findings from exhaustive audit:**

1. **GH CLI install (L39-46): CLEAN.** Downloads as `gh_${GH_VERSION}_linux_${ARCH}.tar.gz`, checksums reference same filename, `sha256sum -c` matches. Fixed in mika#1241.
2. **GWS CLI install (L49-58): CLEAN.** Downloads as `gws.tar.gz`, constructs checksum line from `.sha256` file content. The `.sha256` file from the GWS release contains only the hash (no filename), so the `echo "$(cat gws.tar.gz.sha256)  gws.tar.gz"` pattern is correct.
3. **`.dockerignore` interaction: CLEAN for Dockerfile.agent.** It excludes `docs/`, `*.md`, and `scripts/` — but Dockerfile.agent doesn't COPY any of these. It copies only `Cargo.toml`, `Cargo.lock`, `crates/`, `packages/`, `dashboard/`, and `package.json`/`package-lock.json`.
4. **Builder stub for mika-gateway (L22-23):** Creates dummy `src/main.rs` + empty `migrations/.keep`. Correct — workspace compilation needs all members present.
5. **No `COPY docs/` or `COPY skills/` needed.** Dockerfile.agent builds only `mika-spirit` from `mika-agent` crate. The `build.rs` in mika-agent embeds docs and skills via `include_str!(concat!(env!("OUT_DIR"), ...))` — but it reads from the source tree, which IS in the build context (`crates/mika-agent/` is COPY'd). Wait — `build.rs` walks `../../docs/` and `../../skills/bundled/` relative to the crate. Since `.dockerignore` excludes `docs/` and `skills/` isn't excluded, let me verify...

   **CRITICAL FINDING:** `crates/mika-agent/build.rs` copies `docs/` and `skills/bundled/` into `OUT_DIR` at build time. The path resolution is relative to the workspace root (`../../docs/` from `crates/mika-agent/`). With `docs/` in `.dockerignore`, the `os/Dockerfile` builder stage's `COPY docs/ docs/` would fail or be empty. For `Dockerfile.agent`, this is also a latent bug — the build succeeds only because `build.rs` has fallback behavior when docs are missing (it generates empty constants, and `mika-spirit` doesn't serve them in agent-container mode). **But this means bundled skills with `system_prompt.md` won't embed correctly in Dockerfile.agent either, since `skills/` is NOT in `.dockerignore` but `*.md` IS** — meaning `system_prompt.md` files within `skills/bundled/` are excluded from the Docker build context.

   Actually, re-reading `.dockerignore`: the `*.md` glob only matches files at the root level, not recursively. Docker's `.dockerignore` uses Go filepath matching where `*.md` matches only in the root. To match recursively it would need `**/*.md`. So `skills/bundled/*/system_prompt.md` files ARE included. `docs/` is excluded as a directory. This means:
   - `Dockerfile.agent`: builder can access `skills/` (including .md files inside it) but NOT `docs/`. The `build.rs` will fail to copy docs but handles this gracefully. **CLEAN for agent-only use.**
   - `os/Dockerfile`: `COPY docs/ docs/` on L42 **WILL FAIL** because `docs/` is excluded by `.dockerignore`.

6. **mika-a2a crate missing from Dockerfile.agent:** The builder doesn't COPY `crates/mika-a2a/` but mika-agent depends on it. This works only if mika-a2a is an optional dependency or if the gateway stub satisfies the workspace. Need to verify — if mika-agent's `Cargo.toml` has `mika-a2a` as a dependency, the build would fail. **Checked:** mika-agent does depend on mika-a2a. The current Dockerfile.agent might be relying on cached layers. **This is a latent build-from-scratch bug.**

### Current State: os/Dockerfile (138 lines)

Three-stage build: dashboard-builder (node:24-slim) → builder (rust:1.93-slim) → runtime (gentoo/stage3:20250505). Cross-compilation from Debian builder to Gentoo runtime.

**Findings:**

1. **`.dockerignore` breaks the build.** `COPY docs/ docs/` (L42) fails because `docs/` is excluded. This is a **blocking build bug** in the current os/Dockerfile.
2. **Debian builder → Gentoo runtime:** Binaries compiled in `rust:1.93-slim` (Debian bookworm glibc) are COPY'd into `gentoo/stage3`. This works because both use glibc, but the glibc versions must be ABI-compatible. Not handbook-style per AC4.
3. **Ollama install (L87-95): No checksum verification.** Just `wget -qO` + `chmod 755`. Security gap.
4. **GH CLI install (L72-84): Same class as fixed Dockerfile.agent bugs.** Downloads as `/tmp/gh.tar.gz` (renamed) but checksum file references `gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz` (original name). The `grep` filters the right line, and `sha256sum -c -` reads from stdin where the filename is whatever the checksum line says. **BUT** the checksum line will say `gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz` while the file is `/tmp/gh.tar.gz`. This means `sha256sum -c` will look for a file named `gh_2.65.0_linux_amd64.tar.gz` in `/tmp/`, not `gh.tar.gz`. **This is the same bug class as mika#1240** — download-rename breaks `sha256sum -c`.
5. **`useradd` (L98):** Uses `-s /bin/bash` which is fine on Gentoo (bash is in stage3). Not a bug.
6. **No mika-cli crate or gateway crate stubs in Dockerfile.agent builder** but os/Dockerfile copies all crates — correct for building all three binaries.

## Plan

### Phase 1: Per-Dockerfile `.dockerignore` for os/Dockerfile (prerequisite)

**Problem:** The shared `.dockerignore` excludes `docs/` intentionally — `build.rs` has a two-tier fallback (`../../docs/` → crate-local `docs/`) validated in mika#1237. Removing `docs/` from the shared `.dockerignore` would penalize `Dockerfile.agent` builds (larger context) for zero benefit to that image.

**Decision: Per-Dockerfile `.dockerignore` file (F1 resolution).**

BuildKit supports `<Dockerfile>.dockerignore` files. For `docker build -f os/Dockerfile .`, BuildKit checks `os/Dockerfile.dockerignore`. Create this file as a copy of root `.dockerignore` with `docs/` and `*.md` lines removed (os/Dockerfile needs both for build.rs workspace-root docs path and for runtime KG ingestion).

**Changes:**
- **Keep root `.dockerignore` unchanged** — Dockerfile.agent and Dockerfile.gateway continue using it as-is
- **Create `os/Dockerfile.dockerignore`** — same content as root but without `docs/` and `*.md` exclusions
- Root `.dockerignore` stays the default for builds that don't specify `-f os/Dockerfile`

### Phase 2: Dockerfile.agent audit fixes (F2 — pinned at base SHA)

**Base SHA:** `0646e85b` (HEAD of `chore/1243/docker-audit-dockerfile-agent-rewrite` at plan time, tip of main — `fix(dockerfile-agent): gh CLI install checksum verification fails — wget filename mismatch (#1241)`).

**Verbatim COPY block (L18-23) at base SHA:**
```dockerfile
COPY Cargo.toml Cargo.lock ./
COPY crates/mika-common/ crates/mika-common/
COPY crates/mika-agent/ crates/mika-agent/
COPY crates/mika-gateway/Cargo.toml crates/mika-gateway/Cargo.toml
RUN mkdir -p crates/mika-gateway/src && echo "fn main() {}" > crates/mika-gateway/src/main.rs \
    && mkdir -p crates/mika-gateway/migrations && touch crates/mika-gateway/migrations/.keep
```

**Bug verification:** Workspace `Cargo.toml` has `members = ["crates/*"]`, which resolves to `mika-common`, `mika-a2a`, `mika-agent`, `mika-gateway`, `mika-cli`. The builder COPY block includes only `mika-common`, `mika-agent`, and `mika-gateway` (as a stub). Missing: `mika-a2a` (full source needed — mika-agent depends on it via `mika-a2a.workspace = true`) and `mika-cli` (stub needed — workspace member but not built by this Dockerfile). With BuildKit cache, a warm build resolves these from the registry cache, masking the bug. A clean `docker build --no-cache` from a fresh checkout will fail at `cargo build --release --bin mika-spirit` when the resolver can't find `mika-a2a`.

**Fix 1: Add missing mika-a2a crate COPY.**
Insert after L19 (mika-common COPY), before L20 (mika-agent COPY):
```dockerfile
COPY crates/mika-a2a/ crates/mika-a2a/
```

**Fix 2: Add missing mika-cli stub.**
Insert after the mika-gateway stub (L22-23):
```dockerfile
COPY crates/mika-cli/Cargo.toml crates/mika-cli/Cargo.toml
RUN mkdir -p crates/mika-cli/src && echo "fn main() {}" > crates/mika-cli/src/main.rs
```

**Fix 3: Verify end-to-end build.** `docker build --no-cache -f Dockerfile.agent -t mika-agent:test .` must succeed. (AC2)

### Phase 3: os/Dockerfile rewrite as multi-stage handbook-style Gentoo

This is the core deliverable. **Full replacement** of the current 138-line placeholder (mika#1115, SHA `67b438cc`).

**mika#1115 contract survival (F3 resolution):**

The mika#1115 GROOMED plan established two contracts that must survive the rewrite:

| Contract | Source | Survival in this plan |
|----------|--------|----------------------|
| **F1: Full-OpenRC operator decision** | mika#1115 plan, Vincent's call | ✅ Preserved. Both mika-os and mika-runtime use OpenRC with `supervise-daemon` for process supervision. Same `rc-update add` pattern. |
| **F2: Entrypoint lifecycle** | `os/init/mika-os-init.sh` (8 lines of contract) | ✅ Preserved unchanged. The entrypoint script is COPY'd as-is from `os/init/`. Boot order (rc default → services in dependency graph), SIGTERM trap (reverse-order stop), child-exit respawn (supervise-daemon config), and container-alive tail are all retained. |
| **OpenRC init.d scripts** | `os/openrc/init.d/{mika-spirit,mika-gateway}` | ✅ Preserved unchanged. `command_background=yes`, `supervise_daemon_args`, `depend()` ordering, `start_pre()` env sourcing. |
| **OpenRC conf.d files** | `os/openrc/conf.d/{mika-spirit,mika-gateway}` | ✅ Preserved unchanged. Env var sourcing pattern. |
| **Three audiences** | mika#1115 README | ✅ Preserved: (1) mika-cloud via mika-runtime, (2) single-box self-host via mika-os, (3) developer reference via mika-os with full toolchain. |

**What changes:** The Rust builder stage moves from external `rust:1.93-slim` (Debian) to in-stage `emerge rust-bin` (Gentoo handbook-style). The Debian-pattern binary downloads (`wget` + rename) are replaced with `emerge` for all portage-available packages. Non-portage binaries (gh, gws, ollama) keep the download pattern but with correct sha256 verification. The `node:24-slim` dashboard-builder stage is eliminated — nodejs is emerged and dashboard built in-stage.

#### Architecture: Two named build targets

```
┌─────────────────────────────────────────────┐
│  mika-os  (gentoo/stage3:pinned)            │
│  → emerge-webrsync (portage tree)           │
│  → emerge toolchain + all deps              │
│    (rust-bin, nodejs, gcc, pkgconf, jq,     │
│     sqlite, wget, ca-certificates)          │
│  → npm ci + npm run build (dashboard)       │
│  → cargo build --release (all 3 binaries)   │
│  → emerge --depclean + @world update        │
│  → OpenRC services + configs                │
│  → gh CLI + gws CLI (non-portage, sha256)   │
│  → ollama binary (non-portage, sha256)      │
│  → Default configs at /etc/mika/            │
│  → User mika:1000                           │
│  BUILD TARGET: --target mika-os             │
└──────────────────┬──────────────────────────┘
                   │ COPY binaries + services + configs
                   ▼
┌─────────────────────────────────────────────┐
│  mika-runtime  (gentoo/stage3:pinned)       │
│  → emerge runtime-only deps (jq, sqlite)    │
│  → NO compilers, NO headers, NO rust, NO node│
│  → COPY from mika-os:                       │
│    - /usr/local/bin/mika*                   │
│    - /usr/local/bin/gh, gws, ollama         │
│    - /etc/init.d/mika-*, /etc/conf.d/mika-*│
│    - /etc/mika/ (default configs)           │
│    - /home/mika/ (user home)                │
│    - /app/docs/ (KG ingestion)              │
│  → OpenRC configured                        │
│  BUILD TARGET: --target mika-runtime        │
└─────────────────────────────────────────────┘
```

**No separate dashboard-builder stage.** AC4 requires "Dashboard built via npm" in the mika-os stage itself, with `net-libs/nodejs` emerged via portage. This is handbook-style: all build deps live in the build stage, no external Debian-based builder. The dashboard `npm ci && npm run build` runs in mika-os after nodejs is emerged.

#### Dynamic linking strategy

**Decision: Use default dynamic linking (NOT `crt-static`).**

Rationale:
- Both mika-os and mika-runtime use the same `gentoo/stage3` base image (same glibc).
- Binaries compiled in-stage on mika-os link against the stage3 glibc — same version in runtime.
- `crt-static` would bloat each binary by ~2-4MB and is unnecessary when base images match.
- For mika-runtime, we don't need to COPY glibc — it's already in the stage3 base.
- We DO need to verify no other dynamic deps are missing: run `ldd` on compiled binaries in the implementation step and document the deps.

#### mika-os stage details (AC4)

1. **Base:** `gentoo/stage3:20250505` (pinned, same as current placeholder)
2. **Portage setup:**
   - Sync portage tree: `emerge-webrsync` (faster than `emerge --sync` in Docker, no git needed)
   - Set `MAKEOPTS="-j$(nproc)"` in `make.conf` for parallel compilation
   - Set USE flags: `"-X -gtk -qt5 -wayland"` (headless server, no GUI)
   - Set `ACCEPT_KEYWORDS="amd64"` (stable only)
3. **System packages via emerge:**
   - `dev-lang/rust-bin` (Rust toolchain — binary package, avoids 30+ min source bootstrap)
   - `net-libs/nodejs` (Node.js for dashboard build — `npm ci && npm run build` runs in-stage per AC4)
   - `sys-devel/gcc` (C compiler for rusqlite's bundled SQLite)
   - `sys-libs/glibc` (libc headers for compilation — already in stage3 but ensure headers present)
   - `dev-util/pkgconf` (pkg-config replacement, Gentoo default)
   - `app-misc/jq` (required by skill handler scripts)
   - `dev-db/sqlite` (runtime dep)
   - `net-misc/wget` (for non-portage binary downloads)
   - `app-misc/ca-certificates` (TLS root certs)
4. **Non-portage binaries (with sha256 verification):**
   - `gh` CLI (not in Gentoo portage) — download + verify checksum + install to `/usr/local/bin/`
   - `gws` CLI (not in Gentoo portage) — same pattern
   - `ollama` (not in Gentoo portage) — **ADD checksum verification** (currently missing, AC7)
5. **Dashboard build (in-stage, per AC4):**
   - `COPY package.json package-lock.json ./`
   - `COPY packages/ packages/`
   - `COPY dashboard/ dashboard/`
   - `npm ci --ignore-scripts && npm run build --prefix dashboard`
6. **Mika compilation:**
   - `COPY` workspace source (Cargo.toml, Cargo.lock, crates/, docs/, skills/)
   - `cargo build --release --features telemetry --bin mika --bin mika-spirit --bin mika-gateway`
   - Install to `/usr/local/bin/`
7. **OpenRC services:** COPY init.d + conf.d scripts from `os/openrc/`, `rc-update add`
8. **Default configs at `/etc/mika/` (FHS-compliant, per AC5):**
   - `COPY os/config/config.toml /etc/mika/config.toml`
   - `COPY os/config/mika.env /etc/mika/mika.env.template`
   - Mika reads from `$MIKA_HOME` at runtime; the entrypoint script uses a **copy-if-not-exists guard** (F4 resolution): `[ -f "$MIKA_HOME/config.toml" ] || cp /etc/mika/config.toml "$MIKA_HOME/config.toml"` — this preserves user-mounted configs on volume restart and never overwrites operator customizations
9. **User creation:** `useradd -m -u 1000 -s /bin/bash mika` (Gentoo-native)
10. **World set and portage cleanup (per AC4, F5 resolution):**
    - `emerge --update --newuse --deep @world` — ensure world set is consistent
    - `emerge --depclean` — remove orphaned packages
    - `rm -rf /var/tmp/portage /var/cache/distfiles` — reclaim build artifacts
    - **Keep `/var/db/repos/gentoo` (portage tree) in mika-os** — this is a developer reference image; users need `emerge` capability post-build. The portage tree is only stripped in `mika-runtime` (production image where no emerge is needed)

#### mika-runtime stage details (AC5)

1. **Base:** Same `gentoo/stage3:20250505` pinned tag
2. **Minimal emerge:** Only runtime deps (no compilers, no dev headers):
   - `app-misc/jq`
   - `dev-db/sqlite` (libsqlite3 for rusqlite runtime linking)
   - `app-misc/ca-certificates`
3. **COPY list from mika-os (with justification per AC5):**

   | Source (from mika-os) | Destination | Justification |
   |---|---|---|
   | `/usr/local/bin/mika` | `/usr/local/bin/mika` | TUI CLI binary |
   | `/usr/local/bin/mika-spirit` | `/usr/local/bin/mika-spirit` | Agent HTTP server |
   | `/usr/local/bin/mika-gateway` | `/usr/local/bin/mika-gateway` | Webhook gateway |
   | `/usr/local/bin/gh` | `/usr/local/bin/gh` | GitHub CLI for builtin skill |
   | `/usr/local/bin/gws` | `/usr/local/bin/gws` | Google Workspace CLI for builtin skill |
   | `/usr/local/bin/ollama` | `/usr/local/bin/ollama` | Local LLM inference |
   | `/etc/init.d/mika-spirit` | `/etc/init.d/mika-spirit` | OpenRC service script |
   | `/etc/init.d/mika-gateway` | `/etc/init.d/mika-gateway` | OpenRC service script |
   | `/etc/conf.d/mika-spirit` | `/etc/conf.d/mika-spirit` | OpenRC env config |
   | `/etc/conf.d/mika-gateway` | `/etc/conf.d/mika-gateway` | OpenRC service config |
   | `/etc/mika/` | `/etc/mika/` | Default config files (FHS-compliant, per AC5) |
   | `/home/mika/` | `/home/mika/` | User home directory |
   | `/app/docs/` | `/app/docs/` | Docs for KG ingestion |
   | `/usr/local/bin/mika-os-init.sh` | `/usr/local/bin/mika-os-init.sh` | Container entrypoint |

4. **OpenRC registration:** `rc-update add mika-spirit default && rc-update add mika-gateway default`
5. **Portage cleanup:** `rm -rf /var/tmp/portage /var/cache/distfiles /var/db/repos/gentoo` — strip portage tree in runtime image (no emerge needed post-build, unlike mika-os which keeps it for developer use)
6. **Same EXPOSE, HEALTHCHECK, ENTRYPOINT** as current os/Dockerfile

### Phase 4: Verify builds (AC2, AC6)

1. `docker build -f Dockerfile.agent -t mika-agent:test .` — must succeed
2. `docker build --target mika-os -f os/Dockerfile -t mika-os:test .` — must succeed
3. `docker build --target mika-runtime -f os/Dockerfile -t mika-runtime:test .` — must succeed

Note: Full functional verification (AC6: "entrypoint runs OpenRC, services start, healthcheck returns 0") requires runtime API keys and is deferred to manual verification. The build succeeding is the CI-testable gate.

### Phase 5: Update os/README.md

Update to reflect the multi-stage build targets:
- Document `--target mika-os` and `--target mika-runtime` usage
- Remove "PRE-AUDIT PLACEHOLDER" banner
- Update "What's Inside" section

## File Change Summary

| File | Action | AC |
|------|--------|-----|
| `.dockerignore` | **Unchanged** — Dockerfile.agent continues using it; build.rs two-tier fallback is validated | — |
| `os/Dockerfile.dockerignore` | **New** — per-Dockerfile ignore (BuildKit), same as root minus `docs/` and `*.md` | AC3, AC4 |
| `Dockerfile.agent` | Edit: add mika-a2a COPY, add mika-cli stub | AC1, AC2 |
| `os/Dockerfile` | Rewrite: multi-stage handbook-style Gentoo | AC3-AC7, AC9 |
| `os/init/mika-os-init.sh` | Edit: add copy-if-not-exists guard for `/etc/mika/` → `$MIKA_HOME` | AC6 |
| `os/README.md` | Update: document build targets, remove placeholder banner | AC8 |

## Risks and Mitigations

1. **Gentoo emerge is slow in Docker.** `emerge-webrsync` + `rust-bin` mitigate the worst cases. BuildKit layer caching means rebuilds only re-run changed layers.
2. **Stage3 image size.** Gentoo stage3 is ~700MB base. mika-runtime will be ~800MB+ even with cleanup. This is acceptable for the target audience (self-host, developer reference). mika-cloud production images will eventually use a stripped-down base.
3. **Portage tree freshness.** `emerge-webrsync` pulls a date-stamped snapshot. Pin to a specific date or accept latest-at-build-time. Decision: accept latest — portage packages are stable-keyworded.
4. **glibc ABI compatibility.** Both stages use the same stage3 base → same glibc. No cross-compilation risk.

## Implementation Order

1. `.dockerignore` fix (unblocks everything)
2. `Dockerfile.agent` audit fixes (small, independent)
3. `os/Dockerfile` rewrite (core deliverable)
4. `os/README.md` update (documentation)
5. Build verification (both Dockerfiles)

## Pre-decided for implementation (no clarifying questions needed)

The implementing pilot MUST proceed without asking the operator. All design ambiguities are pre-resolved here:

1. **Ollama checksum verification (AC7):** Pin to ollama `v0.6.0`. Derive the sha256 at build time from the official release URL:

   ```dockerfile
   ARG OLLAMA_VERSION=0.6.0
   RUN ARCH=$(dpkg --print-architecture 2>/dev/null || echo amd64) && \
       case "$ARCH" in amd64) OLLAMA_ARCH=amd64;; arm64) OLLAMA_ARCH=arm64;; *) echo "unsupported $ARCH"; exit 1;; esac && \
       wget -qO "/tmp/ollama-linux-${OLLAMA_ARCH}" "https://github.com/ollama/ollama/releases/download/v${OLLAMA_VERSION}/ollama-linux-${OLLAMA_ARCH}" && \
       wget -qO "/tmp/ollama-linux-${OLLAMA_ARCH}.sha256" "https://github.com/ollama/ollama/releases/download/v${OLLAMA_VERSION}/ollama-linux-${OLLAMA_ARCH}.sha256" && \
       cd /tmp && sha256sum -c "ollama-linux-${OLLAMA_ARCH}.sha256" && \
       install -m 755 "ollama-linux-${OLLAMA_ARCH}" /usr/local/bin/ollama && rm -f /tmp/ollama-linux-*
   ```

   If the upstream `.sha256` file format differs (some releases use `<hash>  <filename>` two-column shape, others raw hash), adapt by constructing the checksum line locally — but verify via the standard `sha256sum -c` pattern. Document the chosen pattern in the PR description.

2. **emerge-webrsync timing:** Accept latest-at-build-time per Risk #3 (already settled in this plan — no re-litigation).

3. **Phase 4 verification scope:** Build-only verification (`docker build --target mika-os` + `docker build --target mika-runtime` succeed). Runtime/functional testing is operator-manual, NOT pilot's responsibility. Do NOT attempt `docker run` or service starts.

**Contract:** if the implementation surfaces a NEW ambiguity not pre-decided above (something genuinely undecided), default-decide using your best judgment and document the decision in the PR description. Do NOT end the turn asking the operator — claude-pilot has no operator-question relay, asking just triggers `pipeline_incomplete` and burns the dispatch.

## Tie-back to Acceptance Criteria

- **AC1:** Phase 2 — exhaustive Dockerfile.agent audit, all instances fixed
- **AC2:** Phase 4 — `docker build -f Dockerfile.agent` succeeds
- **AC3:** Phase 3 — single Dockerfile, two named targets
- **AC4:** Phase 3 mika-os stage — emerge-based, handbook-style
- **AC5:** Phase 3 mika-runtime stage — COPY list documented with justification
- **AC6:** Phase 4 — build verification (functional verification deferred to manual)
- **AC7:** Phase 3 — ollama checksum added, all sha256sum patterns swept
- **AC8:** This plan document covers both surfaces
- **AC9:** Phase 3 mika-os stage — portage configuration, emerge packages, handbook framing
