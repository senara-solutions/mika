#!/bin/sh
# Install Mika CLI — AI Executive Assistant
# Usage: curl -fsSL https://raw.githubusercontent.com/senara-solutions/mika/main/install.sh | sh
# Pin a version: curl -fsSL ... | sh -s -- v0.2.0
set -eu

REPO="senara-solutions/mika"
BINARY="mika"
INSTALL_DIR="${MIKA_INSTALL_DIR:-$HOME/.local/bin}"

# Accept optional version argument
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
