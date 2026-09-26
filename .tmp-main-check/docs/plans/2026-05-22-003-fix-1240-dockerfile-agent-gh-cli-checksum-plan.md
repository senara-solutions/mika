# Plan: Fix Dockerfile.agent gh CLI install checksum verification (mika#1240)

## Metadata

- **Issue:** mika#1240
- **Type:** fix
- **Branch:** `fix/1240/dockerfile-agent-gh-cli-install-checksum`
- **Priority:** p1-important (second blocker on agent image build)
- **Date:** 2026-05-22

## Problem

`Dockerfile.agent` lines 38-46 install the GitHub CLI with checksum verification, but the download
filename doesn't match what the checksum file references:

1. `wget -qO /tmp/gh.tar.gz` saves the tarball as `/tmp/gh.tar.gz` (renamed)
2. The checksums file contains lines like `<hash>  gh_2.65.0_linux_amd64.tar.gz`
3. `sha256sum -c -` parses the checksum line and looks for `gh_2.65.0_linux_amd64.tar.gz` on disk
4. File not found -> build fails

This is the second Dockerfile.agent build blocker (after mika#1237 which removed broken COPY lines).

## Root cause

The `wget -qO` flag renames the downloaded file, but `sha256sum -c` resolves filenames from the
checksum file content. The filename on disk must match the filename in the checksum line.

## Fix

Two-line change in `Dockerfile.agent` lines 41 and 44:

### Step 1: Download to original filename

**File:** `Dockerfile.agent` line 41

```diff
-    wget -qO /tmp/gh.tar.gz "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_${ARCH}.tar.gz" && \
+    wget -qO "/tmp/gh_${GH_VERSION}_linux_${ARCH}.tar.gz" "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_${ARCH}.tar.gz" && \
```

### Step 2: Extract from original filename

**File:** `Dockerfile.agent` line 44

```diff
-    tar -xzf /tmp/gh.tar.gz -C /tmp && \
+    tar -xzf "/tmp/gh_${GH_VERSION}_linux_${ARCH}.tar.gz" -C /tmp && \
```

### Verification

The cleanup line 46 (`rm -rf /tmp/gh*`) already covers the new filename pattern — no change needed.

Line 45 (`mv /tmp/gh_${GH_VERSION}_linux_${ARCH}/bin/gh /usr/local/bin/gh`) references the
extracted directory name (from the tarball's internal structure), not the tarball filename — no
change needed.

## AC tie-back

- **AC1 (implicit):** `sha256sum -c -` succeeds during `docker build -f Dockerfile.agent` because
  the file on disk matches the filename in the checksum line.

## Out of scope

- CI `docker build` smoke test (mentioned as follow-up in the issue — separate ticket)
- Google Workspace CLI install (lines 48-58 — uses a different pattern that already works: downloads
  as `gws.tar.gz` and constructs the checksum line manually with `echo "$(cat gws.tar.gz.sha256)  gws.tar.gz"`)

## Risk

Minimal. Two-line change, both within the same `RUN` instruction. The fix aligns the filename on
disk with the filename the checksum file expects — identical to how the gh CLI install was likely
intended to work originally.
