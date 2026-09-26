# Plan: fix(dockerfile-gateway): add missing mika-a2a workspace member stub

**Ticket:** mika#1330
**Type:** bug fix
**Risk:** low — Dockerfile-only changes + CI addition, no Rust code changes

## Problem

`Dockerfile.gateway` fails to build because it stubs `mika-agent` (lines 14–17) but doesn't stub `mika-a2a`, which `mika-agent` path-depends on. Cargo cannot resolve the workspace graph since `crates/mika-a2a/Cargo.toml` is never copied into the build context.

This is the third Dockerfile-drift bug (after mika#1237 and mika#1329). The root cause is that workspace member stubs are maintained manually and no CI job validates that the Dockerfiles actually build.

## Dependency analysis

Workspace: `members = ["crates/*"]` → 5 crates: `mika-a2a`, `mika-agent`, `mika-cli`, `mika-common`, `mika-gateway`.

`Dockerfile.gateway` builds only `mika-gateway`. It needs:
- **Full copy:** `mika-common` (runtime dep), `mika-gateway` (the target)
- **Stub:** everything else so `cargo` can parse the workspace graph

Current stubs: `mika-agent` only. Missing: `mika-a2a`, `mika-cli`.

`mika-a2a` has no path dependencies (only workspace deps like serde, uuid, chrono). `mika-cli` has no path dependencies beyond its bin target. Both are safe to stub with `Cargo.toml` + empty `src/lib.rs` or `src/main.rs`.

## Implementation

### Step 1: Fix `Dockerfile.gateway` stubs (the fix)

Add stubs for `mika-a2a` and `mika-cli` alongside the existing `mika-agent` stub. Group all stubs together with a clear comment explaining the pattern:

```dockerfile
# Dummy stubs for workspace members not being built
# (cargo needs their Cargo.toml to resolve the workspace graph)
COPY crates/mika-agent/Cargo.toml crates/mika-agent/Cargo.toml
RUN mkdir -p crates/mika-agent/src/bin && echo "fn main() {}" > crates/mika-agent/src/bin/mika-spirit.rs \
    && echo "" > crates/mika-agent/src/lib.rs \
    && mkdir -p crates/mika-agent/src && echo "fn main() {}" > crates/mika-agent/src/cli.rs

COPY crates/mika-a2a/Cargo.toml crates/mika-a2a/Cargo.toml
RUN mkdir -p crates/mika-a2a/src && echo "" > crates/mika-a2a/src/lib.rs

COPY crates/mika-cli/Cargo.toml crates/mika-cli/Cargo.toml
RUN mkdir -p crates/mika-cli/src && echo "fn main() {}" > crates/mika-cli/src/main.rs
```

**Why stub all three (not just `mika-a2a`):** `mika-cli` is currently not stubbed either — it just hasn't triggered a build failure yet because no existing stub transitively depends on it. Adding it now prevents the next drift bug when any crate adds a `mika-cli` dependency.

**Files:** `Dockerfile.gateway`

### Step 2: Add CI docker-build smoke test

Add a `docker-build` job to `.github/workflows/ci.yml` that validates both Dockerfiles parse and build successfully. This prevents the entire class of Dockerfile-drift bugs (3 incidents so far).

The job should:
- Run on PRs only (not push to main — actual images are built by the release workflow)
- Use `docker build` with `--target builder` to validate the Rust compilation stage without building the full runtime image (faster, no need to pull runtime base)
- Build both `Dockerfile.agent` and `Dockerfile.gateway`
- Use BuildKit cache

```yaml
docker-build:
  name: Docker Build
  runs-on: ubuntu-22.04
  if: github.event_name == 'pull_request'
  strategy:
    matrix:
      dockerfile: [Dockerfile.agent, Dockerfile.gateway]
  steps:
    - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd  # v6
    - name: Set up Docker Buildx
      uses: docker/setup-buildx-action@b5ca514318bd6ebac0fb2aedd5d36ec1b5c232a2  # v3
    - name: Build ${{ matrix.dockerfile }}
      run: docker buildx build -f ${{ matrix.dockerfile }} --platform linux/amd64 --load -t test-${{ matrix.dockerfile }} .
```

Note: `Dockerfile.agent` needs the dashboard `dist/` directory. The CI already has a `dashboard` job that builds it — but rather than adding a dependency, the simpler approach is to create a placeholder `mkdir -p dashboard/dist` (same pattern as the existing `check` job line ~30). This keeps the docker-build job independent.

Actually, `Dockerfile.agent` has a multi-stage build where the first stage (`dashboard-builder`) builds the dashboard from source inside Docker, so no external `dist/` is needed — the `COPY` of `package.json`, `packages/`, and `dashboard/` directories is sufficient.

**Files:** `.github/workflows/ci.yml`

## Verification

1. `DOCKER_BUILDKIT=1 docker build -f Dockerfile.gateway --platform linux/amd64 -t mika-gateway-test .` — must complete successfully
2. Run the resulting image: `docker run --rm mika-gateway-test --help` (or similar) to verify the binary exists
3. Confirm CI docker-build job passes on the PR

## Non-goals

- Refactoring to glob-driven stubs (e.g., iterating `crates/*/Cargo.toml`) — adds Dockerfile complexity for marginal benefit now that CI will catch drift
- Fixing `Dockerfile.agent` — already has all stubs correct (confirmed by reading it)
- Restructuring the workspace `members` declaration — orthogonal concern
