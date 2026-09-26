# Plan: fix(os) — gws install sha256sum filename mismatch

- **Issue:** mika#1249
- **Type:** fix
- **Priority:** p0-critical
- **Branch:** `fix/1249/os-gws-install-sha256sum-filename`

## Problem

`docker build --target mika-runtime -f os/Dockerfile .` fails at the gws (Google Workspace CLI) install step. The `sha256sum -c -` invocation receives a malformed three-token line because the `.sha256` file from GitHub releases already contains the archive's full name (`<hash>  gws-x86_64-unknown-linux-gnu.tar.gz`), and the Dockerfile wraps it with `echo "$(cat .sha256)  gws.tar.gz"`, producing `<hash>  gws-x86_64-unknown-linux-gnu.tar.gz  gws.tar.gz` — sha256sum interprets everything after the hash as a single filename with an embedded space.

This is the 5th instance of the same bug class (download-rename + sha256sum filename mismatch) in 4 days, after mika#1240, mika#1242, mika#1243.

## Fix

Two changes to the gws install block (lines 107-120 of `os/Dockerfile`):

### Change 1: Download under canonical archive name

Replace `wget -qO /tmp/gws.tar.gz` with `wget -qO "/tmp/gws-${GWS_ARCH}.tar.gz"` so the downloaded file's name matches what the `.sha256` file references. Same for the `.sha256` file itself.

### Change 2: Use grep-based checksum verification

Replace:
```sh
echo "$(cat gws.tar.gz.sha256)  gws.tar.gz" | sha256sum -c -
```

With:
```sh
grep "gws-${GWS_ARCH}.tar.gz" "gws-${GWS_ARCH}.tar.gz.sha256" | sha256sum -c -
```

This matches the canonical pattern already working at line 100 (gh CLI install).

### sha256sum audit (AC4)

All three `sha256sum` invocations in `os/Dockerfile` after this fix:

| Line | Binary | Pattern | Status |
|------|--------|---------|--------|
| 100 | gh CLI | `grep "<name>" checksums.txt \| sha256sum -c -` | ✅ Correct (fixed in mika#1243) |
| 117 | gws | `grep "<name>" "<name>.sha256" \| sha256sum -c -` | ✅ Fixed by this PR |
| 136 | ollama | `awk '{print $1}' + echo "$HASH  <name>" \| sha256sum -c -` | ✅ Correct (hash-only extract, filename matches downloaded name) |

**Note:** `Dockerfile.agent` line 58 still uses the broken `echo "$(cat .sha256)  gws.tar.gz"` pattern — same bug class but out of scope for this issue (separate file, separate image). Should be tracked as a follow-up.

## Acceptance Criteria Mapping

- **AC1.** `docker build --target mika-os -f os/Dockerfile .` succeeds — verified by the fix ensuring sha256sum passes.
- **AC2.** `docker build --target mika-runtime -f os/Dockerfile .` succeeds — mika-runtime inherits from mika-os, so fixing the mika-os stage fixes both.
- **AC3.** Uses the same `grep <name> <checksums> | sha256sum -c -` recipe as the gh install on line 100.
- **AC4.** Full audit of all sha256sum invocations in os/Dockerfile completed — see table above. All three are correct after this fix.

## Test Plan

- [ ] `docker build --target mika-os -f os/Dockerfile .` completes successfully
- [ ] `docker build --target mika-runtime -f os/Dockerfile .` completes successfully
- [ ] Verify gws binary is functional: `docker run --rm mika-os:dev gws --version`

## Risk

Minimal — single-file, 4-line change following an established pattern. No behavioral change beyond fixing the broken checksum verification.
