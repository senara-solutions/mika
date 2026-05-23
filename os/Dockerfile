# SPDX-License-Identifier: Apache-2.0
#
# Mika OS — Handbook-style Gentoo multi-stage build
#
# Two named build targets:
#   mika-os      — Full build environment (toolchain + binaries + portage tree)
#                  For developers, forkable reference, single-box self-host
#   mika-runtime — Production runtime (binaries + runtime deps only)
#                  For mika-cloud Helm deployments, minimal attack surface
#
# Build:
#   docker build --target mika-os      -f os/Dockerfile -t mika-os:dev .
#   docker build --target mika-runtime -f os/Dockerfile -t mika-runtime:dev .
#
# Run (either target):
#   docker run --cap-add=SYS_ADMIN --security-opt apparmor=unconfined \
#     -v mika-data:/home/mika/.mika mika-runtime:dev
#
# Requires BuildKit (DOCKER_BUILDKIT=1) for:
#   - Per-Dockerfile .dockerignore (os/Dockerfile.dockerignore)
#   - Cache mounts for cargo registry and target dir

# ============================================================================
# Stage 1: mika-os — Full handbook-style Gentoo build
# ============================================================================
FROM gentoo/stage3:20250505 AS mika-os

# ── Portage configuration (handbook-style) ──
RUN echo 'MAKEOPTS="-j'$(nproc)'"' >> /etc/portage/make.conf && \
    echo 'USE="-X -gtk -qt5 -wayland"' >> /etc/portage/make.conf && \
    echo 'ACCEPT_KEYWORDS="amd64"' >> /etc/portage/make.conf

# Sync portage tree (emerge-webrsync is faster than emerge --sync in Docker)
RUN emerge-webrsync

# ── Emerge build + runtime dependencies ──
# rust-bin: binary package avoids 30+ min source bootstrap
# nodejs: dashboard build (npm ci + npm run build, in-stage per AC4)
# gcc + glibc: C compiler + headers for rusqlite bundled SQLite
# pkgconf: pkg-config replacement (Gentoo default)
# jq: required by skill handler scripts
# sqlite: runtime dep for rusqlite
# wget: non-portage binary downloads (gh, gws, ollama)
# ca-certificates: TLS root certs
RUN emerge --noreplace \
    dev-lang/rust-bin \
    net-libs/nodejs \
    sys-devel/gcc \
    dev-util/pkgconf \
    app-misc/jq \
    dev-db/sqlite \
    net-misc/wget \
    app-misc/ca-certificates

WORKDIR /app

# ── Dashboard build (in-stage, handbook-style — no external node image) ──
COPY package.json package-lock.json ./
COPY packages/ packages/
COPY dashboard/ dashboard/
RUN npm ci --ignore-scripts && npm run build --prefix dashboard

# ── Mika compilation ──
COPY Cargo.toml Cargo.lock ./
COPY crates/mika-common/ crates/mika-common/
COPY crates/mika-a2a/ crates/mika-a2a/
COPY crates/mika-agent/ crates/mika-agent/
COPY crates/mika-gateway/ crates/mika-gateway/
COPY crates/mika-cli/ crates/mika-cli/

# Docs and skills are embedded at build time via build.rs include_str!
COPY docs/ docs/
COPY skills/ skills/

# Build all three binaries with telemetry support
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/root/.cargo/registry \
    cargo build --release --features telemetry \
        --bin mika -p mika-cli \
        --bin mika-server -p mika-agent \
        --bin mika-gateway -p mika-gateway \
    && install -m 755 target/release/mika /usr/local/bin/mika \
    && install -m 755 target/release/mika-server /usr/local/bin/mika-server \
    && install -m 755 target/release/mika-gateway /usr/local/bin/mika-gateway

# ── Non-portage binaries (not available in Gentoo portage, sha256-verified) ──

# GitHub CLI
ARG GH_VERSION=2.65.0
RUN ARCH=$(uname -m) && \
    case "$ARCH" in \
        x86_64) GH_ARCH="amd64" ;; \
        aarch64) GH_ARCH="arm64" ;; \
        *) echo "Unsupported architecture: $ARCH" && exit 1 ;; \
    esac && \
    wget -qO "/tmp/gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz" \
        "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz" && \
    wget -qO /tmp/gh_checksums.txt \
        "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_checksums.txt" && \
    cd /tmp && grep "gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz" gh_checksums.txt | sha256sum -c - && \
    tar -xzf "/tmp/gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz" -C /tmp && \
    install -m 755 "/tmp/gh_${GH_VERSION}_linux_${GH_ARCH}/bin/gh" /usr/local/bin/gh && \
    rm -rf /tmp/gh*

# Google Workspace CLI
ARG GWS_VERSION=0.13.3
RUN ARCH=$(uname -m) && \
    case "$ARCH" in \
        x86_64) GWS_ARCH="x86_64-unknown-linux-gnu" ;; \
        aarch64) GWS_ARCH="aarch64-unknown-linux-gnu" ;; \
        *) echo "Unsupported architecture: $ARCH" && exit 1 ;; \
    esac && \
    wget -qO "/tmp/gws-${GWS_ARCH}.tar.gz" \
        "https://github.com/googleworkspace/cli/releases/download/v${GWS_VERSION}/gws-${GWS_ARCH}.tar.gz" && \
    wget -qO "/tmp/gws-${GWS_ARCH}.tar.gz.sha256" \
        "https://github.com/googleworkspace/cli/releases/download/v${GWS_VERSION}/gws-${GWS_ARCH}.tar.gz.sha256" && \
    cd /tmp && grep "gws-${GWS_ARCH}.tar.gz" "gws-${GWS_ARCH}.tar.gz.sha256" | sha256sum -c - && \
    tar -xzf "/tmp/gws-${GWS_ARCH}.tar.gz" -C /tmp && \
    find /tmp -name gws -type f -executable -exec install -m 755 {} /usr/local/bin/gws \; && \
    rm -rf /tmp/gws*

# Ollama (sha256 verified — AC7)
ARG OLLAMA_VERSION=0.6.0
RUN ARCH=$(uname -m) && \
    case "$ARCH" in \
        x86_64) OLLAMA_ARCH="amd64" ;; \
        aarch64) OLLAMA_ARCH="arm64" ;; \
        *) echo "Unsupported architecture: $ARCH" && exit 1 ;; \
    esac && \
    wget -qO "/tmp/ollama-linux-${OLLAMA_ARCH}" \
        "https://github.com/ollama/ollama/releases/download/v${OLLAMA_VERSION}/ollama-linux-${OLLAMA_ARCH}" && \
    wget -qO "/tmp/ollama-linux-${OLLAMA_ARCH}.sha256" \
        "https://github.com/ollama/ollama/releases/download/v${OLLAMA_VERSION}/ollama-linux-${OLLAMA_ARCH}.sha256" && \
    cd /tmp && \
    EXPECTED=$(cat "ollama-linux-${OLLAMA_ARCH}.sha256" | awk '{print $1}') && \
    echo "${EXPECTED}  ollama-linux-${OLLAMA_ARCH}" | sha256sum -c - && \
    install -m 755 "ollama-linux-${OLLAMA_ARCH}" /usr/local/bin/ollama && \
    rm -f /tmp/ollama-linux-*

# ── Create non-root user ──
RUN useradd -m -u 1000 -s /bin/bash mika

# ── OpenRC service definitions ──
COPY os/openrc/init.d/mika-server /etc/init.d/mika-server
COPY os/openrc/init.d/mika-gateway /etc/init.d/mika-gateway
COPY os/openrc/conf.d/mika-server /etc/conf.d/mika-server
COPY os/openrc/conf.d/mika-gateway /etc/conf.d/mika-gateway
RUN chmod 755 /etc/init.d/mika-server /etc/init.d/mika-gateway

# Register services with default runlevel
RUN rc-update add mika-server default && \
    rc-update add mika-gateway default

# ── Default configs at /etc/mika/ (FHS-compliant, AC5) ──
RUN mkdir -p /etc/mika
COPY os/config/config.toml /etc/mika/config.toml
COPY os/config/mika.env /etc/mika/mika.env.template

# ── Copy docs for KG ingestion at runtime ──
RUN mkdir -p /app/docs
COPY docs/ /app/docs/

# ── Container entrypoint ──
COPY os/init/mika-os-init.sh /usr/local/bin/mika-os-init.sh
RUN chmod 755 /usr/local/bin/mika-os-init.sh

# ── Portage cleanup (keep portage tree for developer use) ──
# NOTE: Do NOT run `emerge --depclean` here — portage has no visibility into
# the dynamically-linked Rust binaries installed to /usr/local/bin/ and would
# remove gcc runtime libs (libgcc_s.so.1) needed at runtime.
RUN rm -rf /var/tmp/portage /var/cache/distfiles

ENV MIKA_HOME=/home/mika/.mika

EXPOSE 8080 3001

HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=3 \
    CMD rc-service mika-server status

ENTRYPOINT ["/usr/local/bin/mika-os-init.sh"]

# ============================================================================
# Stage 2: mika-runtime — Production runtime (no compilers, no portage tree)
# ============================================================================
FROM gentoo/stage3:20250505 AS mika-runtime

LABEL mika.image-type="runtime"

# ── Minimal portage setup for runtime deps only ──
RUN emerge-webrsync && \
    emerge --noreplace \
        app-misc/jq \
        dev-db/sqlite \
        app-misc/ca-certificates && \
    rm -rf /var/tmp/portage /var/cache/distfiles /var/db/repos/gentoo

# ── Create non-root user (no login shell — hardened for production) ──
RUN useradd -m -u 1000 -s /bin/false mika

# ── Copy binaries from mika-os ──
COPY --from=mika-os /usr/local/bin/mika /usr/local/bin/mika
COPY --from=mika-os /usr/local/bin/mika-server /usr/local/bin/mika-server
COPY --from=mika-os /usr/local/bin/mika-gateway /usr/local/bin/mika-gateway
COPY --from=mika-os /usr/local/bin/gh /usr/local/bin/gh
COPY --from=mika-os /usr/local/bin/gws /usr/local/bin/gws
COPY --from=mika-os /usr/local/bin/ollama /usr/local/bin/ollama

# ── Copy OpenRC service definitions ──
COPY --from=mika-os /etc/init.d/mika-server /etc/init.d/mika-server
COPY --from=mika-os /etc/init.d/mika-gateway /etc/init.d/mika-gateway
COPY --from=mika-os /etc/conf.d/mika-server /etc/conf.d/mika-server
COPY --from=mika-os /etc/conf.d/mika-gateway /etc/conf.d/mika-gateway

# Register services with default runlevel
RUN rc-update add mika-server default && \
    rc-update add mika-gateway default

# ── Copy default configs (FHS-compliant) ──
COPY --from=mika-os /etc/mika/ /etc/mika/

# ── Copy docs for KG ingestion ──
COPY --from=mika-os /app/docs/ /app/docs/

# ── Copy entrypoint ──
COPY --from=mika-os /usr/local/bin/mika-os-init.sh /usr/local/bin/mika-os-init.sh

ENV MIKA_HOME=/home/mika/.mika

WORKDIR /app
EXPOSE 8080 3001

HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=3 \
    CMD rc-service mika-server status

ENTRYPOINT ["/usr/local/bin/mika-os-init.sh"]
