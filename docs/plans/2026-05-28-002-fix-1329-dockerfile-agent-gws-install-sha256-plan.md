# Plan: fix(dockerfile-agent): gws install sha256 verification fails — checksum asset already contains filename

**Ticket:** mika issue#1329
**Type:** Bug fix
**Scope:** `Dockerfile.agent` line 58

## Problem

The `gws` (Google Workspace CLI) install step in `Dockerfile.agent` fails during checksum verification. The upstream `.sha256` asset contains the format `<hash> *<original-filename>` (e.g., `81e35ebb...2160 *gws-x86_64-unknown-linux-gnu.tar.gz`), but line 58 appends a second filename:

```sh
echo "$(cat gws.tar.gz.sha256)  gws.tar.gz" | sha256sum -c -
# Expands to:
echo "81e35ebb...2160 *gws-x86_64-unknown-linux-gnu.tar.gz  gws.tar.gz" | sha256sum -c -
```

`sha256sum -c` parses the filename as `gws-x86_64-unknown-linux-gnu.tar.gz  gws.tar.gz` (concatenated), which doesn't exist → `FAILED` → exit 1.

This blocks the mika-cloud deploy: the agent image cannot be built from a clean `main`.

## Fix

**Single-line change** in `Dockerfile.agent` line 58. Extract only the bare hash from the checksum file before appending the local filename:

```diff
-    cd /tmp && echo "$(cat gws.tar.gz.sha256)  gws.tar.gz" | sha256sum -c - && \
+    cd /tmp && echo "$(cut -d' ' -f1 gws.tar.gz.sha256)  gws.tar.gz" | sha256sum -c - && \
```

`cut -d' ' -f1` takes the first space-delimited field (the bare hash), discarding the `*<original-filename>` suffix. This matches the pattern used by `gh` install on line 46 (which works because `gh_checksums.txt` uses `<hash>  <filename>` format matching the local filename).

## Files Changed

| File | Change |
|------|--------|
| `Dockerfile.agent` | Line 58: `cat` → `cut -d' ' -f1` to extract bare hash |

## Verification

1. `docker build -f Dockerfile.agent --platform linux/amd64 -t mika-agent-test .` completes successfully
2. `docker run --rm mika-agent-test gws --version` confirms the binary is present and executable

## Risk Assessment

**Minimal.** Single-line change in a `RUN` step. The fix is idempotent — if upstream ever changes the `.sha256` format to bare hash, `cut -d' ' -f1` still returns the correct value. No runtime behavior change; only affects image build.
