# Plan: chore(ci): release artifact hardening — checksums, signatures, archive naming

**Ticket:** mika issue#622
**Milestone:** Distribution channels (#11)
**Branch:** `chore/622/release-artifact-hardening-checksums`

## Context

The `release.yml` workflow already uses `taiki-e/upload-rust-binary-action` with `checksum: sha256` for both `mika` and `mika-server` binaries. Archive naming follows the pattern `mika-$tag-$target` / `mika-server-$tag-$target`, which is already reasonable.

This ticket hardens the release pipeline as a foundation for the packaging sub-issues (#623 Debian, #624 Gentoo, #625 Homebrew). Those packaging workflows will consume release artifacts and need reliable checksums, attestations, and consistent naming.

## Current State Audit

| Area | Status | Notes |
|------|--------|-------|
| SHA-256 checksums | ✅ Present | `checksum: sha256` on both upload steps |
| Archive naming | ✅ Consistent | `mika-$tag-$target` / `mika-server-$tag-$target` |
| Build provenance | ❌ Missing | No `actions/attest-build-provenance` |
| SBOM | ❌ Missing | No SBOM generation or attestation |
| Cosign/sigstore signing | ❌ Missing | No keyless signing of release artifacts |
| Verification docs | ❌ Missing | No instructions for users to verify downloads |

## Changes

### 1. Add GitHub Artifact Attestations (build provenance)

**File:** `.github/workflows/release.yml`

Add `actions/attest-build-provenance` after each `upload-rust-binary-action` step. This uses GitHub's native Sigstore integration — keyless, no secrets needed, just the `id-token: write` and `attestations: write` permissions.

Each binary archive (`.tar.gz` on Linux/macOS) gets a provenance attestation tied to the workflow run. Users verify with `gh attestation verify`.

Implementation:
- Add `id-token: write` and `attestations: write` to the job's permissions
- After each upload step, glob for the built archive in the working directory and run `actions/attest-build-provenance` on it
- Pin the action to a commit SHA per CI convention

### 2. Add cosign keyless signing of checksums

**File:** `.github/workflows/release.yml`

Add a post-build step that signs the SHA-256 checksum files (`.sha256`) using Sigstore cosign keyless signing. This provides a second, independent verification layer beyond GitHub attestations.

Implementation:
- Install cosign via `sigstore/cosign-installer` (pinned SHA)
- After both binaries are uploaded, sign each `.sha256` file with `cosign sign-blob --yes --bundle <file>.cosign-bundle <file>`
- Upload the `.cosign-bundle` files to the GitHub release via `gh release upload`
- The `--yes` flag enables keyless mode using the GitHub Actions OIDC token

### 3. Generate SBOM via `cargo-sbom`

**File:** `.github/workflows/release.yml`

Generate a CycloneDX SBOM for the workspace and attach it to the release. This is "cheap" per the ticket — a single `cargo sbom` invocation produces a dependency manifest without touching the build.

Implementation:
- Install `cargo-sbom` via `taiki-e/install-action` (already used for other tools)
- Run `cargo sbom --output-format cyclonedx_json > mika-$tag-sbom.cdx.json` once per release (not per-target — dependencies are target-independent at the Cargo.toml level)
- Upload the SBOM to the release
- Attest the SBOM with `actions/attest-sbom` (pinned SHA)

To avoid running SBOM generation 4× (once per matrix target), add a separate `sbom` job that runs on `ubuntu-22.04` after the build matrix, using `needs: build`.

### 4. Add verification documentation

**File:** `docs/verification.md`

Add a concise doc explaining how users verify downloaded binaries:

```
## Verify checksums
sha256sum -c mika-v0.2.0-x86_64-unknown-linux-gnu.sha256

## Verify cosign signature
cosign verify-blob --bundle mika-v0.2.0-x86_64-unknown-linux-gnu.tar.gz.sha256.cosign-bundle \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp 'github.com/senara-solutions/mika' \
  mika-v0.2.0-x86_64-unknown-linux-gnu.tar.gz.sha256

## Verify GitHub attestation
gh attestation verify mika-v0.2.0-x86_64-unknown-linux-gnu.tar.gz \
  --repo senara-solutions/mika
```

### 5. Standardize archive naming (no-op confirmation)

The current naming `mika-$tag-$target` already follows the `<binary>-<version>-<triple>` convention that packaging tools (Homebrew, cargo-binstall) expect. No change needed — document this as an explicit decision so future contributors don't "fix" it.

Add a comment in `release.yml` noting the naming convention is intentional and consumed by downstream packaging workflows.

## Non-goals

- **GPG signing:** Sigstore keyless is the modern standard; GPG key management adds operational burden with no benefit for this project's threat model.
- **Per-target SBOM:** Cargo dependencies are declared at the workspace level, not per-target. One SBOM covers all targets.
- **macOS codesign / notarization:** That's Homebrew territory (#625), not release artifact hardening.
- **Windows targets:** Not in the current build matrix; out of scope.

## Sequence

1. Permissions + attestation steps (build provenance) — smallest change, validates the attestation plumbing
2. Cosign signing of checksum files
3. SBOM generation + attestation (separate job)
4. Verification docs
5. Archive naming comment (trivial)

## Testing

- Push a test tag on a fork or use `workflow_dispatch` to validate the workflow changes
- Verify that `gh attestation verify` succeeds on the produced artifacts
- Verify that `cosign verify-blob` succeeds on the signed checksums
- Confirm SBOM is valid CycloneDX JSON via `cyclonedx-cli validate`

## Risk

- **Low:** All changes are additive to the release workflow. Existing checksum and archive behavior is unchanged.
- **cosign-installer version:** Pin to SHA, not tag, per CI convention. Watch for breaking changes in cosign v3 bundle format.
- **cargo-sbom availability:** If `cargo-sbom` is not installable via `install-action`, fall back to `cargo install cargo-sbom` with caching.
