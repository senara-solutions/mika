---
title: "GitHub Packages npm has no anonymous read path — publish open-core scoped packages to npmjs.org"
date: 2026-06-26
problem_type: tooling_decision
category: tooling-decisions
module: packages/ui
component: ci-publish
tags:
  - npm
  - github-packages
  - npmjs
  - registry
  - publishConfig
  - npmrc
  - open-core
  - ci-cd
applies_when: "Publishing a scoped npm package (@scope/name) that you want installable without a token, especially for open-core or public consumption."
related_issue: "senara-solutions/mika#1386"
---

# GitHub Packages npm has no anonymous read path — publish open-core scoped packages to npmjs.org

## Context

`@senara-solutions/ui` (the shared dashboard component library) was published to **GitHub Packages**
(`npm.pkg.github.com`). The package was nominally "public", but a fresh `mika-cloud/web` checkout could
not `npm ci` / `npm run build` without setting a token — `tsc` failed with `Cannot find module
'@senara-solutions/ui'`, and the local `web-typecheck` pre-commit hook blocked all web commits when the
var was unset. This contradicts an open-core posture: a "public" shared lib that is un-installable
without auth is friction for every contributor and every machine identity.

## Guidance

**GitHub's npm registry (`npm.pkg.github.com`) returns HTTP 401 for every read when unauthenticated —
even for public packages.** This is unlike `ghcr.io` (GitHub's *container* registry), which allows
anonymous pulls for public images. There is **no anonymous read path** for GitHub Packages npm; every
consumer must set `NODE_AUTH_TOKEN` to some `read:packages` token just to install, regardless of the
package's visibility. (Verified 2026-06-03: anon metadata + tarball fetch both 401.)

If you want token-free installs (open-core, public contributors, CI machine identities), **publish to
npmjs.org as a public package** instead. Three things must change together — and the middle one is the
trap:

1. **`package.json` `publishConfig`** — point at npmjs.org and mark the scoped package public:
   ```json
   "publishConfig": {
     "registry": "https://registry.npmjs.org/",
     "access": "public"
   }
   ```
   `access: "public"` is mandatory — **scoped packages default to `restricted` on npmjs.org**, so the
   first publish fails or publishes private without it.

2. **The package's `.npmrc` scoped-registry line** — `publishConfig.registry` alone is **not
   sufficient**. npm merges `.npmrc` scoped-registry config (`@scope:registry=...`) with `publishConfig`,
   and a stale `@senara-solutions:registry=https://npm.pkg.github.com` line will keep routing scope
   resolution (and can route publish) to GitHub Packages. Retarget it:
   ```
   @senara-solutions:registry=https://registry.npmjs.org/
   ```
   This is the publish-side `.npmrc` (in the package's own repo). A *consumer* repo's `.npmrc` override
   is a separate cleanup in that repo.

3. **The publish workflow** — change `setup-node`'s `registry-url` to `https://registry.npmjs.org`, swap
   `NODE_AUTH_TOKEN` from `secrets.GITHUB_TOKEN` to an npmjs.org automation token (`secrets.NPM_TOKEN`),
   and drop the now-unused `permissions: packages: write`. Add a post-publish verification step
   (`npm view <pkg> version` against the default registry, with a short retry for propagation lag) so a
   silently-failed publish red-checks instead of looking green.

## Why This Matters

A scoped npm lib on GitHub Packages that is "public" but needs a `read:packages` token is a broken
open-core promise: it blocks fresh checkouts, contributor onboarding, and autonomous-loop machine
identities (whose tokens were separately found invalid, compounding the breakage). The asymmetry with
`ghcr.io` is the specific surprise — teams reasonably assume "GitHub Packages public = anonymous read"
because that holds for containers, and it does **not** hold for npm.

The `.npmrc` step is the subtle one. Changing only `publishConfig` looks complete (the package config
*says* npmjs.org) while a leftover scoped `.npmrc` line silently keeps GitHub Packages in the resolution
path. Always grep the whole package dir for the old registry host, not just `package.json`.

## When to Apply

- Migrating any scoped npm package off GitHub Packages for token-free / open-core consumption.
- Diagnosing `npm install`/`npm ci` 401s or `Cannot find module '@scope/...'` failures that only
  reproduce without a token set.
- Auditing whether a "public" scoped package is *actually* anonymously installable.

## Trigger-timing gotcha (publish-on-merge, not on tag)

`packages/ui/`'s publish workflow (`.github/workflows/publish-ui.yml`) triggers on **push to `main`
filtered by `paths: packages/ui/**`**, *not* on a version tag. So **any PR that touches `packages/ui/**`
fires the publish job on merge** — including the migration PR itself, since it edits
`packages/ui/package.json`. Consequence: operator pre-conditions (the `@senara-solutions` org scope on
npmjs.org **and** the `NPM_TOKEN` repo secret) must be in place **before the migration PR merges**, or
the publish step fails with a red check on `main`. Don't assume "the workflow only runs on a release tag"
— read the `on:` block. (Publish is non-idempotent on npmjs.org: re-publishing the same version fails, so
there is no safe automated retry — the operator diagnoses a genuine failure.)

## Related

- `docs/solutions/architecture-patterns/extract-shared-ui-package.md` — how `@senara-solutions/ui` was
  extracted; updated 2026-06-26 with the registry-migration banner.
- `senara-solutions/mika#1386` — the migration (architect-groomed, session b79eb5a3).
