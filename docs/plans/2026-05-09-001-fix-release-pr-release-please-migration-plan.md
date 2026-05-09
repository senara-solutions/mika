---
title: "Replace homegrown release-PR bash with googleapis/release-please-action"
date: 2026-05-09
type: fix
issue: 1049
branch: fix/1049/ci-replace-homegrown-release-pr-bash
modules:
  - .github/workflows/release-pr.yml
  - release-please-config.json
  - .release-please-manifest.json
  - docs/deployment.md
  - docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md
  - docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md
tags:
  - ci-cd
  - release-automation
  - chronic-drift
---

## Problem

`.github/workflows/release-pr.yml` uses handwritten bash + `git-cliff` to manage release PRs. On the merge commit of every release PR, the workflow recreates `release/v0.12.x` from `main` — but at that point `main` and the release branch are identical, so `gh pr create` fails with `"No commits between main and release/v0.12.2"`. This is the latest in a 14+ fix chronic-drift pattern documented in `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`.

The handwritten bash is a chronic-drift generator. Each fix addresses one instance; the next merge surfaces a fresh failure mode. The right move is to delegate state management to a maintained upstream tool.

## Decision

Replace with `googleapis/release-please-action`. Reasons:

1. **Persistent Release PR** — release-please maintains a single long-lived PR branch (`release-please--branches--main`) with proper state reconciliation. No recreate-from-main pattern = no "no commits between" failure.
2. **Cargo workspace support** — `release-type: rust` handles `Cargo.toml` version bumps.
3. **Conventional-commit changelogs** — built-in, no separate `git-cliff` invocation needed.
4. **Tag creation via PAT** — release-please creates the `v*` tag on merge using the provided `token` input. The `token` input governs ALL GitHub API operations (PRs, tags, and releases) — confirmed via source code: `src/index.ts` creates a single `GitHub` client with the `token` input, used by both `manifest.createPullRequests()` and `manifest.createReleases()`. This means using `RELEASE_PLZ_TOKEN` PAT ensures tag pushes fire `release.yml`. (Source: [release-please-action/src/index.ts](https://github.com/googleapis/release-please-action/blob/main/src/index.ts), [issue #1000](https://github.com/googleapis/release-please-action/issues/1000).)
5. **Not release-plz** — the team explicitly migrated away from `release-plz/action` due to Class A failures (7+ fix attempts). Not going back.

## Workspace shape constraints

### Class A awareness

All 5 crates are `publish = false`. release-please does NOT run `cargo package` — it only bumps versions in `Cargo.toml` files. This means Class A (workspace dep resolution with mixed publish-status crates) is structurally avoided. If a future release-please version adds `cargo package` as a default step, that's a regression trigger for this workspace shape.

### `version.workspace = true` handling

This is a virtual workspace — the root `Cargo.toml` has `[workspace.package].version`, not `[package].version`. Member crates use `version.workspace = true`. release-please's Rust strategy (`src/updaters/rust/cargo-toml.ts`) targets `[package].version` by default, which may not correctly locate `[workspace.package].version`.

**Mitigation:** Use release-please's `extra-files` feature with a generic TOML updater to explicitly target the `workspace.package.version` field in root `Cargo.toml`. If `release-type: rust` handles this correctly out of the box (first Release PR will reveal), the `extra-files` fallback can be removed. The first Release PR is the verification gate — inspect the diff before merging to confirm only `[workspace.package].version` was bumped and member crates were left untouched.

### Cargo.lock staleness

release-please does NOT run `cargo update --workspace`. Its `CargoLock` updater (`src/updaters/rust/cargo-lock.ts`) performs direct TOML string replacement on version fields — no Cargo toolchain invocation. The existing workflow runs `cargo update --workspace` explicitly after bumping versions.

**Mitigation:** Add a post-release-please workflow step that checks out the release-please branch, runs `cargo update --workspace`, commits the updated `Cargo.lock`, and pushes. See Step 3 below for the workflow YAML.

## Implementation steps

### Step 1 — Add `release-please-config.json`

Create at repo root:

```json
{
  "$schema": "https://raw.githubusercontent.com/googleapis/release-please/main/schemas/config.json",
  "release-type": "rust",
  "include-component-in-tag": false,
  "include-v-in-tag": true,
  "bump-minor-pre-major": true,
  "bump-patch-for-minor-pre-major": false,
  "packages": {
    ".": {
      "component": "mika",
      "changelog-path": "CHANGELOG.md",
      "extra-files": [
        {
          "type": "toml",
          "path": "Cargo.toml",
          "jsonpath": "$.workspace.package.version"
        }
      ]
    }
  }
}
```

Key decisions:
- **Single root package** (`.`), not per-crate. All crates share the workspace version (`version.workspace = true`), so a single version bump at the root is correct. Per-crate packages would create 5 separate Release PRs, which is wrong for a unified workspace version.
- **`extra-files` with TOML jsonpath** — explicitly targets `workspace.package.version` in root `Cargo.toml`. This is defense-in-depth for the virtual workspace shape: if the Rust strategy's default `[package].version` updater doesn't find the field, the extra-files updater ensures `[workspace.package].version` is still bumped. If both fire (Rust strategy handles it natively), the result is idempotent (same version written twice). First Release PR will confirm which path activates.
- **`include-component-in-tag: false`** — produces `v0.12.3`, not `mika-v0.12.3`. Matches the existing tag format that `release.yml` triggers on (`v*`).
- **`bump-minor-pre-major: true`** — pre-1.0, `feat` commits bump minor (0.12→0.13), not major. Matches existing behavior.
- **`bump-patch-for-minor-pre-major: false`** — `feat` bumps minor even pre-1.0. `fix` bumps patch. This matches the existing bash version-bump logic.

### Step 2 — Add `.release-please-manifest.json`

Create at repo root:

```json
{
  ".": "0.12.2"
}
```

This seeds release-please with the current version. After the first run, release-please updates this file automatically in each Release PR.

### Step 3 — Rewrite `.github/workflows/release-pr.yml`

Replace the entire workflow file. The new workflow:

```yaml
name: Release

on:
  push:
    branches:
      - main
  workflow_dispatch:

permissions:
  contents: write
  pull-requests: write

jobs:
  release-please:
    name: Release Please
    runs-on: ubuntu-22.04
    timeout-minutes: 10
    if: github.repository_owner == 'senara-solutions'
    outputs:
      pr: ${{ steps.release.outputs.pr }}
      prs_created: ${{ steps.release.outputs.prs_created }}
    steps:
      - uses: googleapis/release-please-action@5c625bfb5d1ff62eadeeb3772007f7f66fdcf071  # v4.4.1
        id: release
        with:
          token: ${{ secrets.RELEASE_PLZ_TOKEN }}

  # release-please does NOT run `cargo update --workspace` — its CargoLock updater
  # does string replacement only. This job regenerates Cargo.lock on the release PR
  # branch after release-please creates/updates it.
  update-lockfile:
    name: Update Cargo.lock
    runs-on: ubuntu-22.04
    needs: release-please
    if: needs.release-please.outputs.prs_created == 'true' || needs.release-please.outputs.pr != ''
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd  # v6
        with:
          ref: release-please--branches--main
          token: ${{ secrets.RELEASE_PLZ_TOKEN }}

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@631a55b12751854ce901bb631d5902ceb48146f7  # stable

      - name: Regenerate Cargo.lock
        run: |
          cargo update --workspace
          if git diff --quiet Cargo.lock; then
            echo "Cargo.lock already up to date"
          else
            git config user.name "github-actions[bot]"
            git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
            git add Cargo.lock
            git commit -m "chore: regenerate Cargo.lock after version bump"
            git push
          fi
```

Key changes:
- **Entire `release-pr` job replaced** — no more `git-cliff`, no more manual version bumping, no more branch recreation, no more `gh pr create`.
- **Entire `release-tag` job removed** — release-please creates the tag + GitHub Release on merge of the Release PR. The PAT (`RELEASE_PLZ_TOKEN`) ensures the tag push triggers `release.yml`. Confirmed via source: `src/index.ts` creates a single `GitHub` client with the `token` input, used for both `manifest.createPullRequests()` and `manifest.createReleases()`.
- **`update-lockfile` job added** — runs `cargo update --workspace` on the release-please branch after the PR is created/updated. This compensates for release-please's string-only `Cargo.lock` updater.
- **`workflow_dispatch` kept** — allows manual re-run if needed.
- **`concurrency` block removed** — release-please is idempotent; concurrent runs are safe (later run wins).
- **Action pinned to commit SHA `5c625bfb5d1ff62eadeeb3772007f7f66fdcf071`** (v4.4.1) — per repo convention.

### Step 4 — Clean up orphan release branches (post-verification only)

**Timing: AFTER the first successful Release PR merge + tag creation + release.yml trigger verification.** Do NOT delete old branches before the migration is verified working — they serve as rollback anchors.

After verification, delete orphan branches from the old tool:

```bash
git push origin :release/v0.12.2
```

This is a one-time manual step, not automated in the workflow. Document in the PR description.

### Step 5 — Update `docs/deployment.md` § 3c

Update the "Release PR" section to reflect the new tool:

```markdown
### Release PR (`release-pr.yml`)

Runs on push to `main` (after CI passes):
- Uses `googleapis/release-please-action` to maintain a persistent Release PR with version bumps and changelog
- On merge of the Release PR: creates a git tag (`v{version}`) and GitHub Release
- No crates.io publishing — all crates are `publish = false`
- Requires `RELEASE_PLZ_TOKEN` (PAT with `contents: write` and `pull-requests: write`)

**Important:** Uses a PAT (`RELEASE_PLZ_TOKEN`) instead of `GITHUB_TOKEN` so that the tag push triggers the release binary workflow.
```

### Step 6 — Update `docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md`

Mark the entire document as fully historical. Add frontmatter `superseded_by: release-please` and a header note:

```markdown
> **Historical document.** This describes the release-plz setup (Stage 1, 2026-03-01 → 2026-04-03). The current release automation uses `googleapis/release-please-action` — see `release-please-config.json` at repo root. Retained for institutional memory per the chronic-drift compound doc.
```

### Step 7 — Update `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`

Add a "Stage 4" section documenting the migration:

```markdown
## Stage 4 — release-please (mika#1049, 2026-05-09)

**Tool migration:** git-cliff (handwritten bash) → `googleapis/release-please-action` v4.

**Why this addresses the failure class.** The "No commits between" error from 2026-05-09 run #32 is a Class C variant: the workflow recreates `release/v0.12.x` from `main` on every push, and on the release PR's own merge commit, the branch and `main` are identical. release-please eliminates Class C entirely by maintaining a single persistent Release PR branch with proper state reconciliation — it never recreates the branch from scratch.

**Class coverage:**
- **Class A (workspace deps):** release-please does NOT run `cargo package`. All crates are `publish = false`. Class A is structurally avoided.
- **Class B (comparison mode):** release-please uses its own commit-tracking manifest (`.release-please-manifest.json`), not crates.io or tags. Class B is structurally avoided.
- **Class C (branch state):** Eliminated. release-please manages branch lifecycle internally.
- **Class D (packaging/identity):** Only risk is action-version-specific quirks. Mitigated by SHA-pinning.
```

Also update frontmatter `resolved: true` (or `resolved: validated` if the team prefers a validation gate on the new tool too).

## Files changed

| File | Action |
|------|--------|
| `.github/workflows/release-pr.yml` | Rewrite (remove bash+git-cliff, add release-please-action) |
| `release-please-config.json` | Create |
| `.release-please-manifest.json` | Create |
| `docs/deployment.md` | Update § 3c |
| `docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md` | Mark historical |
| `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` | Add Stage 4 |

## Files NOT changed (per AC)

| File | Reason |
|------|--------|
| `.github/workflows/release.yml` | Working, untouched. Tag-push trigger contract preserved. |
| `.github/workflows/ci.yml` | Working, untouched. |

## Risks and mitigations

| Risk | Mitigation |
|------|-----------|
| release-please doesn't find `[workspace.package].version` (virtual workspace — no `[package]` in root) | `extra-files` TOML updater explicitly targets `$.workspace.package.version`. Defense-in-depth: first Release PR diff is inspected before merge to confirm the version bump landed in the right field. |
| PAT-created tags might not fire `release.yml` | Confirmed via source code: release-please uses the `token` input for ALL GitHub API operations including tag creation ([src/index.ts](https://github.com/googleapis/release-please-action/blob/main/src/index.ts)). Same PAT (`RELEASE_PLZ_TOKEN`) already used for the existing `release-tag` job. |
| release-please changelog format differs from git-cliff | Acceptable. Both generate from conventional commits. Existing `CHANGELOG.md` will be overwritten by release-please on the first Release PR. The diff will be visible in the Release PR for review. |
| `Cargo.lock` staleness after version bump | `update-lockfile` job runs `cargo update --workspace` on the release-please branch after each PR creation/update. Compensates for release-please's string-only CargoLock updater. |
| release-please corrupts member crate `Cargo.toml` files (writes literal version over `version.workspace = true`) | Single root package config (`"."`) — release-please should only touch the root `Cargo.toml`. First Release PR diff is the verification gate: inspect that member crate files are untouched. If corrupted, add `exclude-paths` config to restrict file scope. |

## First-merge verification checklist

Before merging the first Release PR created by release-please, verify:
- [ ] `Cargo.toml` diff shows only `[workspace.package].version` bumped (not `[package].version` or member crates)
- [ ] Member crate `Cargo.toml` files are untouched (no `version.workspace = true` → literal version corruption)
- [ ] `Cargo.lock` is regenerated (by the `update-lockfile` job)
- [ ] `CHANGELOG.md` is present and reflects conventional commits since last tag
- [ ] `.release-please-manifest.json` is updated with the new version

After merging the first Release PR:
- [ ] A `v*` tag was created (check `git tag -l`)
- [ ] The tag was created with the PAT identity (check tagger — should be PAT user, not `github-actions[bot]`)
- [ ] `release.yml` was triggered by the tag push (check Actions tab)
- [ ] Binary builds completed successfully

## Validation gate

Per the chronic-drift compound doc's established pattern (preserved for continuity): **10 consecutive clean merges to main OR 14 days, whichever comes first.** On each merge:
1. `release-pr.yml` should succeed (release-please updates the Release PR)
2. No errors in the workflow run
3. The `update-lockfile` job succeeds when a PR exists

Additionally, on each Release PR merge cycle:
4. Merging the Release PR should produce a `v*` tag
5. The `v*` tag should fire `release.yml` (binary build)

The existing gate standard is preserved rather than relaxed because release-please is new to this workspace and the chronic-drift compound doc's gate has proven valuable for catching late-emerging Class D issues (e.g., mika#1006 unmasked by mika#1003).

## Out of scope

- `release.yml` (binary build) — working, do not modify
- `ci.yml` — working, do not modify
- Reverting to upstream `release-plz/action` — team explicitly migrated away
- Changing the workspace's per-crate `publish` strategy
- Per-crate independent versioning — all crates share workspace version
