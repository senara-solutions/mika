# SPDX-License-Identifier: Apache-2.0
# PRE-AUDIT PLACEHOLDER
#
# Mika OS — Complete runtime image (Gentoo + OpenRC + mika binaries + ollama)
#
# Three audiences:
#   1. Per-customer container base for mika-cloud Helm deployments
#   2. Single-box self-host (docker run mika-os)
#   3. Forkable developer reference environment
#
# Build:   docker build -t mika-os:dev -f os/Dockerfile .
# Run:     docker run --cap-add=SYS_ADMIN --security-opt apparmor=unconfined \
#            -v mika-data:/home/mika/.mika mika-os:dev
#
# The final production recipe will be derived from an audited Gentoo configuration.

# === Dashboard build stage ===
FROM node:24-slim AS dashboard-builder
WORKDIR /app
COPY package.json package-lock.json ./
COPY packages/ packages/
COPY dashboard/ dashboard/
RUN npm ci --ignore-scripts && npm run build --prefix dashboard

# === Rust builder stage ===
FROM rust:1.93-slim AS builder

# Install C compiler for rusqlite bundled SQLite (no OpenSSL needed — uses rustls)
RUN apt-get update && apt-get install -y --no-install-recommends gcc libc-dev pkg-config && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace manifests, lockfile, and all crate source
COPY Cargo.toml Cargo.lock ./
COPY crates/mika-common/ crates/mika-common/
COPY crates/mika-a2a/ crates/mika-a2a/
COPY crates/mika-agent/ crates/mika-agent/
COPY crates/mika-gateway/ crates/mika-gateway/
COPY crates/mika-cli/ crates/mika-cli/

# Copy docs (embedded at build time via build.rs include_str!)
COPY docs/ docs/

# Copy skills (bundled skills discovered at build time via build.rs)
COPY skills/ skills/

# Copy dashboard dist for embedding
COPY --from=dashboard-builder /app/dashboard/dist dashboard/dist

# Build all three binaries with BuildKit cache mounts
# --features telemetry enables OTLP trace export
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release --features telemetry \
        --bin mika -p mika-cli \
        --bin mika-server -p mika-agent \
        --bin mika-gateway -p mika-gateway \
    && cp target/release/mika /usr/local/bin/mika \
    && cp target/release/mika-server /usr/local/bin/mika-server \
    && cp target/release/mika-gateway /usr/local/bin/mika-gateway

# === Runtime stage (Gentoo + OpenRC) ===
FROM gentoo/stage3:20250505

LABEL mika.audit-status="pre-audit"

# Install runtime dependencies
RUN emerge --noreplace app-misc/jq net-misc/wget dev-db/sqlite && \
    rm -rf /var/tmp/portage /var/cache/distfiles

# Install GitHub CLI (same pattern as Dockerfile.agent)
RUN ARCH=$(uname -m) && \
    case "$ARCH" in \
        x86_64) GH_ARCH="amd64" ;; \
        aarch64) GH_ARCH="arm64" ;; \
        *) echo "Unsupported architecture: $ARCH" && exit 1 ;; \
    esac && \
    GH_VERSION="2.65.0" && \
    wget -qO /tmp/gh.tar.gz "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz" && \
    wget -qO /tmp/gh_checksums.txt "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_checksums.txt" && \
    cd /tmp && grep "gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz" gh_checksums.txt | sha256sum -c - && \
    tar -xzf /tmp/gh.tar.gz -C /tmp && \
    mv /tmp/gh_${GH_VERSION}_linux_${GH_ARCH}/bin/gh /usr/local/bin/gh && \
    rm -rf /tmp/gh*

# Install ollama v0.6.0 (binary only — no models pulled at build time)
RUN OLLAMA_VERSION="v0.6.0" && \
    ARCH=$(uname -m) && \
    case "$ARCH" in \
        x86_64) OLLAMA_ARCH="amd64" ;; \
        aarch64) OLLAMA_ARCH="arm64" ;; \
        *) echo "Unsupported architecture: $ARCH" && exit 1 ;; \
    esac && \
    wget -qO /usr/local/bin/ollama "https://github.com/ollama/ollama/releases/download/${OLLAMA_VERSION}/ollama-linux-${OLLAMA_ARCH}" && \
    chmod 755 /usr/local/bin/ollama

# Create non-root user with home directory
RUN useradd -m -u 1000 -s /bin/bash mika

# Copy binaries from builder
COPY --from=builder /usr/local/bin/mika /usr/local/bin/mika
COPY --from=builder /usr/local/bin/mika-server /usr/local/bin/mika-server
COPY --from=builder /usr/local/bin/mika-gateway /usr/local/bin/mika-gateway

# Copy docs into container for KG ingestion
COPY docs/ /app/docs/

# Copy OpenRC service definitions
COPY os/openrc/init.d/mika-server /etc/init.d/mika-server
COPY os/openrc/init.d/mika-gateway /etc/init.d/mika-gateway
COPY os/openrc/conf.d/mika-server /etc/conf.d/mika-server
COPY os/openrc/conf.d/mika-gateway /etc/conf.d/mika-gateway
RUN chmod 755 /etc/init.d/mika-server /etc/init.d/mika-gateway

# Register services with default runlevel
RUN rc-update add mika-server default && \
    rc-update add mika-gateway default

# Copy default config files
COPY os/config/config.toml /home/mika/.mika/config.toml
COPY os/config/mika.env /home/mika/.mika/.env.template
RUN chown -R mika:mika /home/mika/.mika

# Copy entrypoint script
COPY os/init/mika-os-init.sh /usr/local/bin/mika-os-init.sh
RUN chmod 755 /usr/local/bin/mika-os-init.sh

# Set Mika home
ENV MIKA_HOME=/home/mika/.mika

WORKDIR /app
EXPOSE 8080 3001

HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=3 \
    CMD rc-service mika-server status

ENTRYPOINT ["/usr/local/bin/mika-os-init.sh"]
