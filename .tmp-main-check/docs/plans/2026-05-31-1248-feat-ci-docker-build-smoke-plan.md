# Plan: feat(ci): add docker build smoke test to ci.yml

**Ticket:** mika issue#1248
**Date:** 2026-05-31 (revised 2026-06-03)

## Problem

Four Dockerfile bugs shipped to main between 2026-05-19 and 2026-05-22 (mika#1237, #1240, #1242, #1243) — all discoverable by a `docker build` in CI.

## Current State (post-#1330)

PR #1330 and follow-ups (#1353, #1360, #1366) already added a `docker-build` job to `ci.yml`. The current job:

- ✅ Builds `Dockerfile.agent` and `Dockerfile.gateway` via matrix strategy
- ✅ Uses BuildKit via `docker/setup-buildx-action`
- ✅ Uses path filtering via `dorny/paths-filter` (`docker-build-filter` job)
- ❌ Does NOT build `os/Dockerfile` targets (`mika-os`, `mika-runtime`)
- ❌ No BuildKit layer caching (raw `docker buildx build` without `--cache-from`/`--cache-to`)
- ❌ Path filter does not include `os/` paths
- ❌ `CLAUDE.md` CI/CD section not updated (AC6)
- ❌ Branch protection rule not updated (AC5 — operator action)

## Remaining Work

This plan covers only the delta between what exists and the full AC set.

### Step 1: Add `os/Dockerfile` targets to the matrix

**File:** `.github/workflows/ci.yml`

Extend the `docker-build` job's matrix to include the two `os/Dockerfile` targets. The matrix currently has:

```yaml
matrix:
  dockerfile: [Dockerfile.agent, Dockerfile.gateway]
```

Replace with an include-style matrix that supports the `target` parameter needed for `os/Dockerfile`:

```yaml
strategy:
  matrix:
    include:
      - dockerfile: Dockerfile.agent
        tag: mika-agent
      - dockerfile: Dockerfile.gateway
        tag: mika-gateway
      - dockerfile: os/Dockerfile
        target: mika-os
        tag: mika-os
      - dockerfile: os/Dockerfile
        target: mika-runtime
        tag: mika-runtime
```

Update the build step to use the matrix target (when set):

```yaml
- name: Build ${{ matrix.tag }}
  run: |
    docker buildx build \
      -f ${{ matrix.dockerfile }} \
      ${{ matrix.target && format('--target {0}', matrix.target) || '' }} \
      --platform linux/amd64 \
      --load \
      -t ${{ matrix.tag }}:ci .
```

**Tag fix note:** The existing job had a bug (#1366) with uppercase tags from `matrix.dockerfile`. The explicit `tag` field in the include matrix avoids this — each tag is a clean lowercase string.

### Step 2: Add `os/` to the path filter

**File:** `.github/workflows/ci.yml` — `docker-build-filter` job

Add `os/` paths to the `docker-relevant` filter:

```yaml
filters: |
  docker-relevant:
    - 'Dockerfile.agent'
    - 'Dockerfile.gateway'
    - 'os/Dockerfile'
    - 'os/Dockerfile.dockerignore'
    - 'Cargo.toml'
    - 'Cargo.lock'
    - 'crates/**'
    - '.github/workflows/ci.yml'
```

### Step 3: Add BuildKit layer caching (AC4)

**File:** `.github/workflows/ci.yml` — `docker-build` job

Replace raw `docker buildx build` with `docker/build-push-action` for integrated GHA cache support:

```yaml
- name: Build ${{ matrix.tag }}
  uses: docker/build-push-action@<pinned-sha>  # v6
  with:
    context: .
    file: ${{ matrix.dockerfile }}
    target: ${{ matrix.target || '' }}
    tags: ${{ matrix.tag }}:ci
    push: false
    load: true
    cache-from: type=gha,scope=${{ matrix.tag }}
    cache-to: type=gha,scope=${{ matrix.tag }},mode=max
    platforms: linux/amd64
```

**Key decisions:**

1. **Per-matrix-leg cache scope** (`scope=${{ matrix.tag }}`): Each Dockerfile gets its own GHA cache namespace. Without `scope`, all 4 legs would collide on the default cache key, causing eviction thrash.

2. **`mode=max`**: Exports all intermediate layers, not just the final image. Important because the Rust `cargo build --release` layer is the expensive one (~15-20min cold) and sits mid-Dockerfile.

3. **GHA cache backend** over registry-backed: Simpler (no GHCR auth), free within 10GB repo cache budget, sufficient for smoke tests. Switch to `type=registry` if cache pressure becomes an issue.

4. **`push: false` + `load: true`**: Build-only, no registry push. The image is loaded into the runner's Docker daemon for the build to complete the `--load` path (validates the full image is well-formed).

Pin `docker/build-push-action` to a commit SHA (v6.x latest stable), matching the existing convention for `actions/checkout` and `docker/setup-buildx-action`.

### Step 4: Update CLAUDE.md CI documentation (AC6)

**File:** `CLAUDE.md` (repo root)

In the "Architecture Summary" section under "CI/CD:", the sentence currently lists `byte-slice-lint`, `loop-select-lint`, `docs-sync`. Add `docker-build` to this list:

> CI includes [...] a `docker-build` job that builds all Dockerfiles (agent, gateway, mika-os, mika-runtime) on every PR to catch structural bugs before merge.

### Step 5: Post-merge operator action (AC5)

This is a **post-merge operator action** — not a code change. After the PR lands, the repo admin adds `Docker Build` (the job name) to the required status checks in the `senara-solutions/mika` branch protection rule for `main`.

The PR body MUST include:

```
## Post-merge operator action (AC5)

Add `Docker Build` to required status checks on the `main` branch protection rule.
Path: Settings → Branches → main → Required status checks → Add `Docker Build`.
Required to satisfy AC5; without it the gate is advisory-only.
```

**Note on path-filtered required checks:** The `docker-build` job is gated behind `docker-build-filter` (path filter). GitHub branch protection treats a skipped required check as "not satisfied" unless the ruleset is configured with "Do not require status checks on creation" or the job is listed as a non-required check that is simply expected when present. Two options:

- **Option A (recommended):** Make `docker-build-filter` always run and `docker-build` always run (remove path filtering). The build is ~5min cached, acceptable for all PRs. Simplest to reason about.
- **Option B:** Keep path filtering but use the `paths-filter` action's `skip-duplicate-actions` approach with a "success if skipped" wrapper job. More complex but saves CI minutes on docs-only PRs.

The implementor should choose based on CI budget constraints. Option A is the default recommendation.

## Scope

### In scope
- Extend existing `docker-build` matrix with `os/Dockerfile` targets (mika-os, mika-runtime)
- Add `os/` paths to the `docker-build-filter`
- Add GHA layer caching via `docker/build-push-action`
- CLAUDE.md CI section update
- Post-merge operator instruction for branch protection

### Out of scope
- Registry publishing (build-only)
- Multi-arch builds (arm64 + amd64)
- Runtime/functional verification (`docker run`)
- Per-role split of `mika-runtime` (deferred to mika#1247; AC7 covers the target-list update when that lands)

## Risk Assessment

- **Low risk.** Extends an existing job — no new job, no modification to unrelated jobs.
- **CI budget:** 4 matrix legs × ~5min cached = ~20min total, but they run in parallel so wall-clock is ~5min warm. Cold builds (Gentoo stage3 for os/Dockerfile) may hit ~30min.
- **Cache eviction:** GHA cache is 10GB per repo. Four separate scopes may pressure the budget. Monitor after merge; switch to `mode=min` or registry cache if needed.

## AC Traceability

| AC | Status | Covered by |
|----|--------|------------|
| AC1 | ✅ Done (#1330) | `docker-build` job exists |
| AC2 | ⚠️ Partial | Step 1 — add os/Dockerfile targets to complete all 4 builds |
| AC3 | ✅ Done (#1330) | BuildKit via `docker/setup-buildx-action` |
| AC4 | ❌ Open | Step 3 — GHA cache backend via `docker/build-push-action` |
| AC5 | ❌ Open | Step 5 — post-merge operator instruction |
| AC6 | ❌ Open | Step 4 — CLAUDE.md update |
| AC7 | N/A | Future work — updates target list when mika#1247 splits per-role |
