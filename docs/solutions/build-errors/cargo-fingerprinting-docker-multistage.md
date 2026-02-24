---
title: Cargo Fingerprinting Bypass in Docker Multi-Stage Builds with Dummy Source Caching
date: 2026-02-24
category: build-errors
tags:
  - docker
  - cargo
  - caching
  - fingerprinting
  - multi-stage-builds
  - rust
  - openssl
severity: medium
component:
  - Dockerfile.agent
  - Dockerfile.gateway
symptoms:
  - Docker image contains 321KB dummy binary instead of expected 10MB real binary
  - Cargo reuses cached fingerprints across Docker layers despite source file changes
  - Container starts but does not function correctly
  - Binary size discrepancy revealed during smoke testing
root_cause: >
  Cargo fingerprints from the dummy-source build survive in target/release/.fingerprint/
  and cause Cargo to skip recompilation when real source is copied via Docker COPY
---

# Cargo Fingerprinting Bypass in Docker Multi-Stage Builds

## Problem Statement

When building Rust binaries in Docker using the standard dummy-source dependency caching technique, two problems were encountered:

1. **Silent binary corruption:** The multi-stage Docker build produced a 321KB binary instead of the expected ~10MB (with LTO+strip). The container started but did not function correctly. No build error was reported.

2. **Missing OpenSSL build dependencies:** Docker build failed with `openssl-sys` compilation error: `pkg-config` not found. The `reqwest` crate uses OpenSSL by default on Linux, and `rust:1.85-slim` does not include dev headers.

## Investigation Steps

**Problem 1 — Cargo fingerprinting:**

1. Built the Docker image using the standard dummy-source dependency caching technique (create stub `fn main() {}` files, compile to cache deps, then replace with real source).
2. Observed the resulting binary was 321KB — clearly wrong for a binary with LTO+strip enabled (expected ~10MB).
3. Confirmed the container started but did not work correctly, ruling out a simple size anomaly.
4. Traced the Docker build sequence:
   - (a) Create dummy `fn main() {}` source files for all workspace crates
   - (b) Run `cargo build --release` to compile and cache all external dependencies
   - (c) `rm -rf crates/` to remove dummy sources
   - (d) `COPY crates/ crates/` to bring in real source files
   - (e) Run `cargo build --release` again to compile the actual binary
5. Identified that in step (e), Cargo's fingerprints from step (b) were still present in `target/release/.fingerprint/`, causing Cargo to conclude the workspace crates were already up to date and reuse the dummy 321KB binary.

**Problem 2 — OpenSSL dependencies:**

1. Observed build failure on `openssl-sys` during Docker build with error: `pkg-config` not found.
2. Confirmed that local builds succeeded because OpenSSL and `pkg-config` were already installed on the host system.
3. Identified that `reqwest`'s default features include `default-tls` (native-tls/OpenSSL), which requires `pkg-config` and `libssl-dev` to compile.

## Root Cause Analysis

### Cargo Fingerprinting

Cargo stores per-crate fingerprints in `target/release/.fingerprint/<crate-name>-<hash>/`. These fingerprints record source file timestamps and content hashes at the time of compilation.

When real source files are copied into the Docker layer via `COPY crates/ crates/`, the fingerprint files from the dummy build remain intact in the `target/` directory (carried forward by Docker layer caching). Cargo compares the incoming source timestamps against the stored fingerprints and concludes the workspace crates are already built — silently reusing the dummy 321KB binary without recompilation.

The critical detail: **Docker `COPY` does not invalidate Cargo's internal build cache.** Cargo's fingerprinting is filesystem-timestamp and content-hash based, not Docker-layer-aware.

### Missing OpenSSL

The `reqwest` crate's default feature flags pull in `native-tls`/OpenSSL. The `openssl-sys` crate uses `pkg-config` at build time to locate headers. The `rust:1.85-slim` Docker image does not include `pkg-config`, `libssl-dev`, or related OpenSSL build tooling.

## Working Solution

### Fingerprint cleanup (the key fix)

Explicitly clean all workspace crate artifacts between the dummy build and the real build, while preserving external dependency compilation:

```dockerfile
# Build dependencies only, then clean workspace crate artifacts (keep dependency cache)
RUN cargo build --release --bin mika-server -p mika-agent || true \
    && rm -rf crates/ \
    && rm -f target/release/mika-server target/release/mika-cli \
    && rm -rf target/release/.fingerprint/mika-* \
    && rm -rf target/release/deps/libmika_* target/release/deps/mika_*
```

The four-part cleanup:

| Step | Command | Purpose |
|------|---------|---------|
| 1 | `rm -rf crates/` | Remove dummy source files |
| 2 | `rm -f target/release/mika-*` | Remove compiled dummy binaries |
| 3 | `rm -rf target/release/.fingerprint/mika-*` | Remove workspace crate fingerprints only (`mika-*` glob targets workspace crates, not external deps) |
| 4 | `rm -rf target/release/deps/libmika_* target/release/deps/mika_*` | Remove workspace crate artifacts from deps/ |

This forces Cargo to recompile all workspace crates from real source while keeping third-party dependency compilation cached.

### OpenSSL build dependencies

Add `pkg-config` and `libssl-dev` to builder stages:

```dockerfile
# Agent (also needs gcc for rusqlite bundled SQLite):
RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc libc-dev pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Gateway (no gcc needed — -p mika-gateway skips rusqlite):
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
```

## Verification

- Both Docker images rebuilt successfully after applying fixes.
- Agent image: 95.3MB total, binary 10.6MB (correct with LTO+strip).
- Gateway image: 90.3MB total, binary 5.6MB.
- Health endpoints responded correctly on both containers.
- Dependency cache preserved: source-only changes trigger only workspace crate recompilation (~35s), not full dependency rebuild (~25min).

## Prevention Strategies

### For Cargo fingerprinting

1. **Always clean workspace fingerprints** between dummy and real builds. The `rm -rf target/release/.fingerprint/<workspace-prefix>-*` pattern is essential.
2. **Verify binary size** as a post-build check. A binary that is suspiciously small (< 1MB for a non-trivial app) indicates stale artifacts.
3. **Avoid `2>/dev/null`** on the dummy build step — let errors be visible for debugging. Use `|| true` alone to tolerate expected link failures.
4. **Combine build+cleanup in one RUN** to reduce Docker layers and prevent intermediate stale artifacts from persisting.

### For OpenSSL dependencies

1. **Audit native dependencies** before writing Dockerfiles. Common ones: `reqwest` → OpenSSL, `rusqlite` with `bundled` → gcc, `sqlx` → may need `libpq-dev`.
2. **Consider `rustls-tls`** instead of native-tls to eliminate C dependencies entirely: `reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }`.
3. **Use `--no-install-recommends`** to minimize builder image bloat.
4. **Comment why each native dep is needed** — prevents accidental removal and helps future maintainers.

## Checklist for New Rust Dockerfiles

### Builder stage
- [ ] Install all native build deps (gcc, libssl-dev, pkg-config, etc.)
- [ ] Copy workspace manifests + Cargo.lock first (cache layer)
- [ ] Create dummy source files matching all `[[bin]]` and `[lib]` targets
- [ ] Build with `|| true` (no `2>/dev/null`)
- [ ] Clean workspace artifacts: binaries, fingerprints, deps
- [ ] Copy real source and build

### Runtime stage
- [ ] Use minimal base (debian:bookworm-slim)
- [ ] Install only runtime libs (ca-certificates, not dev headers)
- [ ] Non-root user with appropriate home directory setup
- [ ] Verify binary size matches expectations

### Validation
- [ ] Build from clean Docker cache
- [ ] Verify binary size (should match local `cargo build --release`)
- [ ] Test health endpoint
- [ ] Confirm running as non-root (`docker run --rm image id`)

## References

### Internal
- Implementation plan: `docs/plans/2026-02-24-feat-dockerfiles-container-images-plan.md`
- Parent plan: `docs/plans/2026-02-24-feat-platform-systems-gateway-provisioning-heartbeat-plan.md`
- Server architecture: `docs/solutions/architecture-decisions/phase2-axum-http-server-architecture.md`

### Key files
- `Dockerfile.agent` — agent container with rusqlite + OpenSSL
- `Dockerfile.gateway` — gateway container with OpenSSL only
- `Cargo.toml` — workspace root with `[profile.release]` (lto, strip, codegen-units)
- `crates/mika-common/src/home.rs` — home directory bootstrap (creates subdirs on first run)
- `crates/mika-common/src/config.rs` — Settings::load() config cascade
