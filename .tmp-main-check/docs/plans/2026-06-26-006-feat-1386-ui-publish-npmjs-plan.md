---
title: "feat(ui): publish @senara-solutions/ui to npmjs.org for token-free public installs"
date: 2026-06-26
type: feat
issue: senara-solutions/mika#1386
branch: feat/1386/ui-publish-senara-solutions-ui-to-npmjs
status: planned
depth: lightweight
---

# feat(ui): publish @senara-solutions/ui to npmjs.org for token-free public installs

## Summary

`@senara-solutions/ui` is published only to GitHub Packages (`npm.pkg.github.com`), whose npm registry returns HTTP 401 for **every** read when unauthenticated — even for public packages. So every consumer needs a `read:packages` token just to install, which contradicts mika's open-core posture and blocks fresh `mika-cloud/web` checkouts. This plan retargets the publish path to **npmjs.org** (where public = anonymous install), per the architect-authored contract (mika-arch second-pass session `b79eb5a3`, KTD-1 = Option A, npmjs.org only, drop GitHub Packages).

The change is config + CI only: a `publishConfig` edit, a workflow retarget, and a scoped-registry `.npmrc` fix. No Rust, no product runtime behavior change.

## Problem Frame

- **Where it's set:** `packages/ui/package.json` `publishConfig.registry` = `https://npm.pkg.github.com`; the publish CI (`.github/workflows/publish-ui.yml`) sets `setup-node registry-url` to the same and authenticates with `secrets.GITHUB_TOKEN`. `packages/ui/.npmrc` additionally scopes `@senara-solutions:registry` to GitHub Packages.
- **Why it hurts:** GitHub's npm registry has no anonymous read path (unlike `ghcr.io` for containers). Verified 2026-06-03: anon metadata + tarball fetch both 401; the package is not on npmjs.org (404).
- **Decision (architect KTD-1):** Option A — publish to npmjs.org only, drop GitHub Packages. Dual-publish adds a second registry and failure surface with no identified consumer requiring the GH Packages copy.

## Requirements

Governing acceptance criteria from the issue body (architect resolution F2):

- **AC1** — `packages/ui/package.json` `publishConfig` targets `https://registry.npmjs.org/` with `access: "public"`.
- **AC2** — `.github/workflows/publish-ui.yml` publishes to npmjs.org using `secrets.NPM_TOKEN`, not GitHub Packages.
- **AC3** — Post-publish verification step in the workflow: `npm view @senara-solutions/ui version` succeeds against the default registry.
- **AC4** — No workflow in the repo publishes `@senara-solutions/ui` to `npm.pkg.github.com` after this change.
- **AC5** — An unauthenticated `npm install @senara-solutions/ui` against the default registry succeeds after the first publish from the updated workflow.

**Operator pre-conditions** (NOT PR deliverables — Vincent owns these, per F2):

- The `@senara-solutions` org scope exists on npmjs.org.
- An npmjs.org automation token is stored as `NPM_TOKEN` in `senara-solutions/mika` GitHub Actions secrets.

---

## Key Technical Decisions

- **KTD-1 — npmjs.org only; drop GitHub Packages.** Carried verbatim from the architect contract. No dual-publish.
- **KTD-2 — Update `packages/ui/.npmrc`, do not just rely on `publishConfig`.** Ground-truth finding beyond the architect's U-list: `packages/ui/.npmrc` pins `@senara-solutions:registry=https://npm.pkg.github.com`. `npm` merges `.npmrc` scoped-registry config with `publishConfig`; leaving the `.npmrc` line pointed at GH Packages risks the publish CI resolving the scope to the wrong registry and undercuts AC4 (no reference to `npm.pkg.github.com` driving behavior). This `.npmrc` is **publish-side** (in the `mika` repo) and distinct from the out-of-scope `mika-cloud/web` **consumer** `.npmrc` (architect KTD-3). It must be retargeted to `https://registry.npmjs.org/`. **In scope.**
- **KTD-3 — Consumer-side `.npmrc` cleanup is out of scope** (architect KTD-3). `mika-cloud/web`'s `.npmrc` override is a separate follow-up PR in `mika-cloud`, not this PR.
- **KTD-4 — Do NOT change the workflow trigger.** Changing `on:` to a tag trigger is outside the contract; the existing `push`-to-`main`-on-`paths` trigger is preserved. The merge-timing consequence is surfaced as an operator risk below, not silently engineered around.
- **KTD-5 — `packages: write` permission stays or is harmless either way.** The workflow's `permissions: packages: write` was for GitHub Packages. npmjs.org publish authenticates via `NODE_AUTH_TOKEN`/`NPM_TOKEN`, not the `GITHUB_TOKEN` workflow permission, so the `packages: write` grant is now unused. Removing it tightens least-privilege; the architect U-list did not call for it. Decision: drop it to `contents: read` only, since no step needs `packages: write` after the migration (supports AC4's spirit — nothing GitHub-Packages-shaped remains). Low-risk, additive-subtractive cleanup within the same file the contract already mandates editing.

---

## Implementation Units

### U1. Retarget `packages/ui/package.json` publishConfig

- **Goal:** AC1 — publish destination is npmjs.org, scoped package is public.
- **Requirements:** AC1.
- **Dependencies:** none.
- **Files:** `packages/ui/package.json`.
- **Approach:** In the existing `publishConfig` block (file tail), change `registry` from `https://npm.pkg.github.com` to `https://registry.npmjs.org/` and add `"access": "public"`. Scoped packages default to `restricted` on npmjs.org — without `access: public` the first publish fails or publishes private.
- **Patterns to follow:** existing JSON structure; preserve key ordering/style.
- **Test scenarios:** `Test expectation: none — pure package metadata change, validated by the workflow's publish + post-publish view (AC3/AC5) at release time, not by unit tests.`
- **Verification:** `node -p "require('./packages/ui/package.json').publishConfig"` shows `{ registry: 'https://registry.npmjs.org/', access: 'public' }`.

### U2. Retarget the publish workflow to npmjs.org

- **Goal:** AC2 + AC3 — workflow publishes to npmjs.org via `NPM_TOKEN` and verifies public visibility post-publish.
- **Requirements:** AC2, AC3, AC4.
- **Dependencies:** U1 (publishConfig must agree with the workflow registry).
- **Files:** `.github/workflows/publish-ui.yml`.
- **Approach:**
  1. `setup-node` `registry-url`: `https://npm.pkg.github.com` → `https://registry.npmjs.org`.
  2. "Check if version changed" step: drop the `--registry https://npm.pkg.github.com` flag (defaults to npmjs.org now); change its `env.NODE_AUTH_TOKEN` from `secrets.GITHUB_TOKEN` to `secrets.NPM_TOKEN`. (The `npm view` read is anonymous on npmjs.org, but keeping the token env consistent across steps avoids a future gotcha.)
  3. "Publish" step: change `env.NODE_AUTH_TOKEN` from `secrets.GITHUB_TOKEN` to `secrets.NPM_TOKEN`.
  4. Add a "Verify publish" step (runs only when publish ran, i.e., `if: steps.version.outputs.skip != 'true'`) that runs `npm view @senara-solutions/ui version` against the default registry; a non-zero exit fails the run (AC3). Account for npmjs.org propagation delay per architect F3 (e.g., a short retry/sleep before the view) so a transient propagation miss doesn't red-check a healthy publish.
  5. `permissions:` — drop `packages: write` (KTD-5), leaving `contents: read`.
- **Patterns to follow:** existing step structure and SHA-pinned actions (`actions/checkout`, `actions/setup-node` stay pinned).
- **Test scenarios:** `Test expectation: none — GitHub Actions workflow; behavior is exercised by the real publish on the next packages/ui change. No local unit harness for workflow YAML beyond lint/shape.`
- **Verification:** `grep -c "npm.pkg.github.com" .github/workflows/publish-ui.yml` returns 0; `grep "registry.npmjs.org" .github/workflows/publish-ui.yml` present; `secrets.NPM_TOKEN` referenced in both auth'd steps; a "Verify publish" step exists.

### U3. Retarget the publish-side scoped `.npmrc`

- **Goal:** AC4 — no `npm.pkg.github.com` reference remains driving publish-side scope resolution.
- **Requirements:** AC4, AC5.
- **Dependencies:** none (independent of U1/U2; landed together for a coherent AC4).
- **Files:** `packages/ui/.npmrc`.
- **Approach:** Change `@senara-solutions:registry=https://npm.pkg.github.com` to `@senara-solutions:registry=https://registry.npmjs.org/`. (Retarget rather than delete — keeping the explicit scope line documents the intended registry and avoids surprising future installs that read this `.npmrc`.)
- **Patterns to follow:** single-line `.npmrc` scope directive.
- **Test scenarios:** `Test expectation: none — registry config; covered by AC5's anonymous-install check post-publish.`
- **Verification:** `grep "npm.pkg.github.com" packages/ui/.npmrc` returns nothing; the file points the scope at npmjs.org.

---

## Scope Boundaries

**In scope:** U1, U2, U3 — all publish-side config/CI in the `mika` repo.

### Deferred to Follow-Up Work
- **`mika-cloud/web` consumer `.npmrc` cleanup** — remove the `@senara-solutions:registry=https://npm.pkg.github.com` override so consumers install token-free (architect KTD-3). Separate PR in `mika-cloud`.
- **Version drift** — `mika packages/ui` is at 0.3.1; `mika-cloud/web` pins `^0.2.0`. Out of scope (issue "Out of scope").
- **Machine-user `read:packages` token refresh** for the autonomous loop — separate substrate fix (issue "Out of scope").

### Non-Goals
- Changing the workflow `on:` trigger (KTD-4).
- Dual-publishing to GitHub Packages (KTD-1 rejected it).

---

## Risks & Operator Sequencing

- **R1 (HIGH — merge timing) — Verified contradiction of the dispatch summary.** The dispatch note claimed the workflow "fires only on version tag push." **It does not.** `publish-ui.yml` triggers on `push` to `main` filtered by `paths: packages/ui/**`. This PR modifies `packages/ui/package.json` and `packages/ui/.npmrc` (both match the path filter), so **merging this PR will immediately fire the publish workflow.** If the operator pre-conditions (`@senara-solutions` org scope on npmjs.org + `NPM_TOKEN` secret) are not in place at merge time, the publish step fails with a red check on `main`. The "Check if version changed" step compares against npmjs.org where 0.3.1 does not exist → it will attempt to publish. **Mitigation / required sequencing:** Vincent must (1) create the npmjs.org org scope and (2) add the `NPM_TOKEN` secret **before merging this PR.** This is called out in the PR body as a merge gate. Do not silently change the trigger to avoid this — flagging it is the contract-faithful disposition.
- **R2 (LOW) — `access: public` omission** would publish the scope as restricted/private or fail the first publish. Mitigated by U1 explicitly adding `access: public` (AC1) and AC5's anonymous-install check.
- **R3 (LOW) — npmjs.org propagation delay** can make the post-publish `npm view` (AC3) miss a just-published version and red-check a healthy publish. Mitigated by a short retry/sleep in the Verify step (architect F3). No automated republish — publish is non-idempotent on npmjs.org; the operator diagnoses a genuine failure.

---

## Acceptance criteria

The functional ACs (AC5 anonymous install; AC3 post-publish view) can only be fully proven by the **first real publish** from the updated workflow, which is gated on the operator pre-conditions. At PR-review time, verification is structural:

- AC1: `publishConfig` shows npmjs.org + `access: public`.
- AC2: workflow auth uses `secrets.NPM_TOKEN`; `registry-url` is npmjs.org.
- AC3: a Verify-publish step running `npm view ... version` exists and fails the run on error.
- AC4: zero `npm.pkg.github.com` references remain across `.github/`, `packages/ui/package.json`, `packages/ui/.npmrc`.

AC3/AC5 close on the first post-merge publish (operator-gated).
