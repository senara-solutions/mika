# Plan: feat(ci): add docker build smoke test to ci.yml

**Ticket:** mika issue#1248
**Date:** 2026-05-31

## Problem

Four Dockerfile bugs shipped to main between 2026-05-19 and 2026-05-22 (mika#1237, #1240, #1242, #1243) — all discoverable by a `docker build` in CI. Currently `ci.yml` runs Rust and Node builds but never exercises Dockerfiles.

## Phase 0 — Pin (verbatim source citations)

The mika repo lives at `mika-platform/mika/` from the workspace root. Within the worktree (which IS the mika repo), paths are root-relative:

- **`Dockerfile.agent`** — at mika repo root. Verified: `/home/samidarko/workspace/mika-platform/mika/Dockerfile.agent` exists.
- **`Dockerfile.gateway`** — at mika repo root. Verified.
- **`os/Dockerfile`** — at `os/Dockerfile` from worktree, **NOT** `mika/os/Dockerfile`. The issue body's `mika/os/Dockerfile` is workspace-relative; from the worktree (mika repo root) it is `os/Dockerfile`. Verified: `/home/samidarko/workspace/mika-platform/mika/os/Dockerfile` exists with `mika-os` and `mika-runtime` targets (shipped by mika#1243, closed).
- **`CLAUDE.md`** (worktree root) is the **same file** as `mika/CLAUDE.md` (workspace-relative). Issue AC6 cites `mika/CLAUDE.md`; from the worktree it's the root `CLAUDE.md`. Single source — no path conflict.

These pins resolve the F1/F3 ambiguity from the first-pass architect review.

## Approach

Add a `docker-build-smoke` job to `.github/workflows/ci.yml` that runs `docker build` for each Dockerfile on every PR and push to main. Use BuildKit with GitHub Actions cache for layer caching.

## Scope

### In scope
- New CI job building Dockerfile.agent, Dockerfile.gateway, and both `os/Dockerfile` targets (`mika-os` and `mika-runtime`) — all 4 AC2 builds
- BuildKit layer caching via `docker/build-push-action` + GHA cache backend
- CLAUDE.md CI section update (worktree root `CLAUDE.md` = workspace `mika/CLAUDE.md`)
- Post-merge operator instruction for AC5 (branch protection rule update — repo admin action, documented in PR body)

### Out of scope
- Registry publishing (build-only, no push)
- Multi-arch builds
- Runtime/functional verification (`docker run`)
- Per-role split of `mika-runtime` (deferred to mika#1247; AC7 covers the target-list update when that lands)

## Implementation Steps

### Step 1: Add `docker-build-smoke` job to `ci.yml`

**File:** `.github/workflows/ci.yml`

Add a new job `docker-build-smoke` after the existing jobs. Structure:

```yaml
docker-build-smoke:
  name: Docker Build Smoke
  runs-on: ubuntu-22.04
  if: >-
    github.event_name == 'push' ||
    (github.event_name == 'pull_request' &&
     !startsWith(github.head_ref, 'release/') &&
     !startsWith(github.head_ref, 'release-please--'))
  steps:
    - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd  # v6

    - name: Set up Docker Buildx
      uses: docker/setup-buildx-action@<pinned-sha>

    - name: Build agent image
      uses: docker/build-push-action@<pinned-sha>
      with:
        context: .
        file: Dockerfile.agent
        tags: mika-agent:ci
        push: false
        cache-from: type=gha
        cache-to: type=gha,mode=max

    - name: Build gateway image
      uses: docker/build-push-action@<pinned-sha>
      with:
        context: .
        file: Dockerfile.gateway
        tags: mika-gateway:ci
        push: false
        cache-from: type=gha
        cache-to: type=gha,mode=max

    - name: Build mika-os target
      uses: docker/build-push-action@<pinned-sha>
      with:
        context: .
        file: os/Dockerfile
        target: mika-os
        tags: mika-os:ci
        push: false
        cache-from: type=gha
        cache-to: type=gha,mode=max

    - name: Build mika-runtime target
      uses: docker/build-push-action@<pinned-sha>
      with:
        context: .
        file: os/Dockerfile
        target: mika-runtime
        tags: mika-runtime:ci
        push: false
        cache-from: type=gha
        cache-to: type=gha,mode=max
```

**Path note (per Phase 0 Pin):** The mika-os/mika-runtime targets are in `os/Dockerfile` from the worktree (mika repo root), NOT `mika/os/Dockerfile`. The latter is workspace-relative; the worktree IS the mika repo.

**Key decisions:**

1. **`docker/build-push-action`** instead of raw `docker build` — this is the standard GHA action for BuildKit builds, handles `DOCKER_BUILDKIT=1` implicitly, and integrates with GHA cache backend (`type=gha`). All actions must be pinned to commit SHAs per repo convention.

2. **GHA cache backend (`type=gha`)** instead of registry-backed cache — GHA cache is simpler (no GHCR auth needed), free within the repo's cache budget (10 GB), and sufficient for PR-to-PR caching. Registry-backed cache is overkill for smoke tests that don't publish images. The `mode=max` exports all layers, not just the final image — important because the Rust build stage is the expensive layer.

3. **`if` condition** matches `docs-sync` — skip on release-please branches since those don't touch Dockerfiles and the Rust build is expensive (~5min cached, ~20min cold).

4. **`push: false`** — build-only, no registry push. The `tags` field is required by the action but only labels the local image.

5. **Two separate `build-push-action` steps** (not a matrix) — agent and gateway share the same GHA cache namespace, and running sequentially means the gateway build benefits from any shared base layers cached by the agent build. A matrix would create parallel jobs with separate cache scopes, losing this benefit.

### Step 2: Pin action SHAs

Look up the latest stable release SHAs for:
- `docker/setup-buildx-action` (v3.x)
- `docker/build-push-action` (v6.x)

Pin to commit SHAs, matching the convention used for `actions/checkout` and `actions/setup-node` in the existing workflow.

### Step 3: Update CLAUDE.md CI documentation

**File:** `CLAUDE.md` at the worktree root. This is the **same file** as `mika/CLAUDE.md` from the workspace; AC6's `mika/CLAUDE.md` and this plan's `CLAUDE.md` (root) refer to the same on-disk file (verified per Phase 0 Pin).

In the "Architecture Summary" section under "CI/CD:", update the sentence listing CI jobs to include `docker-build-smoke`:

> Four GitHub Actions workflows → still four workflows, but add `docker-build-smoke` to the listed jobs alongside `byte-slice-lint`, `loop-select-lint`, `docs-sync`.

### Step 4: Verify locally (manual)

Run the Docker builds locally to confirm they pass on the current HEAD:

```bash
docker build -f Dockerfile.agent -t mika-agent:ci .
docker build -f Dockerfile.gateway -t mika-gateway:ci .
docker build --target mika-os -f os/Dockerfile -t mika-os:ci .
docker build --target mika-runtime -f os/Dockerfile -t mika-runtime:ci .
```

### Step 5: Post-merge operator action (AC5)

This step is a **post-merge operator action** — it cannot be code-changed by a PR. After the PR lands and CI is green on `main`, the repo admin must add `docker-build-smoke` (the job name from Step 1) to the **required status checks** in the `senara-solutions/mika` branch protection rule for `main`.

Without this step, the CI gate is advisory — it surfaces Dockerfile bugs but doesn't block merges. The operator (samidarko) promoted this to p0-critical specifically because the gate must block (issue comment `IC_kwDORWsgGM8AAAABDbaxRw`).

The PR body MUST include:

```
## Post-merge operator action (AC5)

Add `docker-build-smoke` to required status checks on the `main` branch protection rule.
Path: Settings → Branches → main → Required status checks → Add `docker-build-smoke`.
This is required to satisfy AC5; without it the gate is advisory-only.
```

## Caching Analysis

The Dockerfile.agent is the expensive one:
- **Dashboard builder stage** (Node): `npm ci` + `npm run build` — cached by npm lockfile layer
- **Rust builder stage**: `cargo build --release` — the heavy lift (~15-20min cold). BuildKit `--mount=type=cache` in the Dockerfile caches `/app/target` and `/usr/local/cargo/registry` within a single build, but GHA cache (`type=gha`) caches the *layers* across builds. The layer containing `cargo build` is invalidated whenever any `COPY crates/...` layer changes (any Rust source change). This means most PRs that touch Rust code will re-build from scratch within the Docker context.

**Mitigation:** This is acceptable because:
- The job runs in parallel with `check` — it doesn't add to total CI wall-clock time
- Cold builds are ~15-20min on GitHub-hosted runners, within the 30min budget from AC4
- The primary value is catching Dockerfile structural bugs (COPY paths, stage ordering, install scripts), not providing fast feedback on Rust compilation

## Risk Assessment

- **Low risk.** Additive-only change — new job, no modification to existing jobs.
- **CI budget:** ~15-20min cold, ~5min warm. Runs in parallel with existing jobs, so doesn't extend total PR CI time unless it's the longest job.
- **Cache eviction:** GHA cache is limited to 10GB per repo. Rust Docker layers are large. If cache pressure becomes an issue, can switch to registry-backed cache or reduce `mode=max` to `mode=min`.

## Future Work (mika#1247)

mika#1247 will split `mika-runtime` into per-role targets (e.g., `mika-runtime-server`, `mika-runtime-gateway`). When that lands, the docker-build-smoke job's `--target` list is updated in the same PR per AC7. The Gentoo-based stage3 layers may need a separate caching strategy if total CI time grows beyond the 30min budget.

## AC Traceability

| AC | Covered by |
|----|------------|
| AC1 | Step 1 — `docker-build-smoke` job |
| AC2 | Step 1 — all 4 builds (agent + gateway + os/Dockerfile mika-os + os/Dockerfile mika-runtime) |
| AC3 | Step 1 — BuildKit via `docker/setup-buildx-action` |
| AC4 | Step 1 — GHA cache backend (`type=gha,mode=max`) |
| AC5 | Step 5 — post-merge operator instruction documented in PR body |
| AC6 | Step 3 — CLAUDE.md update (worktree root `CLAUDE.md` ≡ workspace `mika/CLAUDE.md`) |
| AC7 | Future work — updates target list when mika#1247 splits per-role |
