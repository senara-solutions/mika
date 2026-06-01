---
module: ci-cd
tags: [github-actions, docker, buildx, matrix, ci-drift]
problem_type: ci-failure
category: ci-cd
---

# Docker Build matrix produced an uppercase (invalid) image tag

## Problem

`ci.yml`'s `docker-build` job runs a matrix over `dockerfile: [Dockerfile.agent, Dockerfile.gateway]`
and tagged the load-only test build with `-t test-${{ matrix.dockerfile }}`. That interpolates to
`test-Dockerfile.agent` / `test-Dockerfile.gateway` — **invalid**, because Docker repository names
must be lowercase. Every PR that triggered the Docker Build matrix failed in ~10s with:

```
ERROR: failed to build: invalid tag "test-Dockerfile.agent": repository name must be lowercase
```

Introduced by `29db043e` (mika#1353, 2026-05-31). It broke **every** mika PR's Docker Build job
until mika#1366 — the paths-filter work in #1360/#1361 was orthogonal (the filter passed; the build
itself was broken).

## Fix

Use a static lowercase tag — the build is `--load` only (never pushed), the tag is never referenced
by any downstream step, and matrix legs run on isolated runners (separate VMs), so a shared tag
cannot collide:

```yaml
run: docker buildx build -f ${{ matrix.dockerfile }} --platform linux/amd64 --load -t test-image .
```

## Lesson

When a CI tag/name is derived from a value that is **not guaranteed lowercase** (matrix keys,
filenames, branch names, env values), either hardcode a known-valid token or pipe through
`tr '[:upper:]' '[:lower:]'`. Docker tag-validity failures surface only at build time, never at
workflow-parse time — so a green YAML lint says nothing about tag legality. Prefer a static tag for
load-only builds that are never pushed or re-referenced.
