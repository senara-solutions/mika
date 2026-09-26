# Plan: Release artifact hardening — checksums, signatures, archive naming (#622)

## Problem

The release workflow (`release.yml`) produces cross-platform binaries via `taiki-e/upload-rust-binary-action` but needs hardening before the Distribution Channels milestone (#11) can build packaging sub-issues (#623 Debian, #624 Gentoo, #625 Homebrew) on top of it. Current state:

- **Checksums:** Already present (`checksum: sha256` on both binary upload steps). ✅
- **Signatures:** Not present. No cosign or GPG signing step.
- **Archive naming:** Uses `mika-$tag-$target` / `mika-spirit-$tag-$target` — functional but the `$tag` includes the `v` prefix (e.g., `mika-v0.1.5-x86_64-unknown-linux-gnu`). This is fine for GitHub Releases but packaging scripts need a predictable, version-only naming convention.
- **SBOM:** Not present.
- **Verification script:** No `install.sh` or verification doc for self-hosters.

## Scope

This is a CI-only change. No Rust code changes. All work is in `.github/workflows/release.yml` plus a new verification documentation file.

## Changes

### 1. Add cosign signing to release artifacts

Add a signing step after the build job that signs all release artifacts with keyless cosign (Sigstore). `taiki-e/upload-rust-binary-action` does not natively support cosign, so this is a post-upload step that downloads the artifacts, signs them, and re-uploads the `.sig` and `.bundle` files.

**File:** `.github/workflows/release.yml`

- Add a new `sign` job that runs after `build` completes (using `needs: build`)
- Use `sigstore/cosign-installer` action to install cosign
- Download the release assets via `gh release download`
- Sign each `.tar.gz` / `.zip` and each `.sha256` file with `cosign sign-blob --yes --bundle <file>.bundle <file>`
- Upload the `.bundle` signature files back to the release via `gh release upload`
- This uses keyless signing (Sigstore OIDC via GitHub Actions identity) — no secrets management needed

**Why keyless cosign over GPG:** GPG requires key management, rotation, and secret storage. Cosign keyless uses the GitHub Actions OIDC token as identity — the signature proves the artifact was built by this specific workflow in this specific repo. No key to leak, rotate, or distribute. This is the modern standard (used by Kubernetes, Homebrew, etc.).

### 2. Standardize archive naming

The current naming `mika-$tag-$target` produces archives like `mika-v0.1.5-x86_64-unknown-linux-gnu.tar.gz`. This is already reasonable and matches conventions used by ripgrep, bat, and other Rust CLI tools.

**Decision: keep current naming.** The `$tag` variable is substituted by the action and includes the `v` prefix, which is standard. Packaging scripts in #623/#624/#625 will strip the `v` prefix as needed — this is a one-liner in each packaging recipe, not a release-workflow concern.

No changes needed here.

### 3. Add SBOM generation

Add `cargo-sbom` or `cargo-cyclonedx` to generate a CycloneDX SBOM from `Cargo.lock` and attach it to the release. This runs once (not per-target) since dependencies are the same across platforms.

**File:** `.github/workflows/release.yml`

- Add a new `sbom` job that runs in parallel with `build` (no dependency)
- Install `cargo-cyclonedx` via `cargo install cargo-cyclonedx`
- Run `cargo cyclonedx --format json` to produce `bom.json`
- Rename to `mika-$tag-sbom.cdx.json` for clarity
- Upload to the release via `gh release upload`
- The SBOM is also signed by the `sign` job (which runs after both `build` and `sbom`)

**Why CycloneDX over SPDX:** CycloneDX is the more common format for Rust/cargo ecosystems. `cargo-cyclonedx` is maintained and reads directly from `Cargo.lock`.

### 4. Add release artifact verification documentation

Create a short doc explaining how self-hosters verify downloaded artifacts.

**File:** `docs/release-verification.md`

Contents:
- How to verify SHA-256 checksums (`sha256sum --check`)
- How to verify cosign signatures (`cosign verify-blob --bundle`)
- How to inspect the SBOM
- Links to Sigstore documentation

### 5. Add attestation via GitHub Artifact Attestations (optional, low-cost)

GitHub Actions supports native artifact attestations via `actions/attest-build-provenance`. This produces SLSA provenance metadata linking each artifact to its source commit and workflow run.

**File:** `.github/workflows/release.yml`

- Add `attestations: write` to the workflow permissions
- Add `id-token: write` to the workflow permissions (required for OIDC)
- After each binary upload step, call `actions/attest-build-provenance` with the archive path
- This is complementary to cosign — cosign proves "who built it," attestation proves "how it was built"

## Implementation order

1. Step 1 (cosign signing) — the core hardening deliverable
2. Step 3 (SBOM) — parallel, independent
3. Step 5 (attestation) — optional, adds provenance metadata
4. Step 4 (verification docs) — last, references all the above

Step 2 is a no-op (current naming is fine).

## Files changed

| File | Change |
|------|--------|
| `.github/workflows/release.yml` | Add `sign` job, `sbom` job, attestation steps, updated permissions |
| `docs/release-verification.md` | New — verification instructions for self-hosters |

## Risks and mitigations

- **Cosign keyless requires `id-token: write` permission** — this is standard for GitHub Actions OIDC. The permission is scoped to this workflow only.
- **`cargo-cyclonedx` install adds ~2 min to CI** — runs in parallel with build, so no wall-clock impact.
- **Sigstore transparency log is public** — the signing events (repo name, commit SHA) are recorded in Rekor. This is by design and expected for open-source releases.
- **First release after merge needs manual verification** — test the signing and SBOM generation on the next tag push. No way to dry-run tag-triggered workflows without pushing a tag.

## Verification

- After merge, trigger a release (or push a test tag) and verify:
  - Each archive has a companion `.sha256` file (already present)
  - Each archive has a companion `.bundle` cosign signature
  - Release contains `mika-<tag>-sbom.cdx.json`
  - `cosign verify-blob` succeeds against each artifact
  - Attestation is visible via `gh attestation verify`
