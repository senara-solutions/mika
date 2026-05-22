---
title: Dockerfile.agent broken COPY for non-existent repo-root paths
date: 2026-05-22
category: build-errors
module: Dockerfile.agent
problem_type: build_error
component: tooling
symptoms:
  - "docker build -f Dockerfile.agent fails with: ERROR failed to compute cache key: failed to calculate checksum of ref /templates: not found"
  - "docker build -f Dockerfile.agent fails with: ERROR failed to compute cache key: failed to calculate checksum of ref /docs: not found"
  - "mika-agent Docker image cannot be built from a clean checkout"
root_cause: config_error
resolution_type: config_change
severity: high
tags:
  - dockerfile
  - docker-build
  - copy-directive
  - build-context
  - dockerignore
  - include-str
  - build-rs
  - cloud-deploy
---

# Dockerfile.agent broken COPY for non-existent repo-root paths

## Problem

`Dockerfile.agent` contained two `COPY` directives (`COPY templates/ templates/` and `COPY docs/ docs/`) referencing paths that do not exist in the Docker build context. This caused every `docker build -f Dockerfile.agent` to fail at the BuildKit cache-key computation step, blocking the mika-agent image build and the first cloud deploy (Mika Prime).

## Symptoms

- `docker build -f Dockerfile.agent .` fails immediately with BuildKit checksum errors for `/templates` and `/docs`
- The error occurs before `cargo build` runs — no compilation is attempted
- The failure is deterministic on every clean checkout

## What Didn't Work

This was a latent regression, not an active debugging session. The broken lines were present since the dashboard reorg or the `docs/` `.dockerignore` exclusion. No prior fix attempts — the issue was discovered during the first cloud-deploy readiness audit (2026-05-22).

## Solution

Remove both broken `COPY` lines from `Dockerfile.agent`:

```diff
 COPY crates/mika-gateway/Cargo.toml crates/mika-gateway/Cargo.toml
 RUN mkdir -p crates/mika-gateway/src && echo "fn main() {}" > crates/mika-gateway/src/main.rs \
     && mkdir -p crates/mika-gateway/migrations && touch crates/mika-gateway/migrations/.keep
-COPY templates/ templates/
-COPY docs/ docs/
 COPY --from=dashboard-builder /app/dashboard/dist dashboard/dist
```

## Why This Works

Both `COPY` lines were dead code — they referenced paths that either don't exist or are excluded from the build context:

1. **`COPY templates/ templates/`** — No `templates/` directory exists at the repo root. The only templates are at `crates/mika-agent/templates/`, already copied by `COPY crates/mika-agent/ crates/mika-agent/`. The `include_str!` call sites in `bundled_skills.rs` and `executor.rs` use relative paths that resolve inside the crate, not at root.

2. **`COPY docs/ docs/`** — The root `docs/` is excluded by `.dockerignore` (pattern `docs/`). Even without the exclusion, this would be redundant: `build.rs` has a two-tier fallback that uses `crates/mika-agent/docs/` when workspace-root `docs/` is unavailable. All doc references use `include_str!(concat!(env!("OUT_DIR"), "/docs/..."))`, which embeds at compile time — the runtime image never needs source docs.

The existing `COPY crates/mika-agent/ crates/mika-agent/` already provides everything the compiler needs (templates and crate-local docs). The removed lines were vestigial from before the current build.rs fallback and `.dockerignore` configuration.

## Prevention

- **Keep `.dockerignore` and `COPY` directives in sync.** When adding a `.dockerignore` exclusion, grep `Dockerfile.*` for `COPY` directives referencing the excluded path. When adding a `COPY`, verify the source exists in the build context (not excluded by `.dockerignore`).
- **Add a CI job for Dockerfile builds.** There is currently no `docker-build-agent` job in `ci.yml`, which is why this regression went undetected. Tracked as a separate scope item.
- **Understand the `build.rs` fallback contract.** `crates/mika-agent/build.rs` resolves docs from workspace root first, falling back to `crates/mika-agent/docs/`. In Docker builds, the workspace-root path is dockerignored, so the fallback is always used. This is the supported path — don't add root-level `COPY docs/` to "fix" it.

## Related Issues

- [mika#1237](https://github.com/senara-solutions/mika/issues/1237) — Original issue
- `docs/solutions/build-errors/rust-embed-out-dir-crate-boundary.md` — Related: documents the `build.rs` → `OUT_DIR` → `include_str!()` pattern that makes root-level `COPY docs/` unnecessary
- `docs/solutions/architecture-patterns/docker-buildkit-cache-mounts-compose.md` — Documents the `Dockerfile.agent` structure and BuildKit cache mount pattern
