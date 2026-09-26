---
module: ci-cd
tags: [docker, dashboard, npm-workspace, vite, packages-ui, ci-drift]
problem_type: build-failure
category: ci-cd
---

# Dockerfile.agent dashboard build failed — workspace UI package never built

## Problem

`Dockerfile.agent`'s `dashboard-builder` stage failed: the dashboard's `tsc -b` could not
resolve `@senara-solutions/ui`, with `TS2307: Cannot find module '@senara-solutions/ui'`
across ~40 files. The stage ran:

```dockerfile
RUN npm ci --ignore-scripts && npm run build --prefix dashboard
```

This was **masked** until mika#1366 (the uppercase Docker-tag bug) was fixed — the invalid-tag
error aborted the build before this stage ever ran, so nobody saw it.

## Root cause

`@senara-solutions/ui` is a local npm-workspace package (`packages/ui/`). Its `package.json`
points the import at **built** output (`main: ./dist/index.js`, `types: ./dist/index.d.ts`).
The dashboard's build is `tsc -b && vite build`, and `tsc -b` needs `packages/ui/dist/` to exist.

The stage built only the dashboard, never `packages/ui` — and `npm ci --ignore-scripts`
suppresses any lifecycle/`prepare` hook that might have built it. So `packages/ui/dist/` was
absent → `TS2307`.

The canonical local order already builds it first (root `package.json`):
```json
"dev:dashboard": "npm run build --workspace=packages/ui && npm run dev --workspace=dashboard"
```
The Dockerfile simply didn't mirror it.

## Fix

Build the UI workspace package before the dashboard:

```dockerfile
RUN npm ci --ignore-scripts \
 && npm run build --workspace=packages/ui \
 && npm run build --prefix dashboard
```

Verified with a real `docker build -f Dockerfile.agent --target dashboard-builder .`:
`packages/ui` builds (`dist/index.js`, declaration files), then the dashboard `tsc -b && vite
build` succeeds and the stage exports cleanly.

## Lesson

When a Docker stage builds a consumer of a local workspace package that resolves to compiled
output, it must build the producer package first — `npm ci --ignore-scripts` will NOT do it via
a prepare hook. Mirror the repo's canonical build order (root `package.json` scripts) inside the
Dockerfile rather than assuming install side-effects produce `dist/`.
