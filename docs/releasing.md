---
title: Releasing
description: How the operator cuts a Mika release — dry run on a release candidate, launch tag, verification checklist, rerun and rollback
---

# Releasing

A release is one operator gesture: pushing a tag. Everything else — building, signing, attesting and publishing — is `.github/workflows/release.yml`. Nobody but the operator pushes a `v*` tag, and it is pushed with the operator's own credentials: GitHub does not start workflows for a tag created with a workflow's `GITHUB_TOKEN`.

| Tag | What the workflow publishes |
|---|---|
| `vX.Y.Z-rc.N` | a **prerelease**, which never becomes `latest` |
| `vX.Y.Z` | a release, marked `latest` |

**Why the distinction matters.** `install.sh` installs `releases/latest`, and GitHub keeps a release out of `latest` only when it is a prerelease. A candidate published as a normal release would be what the family installs. `scripts/release-tag.sh` classifies the tag, and `scripts/test-release-tag.sh` holds that classification in CI. The `publish` job checks the prerelease flag on the draft, makes the release public, then re-reads `releases/latest`. If a candidate shows up there, the job puts the release back to draft and fails.

## Before the tag

The workflow refuses a tag, before building anything, when:

- it is neither `vX.Y.Z` nor `vX.Y.Z-rc.N`;
- `X.Y.Z` differs from `[workspace.package] version` in `Cargo.toml`;
- the tagged commit is not on `main`;
- `docs/releases/vX.Y.Z.md`, which becomes the release body, is missing.

So the release PR moves the workspace version, `Cargo.lock`, `CHANGELOG.md`, and adds `docs/releases/vX.Y.Z.md`. Once it is merged, you tag the merge commit.

## 1. Dry run on a release candidate

The candidate runs exactly the pipeline the final tag will run, including `publish`, and it is the only way to run `install.sh` against real artifacts before launch day. It never becomes `latest`, so the family keeps installing whatever is `latest` today.

```sh
git fetch origin main
git show origin/main:Cargo.toml | grep -m1 '^version'   # must print version = "1.0.0"
git tag -a v1.0.0-rc.1 origin/main -m "Mika 1.0.0, release candidate 1"
git push origin v1.0.0-rc.1
gh run list -R senara-solutions/mika --workflow release.yml --limit 1
gh run watch -R senara-solutions/mika <run-id>
```

### Checklist

Each step must pass. Paste the run URL and the output of steps 2 to 6 into the release ticket.

1. **The run is green**: `prepare`, the four `build` jobs, `sbom`, `sign` and `publish`. The `sign` job has already run `cosign verify-blob` on every file, with the identity from `docs/release-verification.md`.
2. **It is a prerelease, and `latest` has not moved.**
   ```sh
   gh release view v1.0.0-rc.1 -R senara-solutions/mika --json isPrerelease,isDraft -q .
   gh api repos/senara-solutions/mika/releases/latest -q .tag_name   # still the previous release
   ```
3. **All 36 files are there.** For `mika` and `mika-spirit` on each of the four targets: `.tar.gz`, `.tar.gz.sha256`, `.tar.gz.bundle`, `.tar.gz.sha256.bundle`. Plus `mika-v1.0.0-rc.1-sbom.cdx.json`, `mika-spirit-v1.0.0-rc.1-sbom.cdx.json` and their `.bundle`.
   ```sh
   mkdir rc1 && cd rc1
   gh release download v1.0.0-rc.1 -R senara-solutions/mika
   ls | wc -l
   ```
4. **The user procedure of `docs/release-verification.md` passes, step by step**, from inside `rc1/`:
   ```sh
   # 1. checksums — sha256sum on Linux, `shasum -a 256 --check` on macOS
   sha256sum --check *.tar.gz.sha256
   # 2. cosign signatures
   for f in *.tar.gz *.sha256 *.cdx.json; do
     cosign verify-blob --bundle "$f.bundle" \
       --certificate-oidc-issuer https://token.actions.githubusercontent.com \
       --certificate-identity-regexp '^https://github\.com/senara-solutions/mika/\.github/workflows/release\.yml@' \
       "$f"
   done
   # 3. build provenance
   for f in *.tar.gz; do gh attestation verify "$f" --repo senara-solutions/mika; done
   # 4. SBOM
   python3 -c "import json,sys; b=json.load(open(sys.argv[1])); print(len(b.get('components', [])), 'components')" mika-v1.0.0-rc.1-sbom.cdx.json
   ```
5. **`install.sh` installs it and the binary runs**, on Linux:
   ```sh
   dir="$(mktemp -d)"
   curl -fsSL https://raw.githubusercontent.com/senara-solutions/mika/main/install.sh | MIKA_INSTALL_DIR="$dir" sh -s -- v1.0.0-rc.1
   "$dir/mika" --version    # mika 1.0.0 (<commit>)
   ```
   The verification procedure above never extracts an archive. This step is what catches a name or a layout that drifted from `install.sh`, which is exactly what broke v0.12.x.
6. **Same step 5 on a Mac.** On Apple Silicon this runs the aarch64 build. The x86_64 build is cross-compiled and never runs in CI, so only an Intel Mac proves it.

A candidate can stay published: it is a prerelease, and it records what was tested. To remove it: `gh release delete v1.0.0-rc.1 -R senara-solutions/mika --cleanup-tag --yes`. If the dry run fails, fix forward in a PR and tag `v1.0.0-rc.2`.

### Without publishing anything

A manual run, from `main` only, builds and signs the same files as a workflow artifact and creates no release. It cannot exercise `publish` or `install.sh`, so it is a debugging aid, not a substitute for the candidate.

```sh
gh workflow run release.yml -R senara-solutions/mika --ref main -f tag=v1.0.0-rc.1
gh run download <run-id> -R senara-solutions/mika -n release-v1.0.0-rc.1 -D rc1-manual
```

Checklist steps 1, 3 and 4 apply to `rc1-manual/`.

## 2. Launch

```sh
git fetch origin main
git tag -a v1.0.0 <tested commit> -m "Mika 1.0.0"
git push origin v1.0.0
```

Tag the commit the candidate tested, unless main has moved on purpose since. Then:

```sh
gh api repos/senara-solutions/mika/releases/latest -q .tag_name   # v1.0.0
dir="$(mktemp -d)"
curl -fsSL https://raw.githubusercontent.com/senara-solutions/mika/main/install.sh | MIKA_INSTALL_DIR="$dir" sh
"$dir/mika" --version    # mika 1.0.0 (<commit>)
```

## Rerun, rollback, fallbacks

- **A run that failed before "Make it public" can be rerun as is** (`gh run rerun <run-id> --failed`). `publish` deletes the draft a previous attempt left behind.
- **A run that failed at or after "Make it public" cannot be rerun.** The release is public, and `publish` refuses to touch a published release. Read what the last step reported. If a candidate became `latest`, the job already put it back to draft. If a final release is published but `latest` did not point at it (the API may lag for a moment), check `gh api repos/senara-solutions/mika/releases/latest -q .tag_name` by hand, and fix it with `gh release edit <tag> --latest` if needed.
- **Rollback.** `gh release edit v1.0.0 --draft=true` takes the release out of `latest` at once. `install.sh` then falls back to the previous `latest`. For v1.0.0 that is v0.12.2, which `install.sh` cannot install (its checksum name predates this workflow), so the real remedy is a fixed `v1.0.1`.
- **x86_64 macOS fails to build.** That target is cross-compiled on `macos-14` because `macos-13` never got a runner (runs of v0.12.0 to v0.12.2: "awaiting a runner for 24h0m0s"). If cross-compiling breaks, move that one matrix entry to a native Intel runner label that GitHub still serves; check its runner documentation at that moment.