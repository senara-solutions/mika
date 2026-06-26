---
title: "Scope expensive CI matrix jobs to their own path filter, not a shared one"
date: 2026-06-26
category: ci-cd
module: ci
component: github-actions
tags: [ci, github-actions, paths-filter, dorny, docker-build, matrix, picomatch]
problem_type: best_practice
track: knowledge
applies_when: "A CI matrix mixes cheap and expensive jobs behind a single path-filter output, or you are adding an hours-long build job to an existing matrix."
related:
  - senara-solutions/mika#1484
  - docs/solutions/ci-cd/docker-build-matrix-uppercase-tag-2026-06-01.md
---

# Scope expensive CI matrix jobs to their own path filter, not a shared one

## Context

`.github/workflows/ci.yml` had a single `docker-build-filter` job (`dorny/paths-filter@v3`)
emitting one boolean output, `docker-relevant`, that triggered on a broad set of paths:
`crates/**`, `Cargo.toml`, `Cargo.lock`, `.github/workflows/ci.yml`, plus the Dockerfiles.
That one boolean gated the **entire** 7-target `docker-build` matrix — including 5
`os/Dockerfile` Gentoo stage3 builds that take **hours** each.

Because `crates/**` was in the shared filter, every routine code PR tripped all 5 slow os
builds even though `os/Dockerfile` was untouched. Observed on PRs #1570 and #1578: both
changed only `crates/**`, and both sat with the 5 os-build checks pending 2h+ with zero
merge-gate value (the os image is unaffected by a Rust source change).

## Guidance

When a CI matrix mixes cheap and expensive jobs, **do not gate them with one shared path
filter.** The expensive jobs silently inherit the *broadest* trigger path in the filter set.

Split the work:
1. Give the path-filter job **one output per cost class** (e.g. `images-relevant` for fast
   image builds, `os-relevant` for the slow OS builds), each listing only the paths that
   genuinely affect that class.
2. Split the matrix into **separate jobs**, each gated on its own filter output via
   `needs.<filter-job>.outputs.<name> == 'true'`.

```yaml
docker-build-filter:
  outputs:
    images-relevant: ${{ steps.filter.outputs.images-relevant }}
    os-relevant: ${{ steps.filter.outputs.os-relevant }}
  steps:
    - uses: dorny/paths-filter@v3
      id: filter
      with:
        filters: |
          images-relevant:        # fast agent/gateway Rust images
            - 'Dockerfile.agent'
            - 'Dockerfile.gateway'
            - 'Cargo.toml'
            - 'Cargo.lock'
            - 'crates/**'
            - '.github/workflows/ci.yml'
          os-relevant:             # slow Gentoo os/Dockerfile builds — narrow trigger
            - 'os/**'
            - '**/Dockerfile*'

docker-build:        # 2 fast targets
  if: needs.docker-build-filter.outputs.images-relevant == 'true'
docker-build-os:     # 5 slow os targets
  if: needs.docker-build-filter.outputs.os-relevant == 'true'
```

## Why This Matters

A shared broad filter forces the most expensive jobs to run far more often than their
merge-gate value justifies — burning runner time and leaving PRs in a visually-blocked
"pending" state for hours when the slow job has no bearing on the change. Scoping each cost
class to its own narrow filter means the expensive job runs **only** when its inputs
actually change. The fast jobs keep their broad trigger (a `crates/**` change genuinely can
break the agent/gateway image builds, which is real pre-merge value).

## When to Apply

- Any GitHub Actions matrix that mixes sub-minute jobs with multi-minute/hour jobs behind a
  single `dorny/paths-filter` (or `if:` path condition) output.
- Adding a new long-running build target to an existing matrix — give it its own filter
  output rather than appending it under the existing one.

## Examples

**Glob semantics (`dorny/paths-filter` uses [picomatch], verified empirically):**

| Changed path | `os/**` | `**/Dockerfile*` |
|--------------|---------|------------------|
| `crates/mika-agent/src/x.rs` | no | no |
| `os/Dockerfile` | yes | yes |
| `os/config/x.conf` | yes | no |
| `Dockerfile.agent` (repo root) | no | **yes** |

`os/**` matches anything under `os/`. `**/Dockerfile*` matches root-level Dockerfiles too —
picomatch's `**/` globstar matches **zero or more** leading path segments, so a root file
like `Dockerfile.agent` still matches `**/Dockerfile*`. The `**/Dockerfile*` arm here is a
conservative safety net: when any Dockerfile changes, rebuilding the full os matrix is cheap
insurance, and Dockerfile edits are rare. Keep `.github/workflows/ci.yml` **out** of the
expensive filter — workflow edits are frequent and should not re-trigger hours of builds.

[picomatch]: https://github.com/micromatch/picomatch
