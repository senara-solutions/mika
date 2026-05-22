---
title: Dockerfile.agent gh CLI checksum verification fails due to wget filename mismatch
date: 2026-05-22
category: build-errors
module: Dockerfile.agent
problem_type: build_error
component: tooling
symptoms:
  - "sha256sum: gh_2.65.0_linux_amd64.tar.gz: No such file or directory"
  - "sha256sum: WARNING: 1 listed file could not be read"
  - "docker build -f Dockerfile.agent fails at gh CLI install step with exit code 1"
root_cause: config_error
resolution_type: config_change
severity: high
tags:
  - dockerfile
  - docker-build
  - checksum
  - sha256sum
  - wget
  - gh-cli
  - filename-mismatch
  - cloud-deploy
---

# Dockerfile.agent gh CLI checksum verification fails due to wget filename mismatch

## Problem

`docker build -f Dockerfile.agent` fails during the gh CLI installation step. The `sha256sum -c` checksum verification cannot find the downloaded tarball because `wget -qO` renamed it to a generic filename that doesn't match the filename referenced in the checksums file.

This was the second Dockerfile.agent build blocker discovered during the cloud-deploy readiness audit (after mika#1237, which removed broken COPY directives).

## Symptoms

- `sha256sum: gh_2.65.0_linux_amd64.tar.gz: No such file or directory`
- `sha256sum: WARNING: 1 listed file could not be read`
- Docker build exits with code 1 at the gh CLI install RUN instruction

## What Didn't Work

This bug was latent — the Dockerfile had never been built end-to-end from a clean checkout. There was no CI `docker build` smoke test to catch it. The issue was discovered only when attempting the first cloud deploy.

## Solution

Download the tarball using its original filename instead of renaming it, so `sha256sum -c` can resolve the filename from the checksums file.

**Before:**
```dockerfile
wget -qO /tmp/gh.tar.gz "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_${ARCH}.tar.gz" && \
...
tar -xzf /tmp/gh.tar.gz -C /tmp && \
```

**After:**
```dockerfile
wget -qO "/tmp/gh_${GH_VERSION}_linux_${ARCH}.tar.gz" "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_${ARCH}.tar.gz" && \
...
tar -xzf "/tmp/gh_${GH_VERSION}_linux_${ARCH}.tar.gz" -C /tmp && \
```

Two-line change. The cleanup glob `rm -rf /tmp/gh*` already covers the new filename pattern.

## Why This Works

`sha256sum -c` reads checksum lines in the format `<hash>  <filename>` and verifies by opening `<filename>` relative to the current directory. The gh CLI checksums file contains entries like `abc123  gh_2.65.0_linux_amd64.tar.gz`. When `wget -qO` renames the file to `gh.tar.gz`, `sha256sum` looks for `gh_2.65.0_linux_amd64.tar.gz` and fails with "No such file or directory."

By downloading to the original filename, the file on disk matches the filename in the checksum line, and verification succeeds.

## Prevention

- **Add a CI `docker build` smoke test** to `.github/workflows/ci.yml` — both this issue and mika#1237 would have been caught immediately. Currently tracked as a separate follow-up ticket.
- **When using `sha256sum -c` with upstream checksums files, always preserve the original filename.** The checksums file references the upstream release filename, so the local copy must match. Alternative: construct the checksum line manually (as the Google Workspace CLI install in the same Dockerfile does with `echo "$(cat gws.tar.gz.sha256)  gws.tar.gz" | sha256sum -c -`).
- **Contrast the two patterns in Dockerfile.agent:**
  - gh CLI: uses upstream checksums file with `grep | sha256sum -c` — filename must match
  - Google Workspace CLI: downloads checksum separately, constructs the line manually — filename can be anything

## Related Issues

- [mika#1240](https://github.com/senara-solutions/mika/issues/1240) — this fix
- [mika#1237](https://github.com/senara-solutions/mika/issues/1237) — prior Dockerfile.agent fix (broken COPY directives)
- [docs/solutions/build-errors/dockerfile-agent-broken-copy-nonexistent-paths-2026-05-22.md](dockerfile-agent-broken-copy-nonexistent-paths-2026-05-22.md) — companion doc from the same cloud-deploy audit
