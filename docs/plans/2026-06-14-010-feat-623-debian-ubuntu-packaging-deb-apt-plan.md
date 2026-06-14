# Plan: feat(ci): Debian/Ubuntu packaging (.deb) with apt repo

**Issue:** mika#623
**Milestone:** Distribution channels (#11)
**Type:** feat
**Branch:** `feat/623/ci-debian-ubuntu-packaging-deb-with-apt`

## Context

Mika currently distributes Linux binaries as gzipped tarballs uploaded to GitHub Releases. Users must manually download, extract, and place binaries. There is no native package manager integration for Debian/Ubuntu, which is the most common Linux distribution family for server deployments.

This ticket adds `.deb` package generation via `cargo-deb` to the release pipeline, with a hosted APT repository so users can install and update via `apt-get install mika`.

### Current state

- Release workflow (`.github/workflows/release.yml`) builds two binaries for four targets: `mika` (CLI) and `mika-server` (HTTP agent), using `taiki-e/upload-rust-binary-action`
- Linux targets: `x86_64-unknown-linux-gnu` (native Ubuntu 22.04), `aarch64-unknown-linux-gnu` (cross-compiled)
- Release trigger: `v*` tag push (via release-please)
- Existing artifacts: `.tar.gz` + SHA256 checksums + SBOM + cosign signatures + GitHub attestations
- `mika-gateway` is Docker-only (excluded from binary releases)
- `calibrate` binary is internal-only (not released)

### Open question from ticket

> GitHub Pages + reprepro vs Cloudsmith free tier

**Decision: GitHub Pages + reprepro.** Rationale:
- Zero external dependency — stays within the GitHub ecosystem already used for releases
- No vendor lock-in or free-tier limits to worry about
- `reprepro` is the standard, well-documented tool for self-hosted Debian repos
- GPG signing integrates with existing cosign/sigstore workflow patterns
- The repo is a static file tree — GitHub Pages serves it for free

## Scope

### In scope

1. `cargo-deb` metadata in `Cargo.toml` for `mika-cli` (produces `mika` binary) and `mika-agent` (produces `mika-server` binary)
2. Two `.deb` packages: `mika` (CLI tool) and `mika-server` (HTTP agent daemon)
3. `postinst`/`prerm` scripts for `mika-server` (systemd service lifecycle)
4. systemd service unit file for `mika-server`
5. New CI workflow job in `release.yml` that builds `.deb` packages and uploads to GitHub Release
6. APT repository hosted on GitHub Pages via `reprepro` in a separate repo (`senara-solutions/apt`)
7. CI job that publishes `.deb` packages to the APT repo after release
8. GPG key for signing the APT repository

### Out of scope

- Gentoo ebuild (mika#624)
- Homebrew formula (mika#625)
- Release artifact hardening (mika#622 — already groomed, blocked on mika#1410)
- RPM packaging (no ticket filed yet)
- `mika-gateway` packaging (Docker-only deployment model)
- `calibrate` binary packaging (internal tooling)

## Implementation

### Step 1: systemd service unit for mika-server

Create `packaging/systemd/mika-server.service`:

```ini
[Unit]
Description=Mika AI Agent HTTP Server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=mika
Group=mika
ExecStart=/usr/bin/mika-server
Restart=on-failure
RestartSec=5
Environment=MIKA_HOME=/var/lib/mika
Environment=MIKA_LOG_FORMAT=json
Environment=MIKA_SERVER_LOG_FILE=/var/log/mika/server.log
EnvironmentFile=-/etc/mika/mika-server.env
WorkingDirectory=/var/lib/mika

# Hardening
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=/var/lib/mika /var/log/mika
PrivateTmp=yes

[Install]
WantedBy=multi-user.target
```

**Files:** `packaging/systemd/mika-server.service` (new)

### Step 2: Maintainer scripts for mika-server

Create `packaging/debian/mika-server.postinst`:

```bash
#!/bin/sh
set -e

case "$1" in
  configure)
    # Create mika user/group if not exists
    if ! getent group mika >/dev/null 2>&1; then
      groupadd --system mika
    fi
    if ! getent passwd mika >/dev/null 2>&1; then
      useradd --system --gid mika --home-dir /var/lib/mika --shell /usr/sbin/nologin mika
    fi

    # Create runtime directories
    install -d -m 0755 -o mika -g mika /var/lib/mika
    install -d -m 0755 -o mika -g mika /var/log/mika
    install -d -m 0750 -o mika -g mika /etc/mika

    # Enable and restart service on upgrade
    systemctl daemon-reload
    systemctl enable mika-server.service
    if systemctl is-active --quiet mika-server.service; then
      systemctl restart mika-server.service
    fi
    ;;
esac
```

Create `packaging/debian/mika-server.prerm`:

```bash
#!/bin/sh
set -e

case "$1" in
  remove|purge)
    systemctl stop mika-server.service || true
    systemctl disable mika-server.service || true
    ;;
esac
```

**Files:** `packaging/debian/mika-server.postinst`, `packaging/debian/mika-server.prerm` (new)

### Step 3: cargo-deb metadata in Cargo.toml

Add `[package.metadata.deb]` sections to both binary crates.

**`crates/mika-cli/Cargo.toml`** — append:

```toml
[package.metadata.deb]
name = "mika"
maintainer = "Senara Solutions <engineering@senara.solutions>"
copyright = "2025-2026 Senara Solutions"
depends = "$auto, jq"
section = "utils"
priority = "optional"
extended-description = """Mika is a conversation-first AI executive assistant with persistent memory.
The mika CLI provides a TUI chat interface and management commands for agents, skills, memory, and tasks."""
assets = [
  ["target/release/mika", "usr/bin/mika", "755"],
]
```

**`crates/mika-agent/Cargo.toml`** — append:

```toml
[package.metadata.deb]
name = "mika-server"
maintainer = "Senara Solutions <engineering@senara.solutions>"
copyright = "2025-2026 Senara Solutions"
depends = "$auto, jq"
section = "net"
priority = "optional"
extended-description = """Mika AI Agent HTTP Server.
Per-customer container isolation with SQLite storage, Axum-based HTTP API,
embedded dashboard, and A2A protocol support."""
assets = [
  ["target/release/mika-server", "usr/bin/mika-server", "755"],
]
systemd-units = { unit-name = "mika-server", enable = false }
maintainer-scripts = "../../packaging/debian"
conf-files = ["/etc/mika/mika-server.env"]
```

Notes:
- `$auto` resolves shared library dependencies via `dpkg-shlibdeps`
- `jq` is a runtime dependency (required by skill handler scripts, per CLAUDE.md)
- `systemd-units` tells cargo-deb to include the service file (it looks in the standard path)
- `enable = false` — postinst handles enable/start explicitly
- `calibrate` binary is excluded (internal tooling, not for distribution)

**Files:** `crates/mika-cli/Cargo.toml`, `crates/mika-agent/Cargo.toml` (modify)

### Step 4: Default environment file

Create `packaging/debian/mika-server.env`:

```bash
# Mika Server Configuration
# See https://github.com/senara-solutions/mika for full documentation
#
# Required:
# MIKA_ANTHROPIC_API_KEY=sk-ant-...
# MIKA_ROUTING_URL=http://localhost:3001
# MIKA_INTERNAL_TOKEN=...
#
# Optional:
# MIKA_DEV_MODE=false
# MIKA_LOG_FORMAT=json
# MIKA_TELEMETRY_ENABLED=false
```

This file is installed to `/etc/mika/mika-server.env` as a conffile (preserved on upgrade).

**Files:** `packaging/debian/mika-server.env` (new)

### Step 5: CI workflow — build .deb packages

Add a `deb` job to `.github/workflows/release.yml` that runs after the `build` job:

```yaml
deb:
  name: Build .deb ${{ matrix.target }}
  runs-on: ${{ matrix.os }}
  needs: [build]
  if: github.repository_owner == 'senara-solutions'
  strategy:
    fail-fast: false
    matrix:
      include:
        - target: x86_64-unknown-linux-gnu
          os: ubuntu-22.04
          arch: amd64
        - target: aarch64-unknown-linux-gnu
          os: ubuntu-22.04
          arch: arm64
  steps:
    - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd  # v6

    - name: Install Rust toolchain
      uses: dtolnay/rust-toolchain@631a55b12751854ce901bb631d5902ceb48146f7  # stable
      with:
        targets: ${{ matrix.target }}

    - uses: Swatinem/rust-cache@779680da715d629ac1d338a641029a2f4372abb5  # v2
      with:
        prefix-key: v1-release
        key: ${{ matrix.target }}

    - name: Setup cross-compilation toolchain
      if: matrix.target == 'aarch64-unknown-linux-gnu'
      uses: taiki-e/setup-cross-toolchain-action@b8d1a322a6009a2b7220f53996695778eef89b41  # v1
      with:
        target: ${{ matrix.target }}

    - name: Install cargo-deb
      run: cargo install cargo-deb

    - name: Create dashboard dist placeholder
      run: mkdir -p dashboard/dist

    - name: Build mika .deb
      run: cargo deb -p mika-cli --target ${{ matrix.target }} --no-build
      env:
        CARGO_BUILD_TARGET: ${{ matrix.target }}

    - name: Build mika-server .deb
      run: cargo deb -p mika-agent --target ${{ matrix.target }} --no-build
      env:
        CARGO_BUILD_TARGET: ${{ matrix.target }}

    - name: Upload .deb packages to release
      env:
        GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      run: |
        tag="${{ github.ref_name }}"
        for deb in target/${{ matrix.target }}/debian/*.deb; do
          gh release upload "$tag" "$deb"
        done
```

Wait — `--no-build` requires binaries to already exist. Since the `build` job uses `upload-rust-binary-action` which builds and uploads in one step, the binaries exist only as release artifacts, not as reusable build cache across jobs.

**Revised approach:** The `deb` job must build from source (cargo-deb handles the build internally). This duplicates the compile but ensures correct binary paths for `dpkg-deb`. The rust-cache mitigates compile time. Alternatively, we could restructure to share artifacts, but that adds complexity. The simpler approach: let `cargo deb` build with `--features telemetry`.

```yaml
    - name: Build mika .deb
      run: cargo deb -p mika-cli --target ${{ matrix.target }} -- --features telemetry

    - name: Build mika-server .deb
      run: cargo deb -p mika-agent --target ${{ matrix.target }} -- --features telemetry
```

The `sign` job already runs after all artifact-producing jobs via `needs: [build, sbom]`. Update it to `needs: [build, sbom, deb]` so `.deb` files are signed too. The sign job's `for file in *.tar.gz *.sha256 *.cdx.json` glob needs extending to include `*.deb`.

**Files:** `.github/workflows/release.yml` (modify)

### Step 6: APT repository setup

Create a new repository `senara-solutions/apt` with GitHub Pages enabled. This repo holds the `reprepro` database and serves as the APT endpoint.

Structure:
```
apt/
├── conf/
│   ├── distributions    # reprepro config
│   └── options          # reprepro options
├── pool/                # .deb files (managed by reprepro)
├── dists/               # release metadata (managed by reprepro)
└── .github/
    └── workflows/
        └── publish.yml  # triggered by mika release
```

`conf/distributions`:
```
Origin: Senara Solutions
Label: Mika
Suite: stable
Codename: stable
Architectures: amd64 arm64
Components: main
Description: Mika AI Executive Assistant
SignWith: <GPG_KEY_ID>
```

This step is a manual setup task (creating the repo, generating GPG key, configuring Pages). The CI integration in Step 7 automates the publish flow.

**Files:** New repo `senara-solutions/apt` (manual creation)

### Step 7: CI workflow — publish to APT repo

Add a `publish-apt` job to `release.yml` that runs after `sign`:

```yaml
publish-apt:
  name: Publish to APT repository
  runs-on: ubuntu-22.04
  needs: [sign]
  if: github.repository_owner == 'senara-solutions'
  steps:
    - name: Install reprepro
      run: sudo apt-get update && sudo apt-get install -y reprepro

    - name: Checkout APT repo
      uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd  # v6
      with:
        repository: senara-solutions/apt
        token: ${{ secrets.APT_REPO_TOKEN }}
        path: apt-repo

    - name: Import GPG key
      run: echo "${{ secrets.APT_GPG_PRIVATE_KEY }}" | gpg --batch --import

    - name: Download .deb packages from release
      env:
        GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      run: |
        tag="${{ github.ref_name }}"
        mkdir debs
        gh release download "$tag" \
          --repo "${{ github.repository }}" \
          --pattern "*.deb" \
          --dir debs

    - name: Add packages to repo
      run: |
        cd apt-repo
        for deb in ../debs/*.deb; do
          reprepro includedeb stable "$deb"
        done

    - name: Push APT repo
      run: |
        cd apt-repo
        git config user.name "github-actions[bot]"
        git config user.email "github-actions[bot]@users.noreply.github.com"
        git add -A
        git commit -m "release: add $(basename ../debs/*.deb | head -1 | sed 's/_.*//') packages"
        git push
```

**Secrets required:**
- `APT_REPO_TOKEN` — PAT with `contents: write` on `senara-solutions/apt`
- `APT_GPG_PRIVATE_KEY` — GPG private key for APT repo signing

**Files:** `.github/workflows/release.yml` (modify)

### Step 8: Installation documentation

Add user-facing installation instructions to `docs/getting-started.md` (or create if absent):

```markdown
## Install via APT (Debian/Ubuntu)

```bash
# Add GPG key
curl -fsSL https://senara-solutions.github.io/apt/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/mika-archive-keyring.gpg

# Add repository
echo "deb [signed-by=/usr/share/keyrings/mika-archive-keyring.gpg] https://senara-solutions.github.io/apt stable main" | sudo tee /etc/apt/sources.list.d/mika.list

# Install
sudo apt-get update
sudo apt-get install mika          # CLI only
sudo apt-get install mika-server   # HTTP agent server (includes systemd service)
```

**Files:** `docs/getting-started.md` (modify or create)

## File change summary

| File | Action | Purpose |
|------|--------|---------|
| `crates/mika-cli/Cargo.toml` | Modify | Add `[package.metadata.deb]` section |
| `crates/mika-agent/Cargo.toml` | Modify | Add `[package.metadata.deb]` section |
| `packaging/systemd/mika-server.service` | Create | systemd unit file |
| `packaging/debian/mika-server.postinst` | Create | Post-install script (user creation, service enable) |
| `packaging/debian/mika-server.prerm` | Create | Pre-remove script (service stop) |
| `packaging/debian/mika-server.env` | Create | Default env file template |
| `.github/workflows/release.yml` | Modify | Add `deb` and `publish-apt` jobs, extend `sign` |
| `docs/getting-started.md` | Modify/Create | Installation instructions |

## Risks and mitigations

1. **Cross-compiled .deb for aarch64:** `cargo-deb` may not correctly resolve `dpkg-shlibdeps` for cross-compiled targets. Mitigation: use `--no-strip` and test the aarch64 .deb in a container. If `dpkg-shlibdeps` fails, fall back to explicit `depends` without `$auto`.

2. **Build time doubling:** The `deb` job rebuilds from source (can't reuse `build` job artifacts directly). Mitigation: `rust-cache` with the same prefix-key shares the compilation cache. Incremental: the bulk of compile time is cached; only the final link step runs.

3. **GPG key management:** The APT signing key must be generated, stored as a GitHub secret, and its public key published. Mitigation: document the key generation and rotation procedure.

4. **GitHub Pages rate limits:** APT `apt-get update` hits GitHub Pages on every client update check. Mitigation: GitHub Pages has generous bandwidth for static sites; this is standard practice (e.g., `cli.github.com` uses the same pattern).

## Testing

- [ ] Build `.deb` locally with `cargo deb -p mika-cli` and `cargo deb -p mika-agent`
- [ ] Install on a clean Debian/Ubuntu container: `dpkg -i mika_*.deb`
- [ ] Verify binary paths: `/usr/bin/mika`, `/usr/bin/mika-server`
- [ ] Verify systemd service: `systemctl status mika-server`
- [ ] Verify postinst creates `mika` user and directories
- [ ] Verify upgrade path: install v1, upgrade to v2, confirm service restarts
- [ ] Verify removal: `apt-get remove mika-server` stops service cleanly
- [ ] Test aarch64 .deb in an arm64 container
