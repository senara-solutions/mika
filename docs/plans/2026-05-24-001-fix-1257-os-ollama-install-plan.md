# Plan: Fix ollama install in mika/os/Dockerfile (#1257)

---
type: fix
module: os/Dockerfile
issue: 1257
severity: p0-critical
---

## Problem

`docker build --target mika-os -f mika/os/Dockerfile .` fails at the ollama install block (lines 122-138) with `wget exit code 8` (HTTP 404). Three structural bugs:

1. **Wrong asset name.** Fetches `ollama-linux-amd64` — real asset is `ollama-linux-amd64.tgz` (tarball).
2. **Wrong checksum URL.** Fetches per-binary `ollama-linux-amd64.sha256` — does not exist. Real checksum file is `sha256sum.txt` (single file with all hashes).
3. **Missing tarball extraction.** Even if the URL were right, the asset is a `.tgz` containing `bin/ollama` + `lib/ollama/cuda_v*/` — needs `tar -xzf` then install extracted binary.

## Decision: bin-only (no CUDA libs)

Install only `bin/ollama` from the tarball. Skip `lib/ollama/cuda_v11/` and `lib/ollama/cuda_v12/` libraries.

**Rationale:** Mika OS targets CPU-only inference as the minimum-viable shape. The ollama binary works standalone for CPU inference. GPU acceleration (CUDA) is a deferrable enhancement — it would add ~1.5GB to the image and require NVIDIA runtime configuration. The issue body recommends bin-only for v0.

**Runtime size delta:** ~0 — the ollama binary itself is the same size whether extracted alone or with CUDA libs. Skipping CUDA libs avoids ~1.5GB of shared libraries.

## Scope

### In scope
- Fix the three bugs in the ollama install RUN block (lines 122-138)
- Audit all other non-portage install RUN blocks in `os/Dockerfile` for similar asset-name/checksum-format assumptions

### Out of scope
- GPU/CUDA support (future enhancement)
- Changing ollama version (stays at 0.6.0)
- Changes to `Dockerfile.agent` or `Dockerfile.gateway` (separate images)

## Audit of other install blocks (AC4)

| Block | Lines | Asset format | Checksum format | Status |
|-------|-------|-------------|-----------------|--------|
| GitHub CLI (gh) | 89-103 | `gh_<ver>_linux_<arch>.tar.gz` | `gh_<ver>_checksums.txt` (grep per-asset) | ✅ Correct |
| Google Workspace CLI (gws) | 106-120 | `gws-<triple>.tar.gz` | `gws-<triple>.tar.gz.sha256` (per-asset) | ✅ Correct (fixed in #1249) |
| Ollama | 122-138 | `ollama-linux-<arch>` (WRONG) | `ollama-linux-<arch>.sha256` (WRONG) | ❌ Three bugs |

**Result:** Only the ollama block has asset-name/checksum-format bugs. The gh and gws blocks use the correct patterns for their respective release formats.

## Implementation

### Phase 1: Fix the ollama install RUN block

Replace lines 122-138 in `os/Dockerfile` with:

```dockerfile
# Ollama (sha256 verified)
ARG OLLAMA_VERSION=0.6.0
RUN ARCH=$(uname -m) && \
    case "$ARCH" in \
        x86_64) OLLAMA_ARCH="amd64" ;; \
        aarch64) OLLAMA_ARCH="arm64" ;; \
        *) echo "Unsupported architecture: $ARCH" && exit 1 ;; \
    esac && \
    wget -qO "/tmp/ollama-linux-${OLLAMA_ARCH}.tgz" \
        "https://github.com/ollama/ollama/releases/download/v${OLLAMA_VERSION}/ollama-linux-${OLLAMA_ARCH}.tgz" && \
    wget -qO /tmp/ollama-checksums.txt \
        "https://github.com/ollama/ollama/releases/download/v${OLLAMA_VERSION}/sha256sum.txt" && \
    cd /tmp && grep "ollama-linux-${OLLAMA_ARCH}.tgz" ollama-checksums.txt | sha256sum -c - && \
    tar -xzf "/tmp/ollama-linux-${OLLAMA_ARCH}.tgz" -C /tmp && \
    install -m 755 /tmp/bin/ollama /usr/local/bin/ollama && \
    rm -rf /tmp/ollama-* /tmp/bin /tmp/lib
```

Changes:
1. Asset URL: `ollama-linux-${OLLAMA_ARCH}` → `ollama-linux-${OLLAMA_ARCH}.tgz`
2. Checksum URL: per-binary `.sha256` → release-level `sha256sum.txt`
3. Checksum verification: grep the tarball filename from `sha256sum.txt`, pipe to `sha256sum -c -`
4. Add `tar -xzf` extraction step
5. Install from extracted `bin/ollama` path (not raw download)
6. Cleanup includes extracted `bin/` and `lib/` directories

### Phase 2: Verify

- `docker build --target mika-os -f os/Dockerfile -t mika-os:dev .` succeeds
- `docker build --target mika-runtime -f os/Dockerfile -t mika-runtime:dev .` succeeds
- `docker run --rm mika-os:dev /usr/local/bin/ollama --version` prints version
- `ls -la` confirms `/usr/local/bin/ollama` exists with mode 755

## AC tie-back

| AC | Deliverable |
|----|------------|
| AC1 | `mika-os` target builds end-to-end (no wget 404, no sha256sum failure, ollama at `/usr/local/bin/ollama` mode 755) |
| AC2 | `mika-runtime` target builds (COPY --from=mika-os inherits the fixed binary) |
| AC3 | This plan addresses bin-only decision explicitly with runtime size delta rationale |
| AC4 | Audit table above covers all three non-portage install blocks; gh and gws confirmed correct |

## Risk

**Low.** Single-file edit to a Dockerfile RUN block. The fix mirrors the working gh install pattern (lines 89-103) which uses the same tarball + checksums.txt + grep + sha256sum -c pipeline. No Rust code changes. No schema changes.
