---
title: "feat: Automated Release System with GitHub Binary Downloads"
type: feat
status: completed
date: 2026-03-01
---

# Automated Release System with GitHub Binary Downloads

## Overview

Implement a complete CI/CD and release pipeline for the Mika project using GitHub Actions. The system will:
1. Run CI checks (fmt, clippy, test) on every PR and push to main
2. Automate version bumps and changelogs via release-plz (conventional commits)
3. Publish crates to crates.io in dependency order
4. Build cross-platform binaries and upload them to GitHub Releases
5. Provide a one-liner install script for end users

## Problem Statement / Motivation

Mika is at Phase 4 (deployment infrastructure) with Dockerfiles complete but no CI/CD. Users currently must build from source or use `cargo install mika-cli` (which requires a Rust toolchain + C compiler for bundled SQLite). Pre-built binaries on GitHub Releases would dramatically lower the installation barrier.

The crates have been prepared for crates.io publishing (commits 2eca502, cf37ba6), but there is no automated release flow. Manual releases are error-prone and unsustainable.

## Proposed Solution

**Three GitHub Actions workflows + supporting configuration:**

```
Developer pushes conventional commits to main
         │
         ▼
  ┌─────────────┐     ┌──────────────────┐
  │  ci.yml     │     │ release-plz.yml  │
  │ (fmt/clippy │     │ (creates release │
  │  /test)     │     │  PR with version │
  └─────────────┘     │  bump+changelog) │
                      └────────┬─────────┘
                               │ maintainer merges release PR
                               ▼
                      ┌──────────────────┐
                      │ release-plz.yml  │
                      │ (publishes to    │
                      │  crates.io,      │
                      │  creates git tag)│
                      └────────┬─────────┘
                               │ tag push (v*)
                               ▼
                      ┌──────────────────┐
                      │  release.yml     │
                      │ (builds binaries │
                      │  for 4 targets,  │
                      │  uploads to GH   │
                      │  Release)        │
                      └──────────────────┘
```

**Tool choices:**
- **release-plz** for version management, changelogs, crates.io publishing, git tagging
- **taiki-e/upload-rust-binary-action** for cross-platform binary builds (more control than cargo-dist for our multi-binary workspace)
- **taiki-e/setup-cross-toolchain-action** for aarch64-linux cross-compilation
- **Swatinem/rust-cache** for CI build caching

## Technical Approach

### Phase 1: CI Workflow + CLI Version Support

**File: `.github/workflows/ci.yml`**

Triggers on PR and push to main. Jobs:
- `check`: cargo fmt --check, cargo clippy -D warnings, cargo build, cargo test
- Uses `Swatinem/rust-cache@v2` for caching
- Uses `dtolnay/rust-toolchain@stable` (respects rust-toolchain.toml → 1.93)
- Runner: `ubuntu-22.04` (not `latest`) for glibc compatibility awareness

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always
  CARGO_INCREMENTAL: 0

jobs:
  check:
    name: Check
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
        with:
          prefix-key: "v1-rust"

      - name: Check formatting
        run: cargo fmt --all -- --check

      - name: Clippy
        run: cargo clippy --all-targets --all-features -- -D warnings

      - name: Build
        run: cargo build --all-targets

      - name: Test
        run: cargo test
```

**File: `crates/mika-cli/src/cli.rs` (line 4)**

Add `version` to clap command attribute so `mika --version` works:

```rust
// Before:
#[command(name = "mika", about = "Mika — AI Executive Assistant")]

// After:
#[command(name = "mika", version, about = "Mika — AI Executive Assistant")]
```

This auto-populates from `Cargo.toml` version via `env!("CARGO_PKG_VERSION")`.

### Phase 2: release-plz Configuration

**File: `release-plz.toml`**

```toml
[workspace]
# Let release-plz create GitHub Releases (with changelog body)
git_release_enable = true
git_tag_enable = true
git_tag_name = "v{{ version }}"
changelog_update = true
dependencies_update = true
semver_check = false              # not all crates are public-API libraries
allow_dirty = false
pr_labels = ["release"]
features_always_increment_minor = false
release_always = false            # only release when release PR is merged

[[package]]
name = "mika-common"
publish = true
changelog_path = "CHANGELOG.md"
changelog_include = ["mika-agent", "mika-cli"]

[[package]]
name = "mika-agent"
publish = true
# Per-crate changelog not needed — aggregated in root CHANGELOG.md
changelog_update = false

[[package]]
name = "mika-cli"
publish = true
changelog_update = false

[[package]]
name = "mika-gateway"
release = false                   # publish = false in Cargo.toml, skip entirely

[changelog]
header = """# Changelog\n\nAll notable changes to this project will be documented in this file.\n"""
body = """
## [{{ version }}](https://github.com/senara-solutions/mika/releases/tag/v{{ version }}) — {{ timestamp | date(format="%Y-%m-%d") }}
{% for group, commits in commits | group_by(attribute="group") %}
### {{ group | upper_first }}
{% for commit in commits %}
- {% if commit.scope %}*({{ commit.scope }})* {% endif %}{% if commit.breaking %}**BREAKING** {% endif %}{{ commit.message }}
{%- endfor %}
{% endfor %}
"""
trim = true
protect_breaking_commits = true
commit_parsers = [
    { message = "^feat", group = "Added" },
    { message = "^fix", group = "Fixed" },
    { message = "^refactor", group = "Changed" },
    { message = "^perf", group = "Performance" },
    { message = "^doc", group = "Documentation" },
    { message = "^test", skip = true },
    { message = "^ci", skip = true },
    { message = "^chore", skip = true },
    { message = "^style", skip = true },
]
```

**File: `.github/workflows/release-plz.yml`**

Two jobs: `release-plz-pr` (creates/updates release PR) and `release-plz-release` (publishes + tags on merge).

**Critical:** Uses a PAT (`RELEASE_PLZ_TOKEN`) instead of `GITHUB_TOKEN` so that tag creation triggers the downstream `release.yml` workflow. `GITHUB_TOKEN`-created events do NOT trigger other workflows.

```yaml
# .github/workflows/release-plz.yml
name: Release-plz

on:
  push:
    branches:
      - main

permissions:
  contents: write
  pull-requests: write

jobs:
  release-plz-pr:
    name: Release PR
    runs-on: ubuntu-22.04
    if: github.repository_owner == 'senara-solutions'
    concurrency:
      group: release-plz-pr
      cancel-in-progress: false
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
          token: ${{ secrets.RELEASE_PLZ_TOKEN }}
      - uses: dtolnay/rust-toolchain@stable
      - uses: release-plz/action@v0.5
        with:
          command: release-pr
        env:
          GITHUB_TOKEN: ${{ secrets.RELEASE_PLZ_TOKEN }}

  release-plz-release:
    name: Release
    runs-on: ubuntu-22.04
    if: github.repository_owner == 'senara-solutions'
    concurrency:
      group: release-plz-release
      cancel-in-progress: false
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
          token: ${{ secrets.RELEASE_PLZ_TOKEN }}
      - uses: dtolnay/rust-toolchain@stable
      - uses: release-plz/action@v0.5
        with:
          command: release
        env:
          GITHUB_TOKEN: ${{ secrets.RELEASE_PLZ_TOKEN }}
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

### Phase 3: Binary Release Workflow

**File: `.github/workflows/release.yml`**

Triggered by tag push (`v*`). Builds `mika` CLI binary for 4 targets. Server binaries (`mika-spirit`, `mika-gateway`) are Docker-only and excluded from binary releases.

```yaml
# .github/workflows/release.yml
name: Release Binaries

on:
  push:
    tags:
      - "v*"

permissions:
  contents: write

jobs:
  build:
    name: Build ${{ matrix.target }}
    runs-on: ${{ matrix.os }}
    if: github.repository_owner == 'senara-solutions'
    strategy:
      fail-fast: false
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-22.04
          - target: aarch64-unknown-linux-gnu
            os: ubuntu-22.04
          - target: x86_64-apple-darwin
            os: macos-13
          - target: aarch64-apple-darwin
            os: macos-14

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - uses: Swatinem/rust-cache@v2
        with:
          prefix-key: "v1-release"
          key: ${{ matrix.target }}

      - name: Setup cross-compilation toolchain
        if: matrix.target == 'aarch64-unknown-linux-gnu'
        uses: taiki-e/setup-cross-toolchain-action@v1
        with:
          target: ${{ matrix.target }}

      - name: Build and upload mika
        uses: taiki-e/upload-rust-binary-action@v1
        with:
          bin: mika
          target: ${{ matrix.target }}
          archive: mika-$tag-$target
          checksum: sha256
          token: ${{ secrets.GITHUB_TOKEN }}
```

**Build matrix rationale:**

| Target | Runner | Notes |
|--------|--------|-------|
| `x86_64-unknown-linux-gnu` | `ubuntu-22.04` | glibc 2.35, broad compat |
| `aarch64-unknown-linux-gnu` | `ubuntu-22.04` | Cross-compiled via setup-cross-toolchain |
| `x86_64-apple-darwin` | `macos-13` | Intel Mac runner |
| `aarch64-apple-darwin` | `macos-14` | Apple Silicon (M1) runner |

**Why only `mika` CLI binary?**
- `mika-spirit` and `mika-gateway` are deployed via Docker containers with their own Dockerfiles
- Including them would increase build time and create confusion (users don't need server binaries)
- Docker images can be automated separately in a future workflow

### Phase 4: Install Script

**File: `install.sh`** (repository root)

```bash
#!/bin/sh
# Install Mika CLI — AI Executive Assistant
# Usage: curl -fsSL https://raw.githubusercontent.com/senara-solutions/mika/main/install.sh | sh
# Pin a version: curl -fsSL ... | sh -s -- v0.2.0
set -eu

REPO="senara-solutions/mika"
BINARY="mika"
INSTALL_DIR="${MIKA_INSTALL_DIR:-$HOME/.local/bin}"

# Accept optional version argument (e.g., v0.2.0)
VERSION="${1:-}"

# Detect platform
ARCH=$(uname -m)
OS=$(uname -s)

case "${OS}" in
    Linux)   TARGET_OS="unknown-linux-gnu" ;;
    Darwin)  TARGET_OS="apple-darwin" ;;
    *)       echo "Error: Unsupported OS '${OS}'. Mika supports Linux and macOS." >&2; exit 1 ;;
esac

case "${ARCH}" in
    x86_64|amd64)  TARGET_ARCH="x86_64" ;;
    aarch64|arm64) TARGET_ARCH="aarch64" ;;
    *)             echo "Error: Unsupported architecture '${ARCH}'. Mika supports x86_64 and aarch64." >&2; exit 1 ;;
esac

TARGET="${TARGET_ARCH}-${TARGET_OS}"

# Get version (latest release or user-specified)
if [ -z "${VERSION}" ]; then
    VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')
    if [ -z "${VERSION}" ]; then
        echo "Error: Could not determine latest release." >&2; exit 1
    fi
fi

ARCHIVE="${BINARY}-${VERSION}-${TARGET}"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}.tar.gz"
CHECKSUM_URL="${URL}.sha256"

echo "Installing ${BINARY} ${VERSION} for ${TARGET}..."
TMPDIR=$(mktemp -d)
trap 'rm -rf "${TMPDIR}"' EXIT

curl -fsSL "${URL}" -o "${TMPDIR}/${ARCHIVE}.tar.gz"

# Verify checksum (platform-aware)
echo "Verifying checksum..."
EXPECTED=$(curl -fsSL "${CHECKSUM_URL}" | awk '{print $1}')
case "${OS}" in
    Linux)  ACTUAL=$(sha256sum "${TMPDIR}/${ARCHIVE}.tar.gz" | awk '{print $1}') ;;
    Darwin) ACTUAL=$(shasum -a 256 "${TMPDIR}/${ARCHIVE}.tar.gz" | awk '{print $1}') ;;
esac

if [ "${EXPECTED}" != "${ACTUAL}" ]; then
    echo "Error: Checksum mismatch! Expected ${EXPECTED}, got ${ACTUAL}" >&2
    exit 1
fi

# Extract and install
tar -xzf "${TMPDIR}/${ARCHIVE}.tar.gz" -C "${TMPDIR}"
mkdir -p "${INSTALL_DIR}"
mv "${TMPDIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
chmod +x "${INSTALL_DIR}/${BINARY}"

echo ""
echo "${BINARY} ${VERSION} installed to ${INSTALL_DIR}/${BINARY}"

# Check PATH
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *) echo "Warning: ${INSTALL_DIR} is not in your PATH. Add it:"
       echo "  export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
esac
```

### Phase 5: TLS Portability Fix

**Problem:** `reqwest 0.12` defaults to `native-tls` (OpenSSL). Binaries dynamically linked against OpenSSL are not portable across Linux distributions with different OpenSSL versions.

**File: `Cargo.toml` (workspace root, line 21)**

```toml
# Before:
reqwest = { version = "0.12", features = ["json"] }

# After:
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls-native-roots"] }
```

This switches to rustls (pure Rust TLS) with the system's native root certificates, eliminating the OpenSSL dynamic linkage. The `rmcp` crate also uses reqwest and should inherit the workspace TLS settings.

**Validation:** Run `cargo test` after the change to ensure no regressions. Check that `ldd target/release/mika` shows no `libssl` dependency.

### Phase 6: Documentation Updates

**File: `docs/getting-started.md`** (update Installation section)

Add binary download and install script options before the existing "build from source" instructions:

```markdown
## 2. Installation

### Quick install (Linux / macOS)

```sh
curl -fsSL https://raw.githubusercontent.com/senara-solutions/mika/main/install.sh | sh
```

This downloads the latest pre-built binary for your platform and installs it to `~/.local/bin/mika`.

### Download from GitHub Releases

Pre-built binaries are available on the [Releases page](https://github.com/senara-solutions/mika/releases). Download the archive for your platform, extract, and place the `mika` binary on your PATH.

### Install from crates.io

```sh
cargo install mika-cli
```

### Build from source

```sh
git clone https://github.com/senara-solutions/mika.git
cd mika
cargo build --release
cp target/release/mika ~/.local/bin/
```
```

**File: `CLAUDE.md`** — Update "Current phase" and "Pending Work" to reflect CI/CD completion.

**File: `README.md`** — Add installation section if not present.

## Acceptance Criteria

### Functional Requirements

- [ ] `mika --version` prints the crate version (e.g., `mika 0.1.0`)
- [ ] CI workflow runs on every PR: fmt, clippy, build, test
- [ ] CI workflow runs on push to main
- [ ] release-plz creates a release PR when conventional commits land on main
- [ ] Release PR contains version bump in root `Cargo.toml` and `CHANGELOG.md` update
- [ ] Merging the release PR publishes crates to crates.io (mika-common → mika-agent → mika-cli)
- [ ] Merging the release PR creates a git tag (`v{version}`)
- [ ] Tag push triggers binary builds for 4 targets
- [ ] GitHub Release page contains: 4 binary archives + 4 SHA256 checksum files
- [ ] `install.sh` successfully installs on Linux x86_64, Linux aarch64, macOS x86_64, macOS aarch64
- [ ] `install.sh` verifies SHA256 checksum before installing
- [ ] Distributed binaries have no OpenSSL dynamic linkage (rustls-tls)

### Non-Functional Requirements

- [ ] CI completes in under 10 minutes
- [ ] Binary release builds complete in under 20 minutes
- [ ] All workflows use least-privilege `permissions:` blocks
- [ ] All workflows include `concurrency:` groups to prevent parallel runs
- [ ] Fork safety: `if: github.repository_owner == 'senara-solutions'` on release workflows

### Quality Gates

- [ ] All existing ~680 tests pass after TLS switch
- [ ] `ldd target/release/mika` shows no `libssl` linkage
- [ ] `cargo clippy` clean after all changes

## Dependencies & Prerequisites

### Repository Secrets (must be configured before workflows run)

| Secret | Purpose | How to create |
|--------|---------|---------------|
| `RELEASE_PLZ_TOKEN` | PAT with `contents:write` + `pull_requests:write` | GitHub Settings → Developer settings → Personal access tokens → Fine-grained → Scope to senara-solutions/mika |
| `CARGO_REGISTRY_TOKEN` | crates.io API token | https://crates.io/settings/tokens → Create scoped to mika-common, mika-agent, mika-cli |

### External Dependencies

- Conventional commits must be adopted by all contributors (enforced by convention, not CI initially)
- crates.io accounts must have publish access for the three crates

## Risk Analysis & Mitigation

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| `GITHUB_TOKEN` cannot trigger downstream workflows | No binaries built on release | High (known GitHub limitation) | Use PAT via `RELEASE_PLZ_TOKEN` |
| Partial crates.io publish (mika-common publishes but mika-agent fails) | Inconsistent state | Medium | release-plz handles "already published" as success; re-run is idempotent |
| aarch64-linux cross-compile fails due to bundled SQLite C deps | No ARM Linux binary | Medium | `setup-cross-toolchain-action` provides gcc-aarch64-linux-gnu; fallback to `cross` |
| OpenSSL dynamic linkage in binaries | Binaries fail on other distros | High | Switch to `rustls-tls-native-roots` in Phase 5 |
| macOS Gatekeeper warnings for unsigned binaries | Poor first-run UX on macOS | High | Document workaround in install script output; code signing is future work |
| glibc version mismatch (runner too new) | Binaries fail on older Linux | Medium | Pin to `ubuntu-22.04` (glibc 2.35) |

## Future Considerations

- **Docker image workflow:** Automate Docker image builds on release (separate workflow, triggered by same tag)
- **macOS code signing:** Sign binaries with Apple Developer certificate
- **SLSA provenance attestation:** Generate supply chain attestation for binaries
- **Homebrew tap:** `senara-solutions/homebrew-mika` for `brew install mika`
- **Commitlint CI check:** Enforce conventional commits on PR titles
- **`mika self-update` command:** Built-in binary update mechanism
- **Trusted publishing:** Migrate from `CARGO_REGISTRY_TOKEN` to OIDC-based crates.io publishing
- **Dependabot:** Auto-update Cargo and GitHub Actions dependencies

## Implementation Order

1. **Phase 1:** Add `version` to CLI clap attribute + create `.github/workflows/ci.yml`
2. **Phase 5:** Switch reqwest to `rustls-tls-native-roots` (do this early — it affects all builds)
3. **Phase 2:** Add `release-plz.toml` + `.github/workflows/release-plz.yml`
4. **Phase 3:** Add `.github/workflows/release.yml`
5. **Phase 4:** Add `install.sh`
6. **Phase 6:** Update docs (getting-started.md, CLAUDE.md, README.md)

**Note:** Phases 1+5 should be in the first commit/PR. Phases 2+3+4 can be in a second commit. Phase 6 in a third.

## References & Research

### Internal References
- `Cargo.toml:1-122` — Workspace configuration, release profile, dependency versions
- `crates/mika-cli/src/cli.rs:4` — Clap CLI definition (missing `version`)
- `crates/mika-cli/Cargo.toml` — CLI crate metadata
- `crates/mika-gateway/Cargo.toml:9` — `publish = false`
- `Dockerfile.agent`, `Dockerfile.gateway` — Existing Docker builds
- `docs/getting-started.md:32-57` — Current installation instructions
- `rust-toolchain.toml` — Pinned to Rust 1.93

### External References
- [release-plz documentation](https://release-plz.ieni.dev/docs)
- [release-plz GitHub Actions quickstart](https://release-plz.ieni.dev/docs/github/quickstart)
- [taiki-e/upload-rust-binary-action](https://github.com/taiki-e/upload-rust-binary-action)
- [taiki-e/setup-cross-toolchain-action](https://github.com/taiki-e/setup-cross-toolchain-action)
- [Swatinem/rust-cache](https://github.com/Swatinem/rust-cache)
- [Fully Automated Releases for Rust Projects — Orhun's Blog](https://blog.orhun.dev/automated-rust-releases/)
- [Conventional Commits v1.0.0](https://www.conventionalcommits.org/en/v1.0.0/)
- [crates.io Trusted Publishing](https://crates.io/docs/trusted-publishing)

### Resolved Publishing Blockers (from todos/)
- `include_str!` paths now crate-relative (commit cf37ba6)
- Documentation updated for mika-cli package name (commit cf37ba6)
- Publishing metadata consistent across crates (commit cf37ba6)
- mika-gateway marked `publish = false` (commit cf37ba6)
- `rust-version` field added to workspace (commit cf37ba6)
