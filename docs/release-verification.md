---
title: Release Artifact Verification
description: How to verify SHA-256 checksums, cosign signatures, build provenance, and SBOMs for Mika releases
---

# Release Artifact Verification

Every Mika release on GitHub includes checksums, cosign signatures, an SBOM, and build provenance attestations. This document explains how to verify each.

## Prerequisites

- [cosign](https://docs.sigstore.dev/cosign/system_config/installation/) (v2+)
- [GitHub CLI](https://cli.github.com/) (`gh`) for attestation verification
- `sha256sum` (Linux) or `shasum` (macOS)

## 1. Verify SHA-256 checksums

Each archive has a companion `.sha256` file.

```bash
# Linux
sha256sum --check mika-v0.2.0-x86_64-unknown-linux-gnu.tar.gz.sha256

# macOS
shasum -a 256 --check mika-v0.2.0-x86_64-apple-darwin.tar.gz.sha256
```

A successful check prints `OK` for each verified file.

## 2. Verify cosign signatures

Each archive and checksum file is signed with keyless cosign (Sigstore OIDC). The signature proves the artifact was built by the `release.yml` workflow in the `senara-solutions/mika` repository.

```bash
cosign verify-blob \
  --bundle mika-v0.2.0-x86_64-unknown-linux-gnu.tar.gz.bundle \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/senara-solutions/mika/\.github/workflows/release\.yml@' \
  mika-v0.2.0-x86_64-unknown-linux-gnu.tar.gz
```

Replace the filename with the archive you downloaded. The `--certificate-identity-regexp` restricts verification to artifacts built by this specific workflow.

## 3. Verify build provenance attestation

GitHub Artifact Attestations provide SLSA provenance metadata linking each artifact to its source commit and workflow run.

```bash
gh attestation verify mika-v0.2.0-x86_64-unknown-linux-gnu.tar.gz \
  --repo senara-solutions/mika
```

This checks the artifact against GitHub's attestation store and prints the source commit, workflow, and runner details.

## 4. Inspect the SBOM

Each release includes a CycloneDX SBOM listing all Rust dependencies (`mika-<tag>-sbom.cdx.json`).

```bash
# Pretty-print the SBOM
cat mika-v0.2.0-sbom.cdx.json | python3 -m json.tool

# List dependency names
cat mika-v0.2.0-sbom.cdx.json | python3 -c "
import json, sys
bom = json.load(sys.stdin)
for c in bom.get('components', []):
    print(c.get('name', ''))
"
```

CycloneDX SBOMs can also be imported into vulnerability scanners like [Grype](https://github.com/anchore/grype) or [Trivy](https://github.com/aquasecurity/trivy):

```bash
grype sbom:mika-v0.2.0-sbom.cdx.json
```

## References

- [Sigstore documentation](https://docs.sigstore.dev/)
- [cosign verify-blob reference](https://docs.sigstore.dev/cosign/verifying/verify/)
- [GitHub Artifact Attestations](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations)
- [CycloneDX specification](https://cyclonedx.org/specification/overview/)
