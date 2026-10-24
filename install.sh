#!/bin/sh
# Install Mika CLI — AI Executive Assistant
# Usage: curl -fsSL https://raw.githubusercontent.com/senara-solutions/mika/main/install.sh | sh
# Pin a version: curl -fsSL ... | sh -s -- v0.2.0
#
# Options:
#   --allow-major-upgrade  Install even when the target version changes the major
#                          number of the version already in INSTALL_DIR. From
#                          v1.0 Mika follows strict semver, so a major change may
#                          need a migration step: without this option the install
#                          stops before replacing anything and prints the
#                          migration guide of the target version (mika#2897).
#   -h, --help             Print the usage above and exit.
#
# Environment:
#   MIKA_INSTALL_DIR  Where the binary lands (default: $HOME/.local/bin).
set -eu

REPO="senara-solutions/mika"
BINARY="mika"
INSTALL_DIR="${MIKA_INSTALL_DIR:-$HOME/.local/bin}"

# Printed by -h/--help. Plain `echo` rather than a here-document: this script is
# consumed by `curl … | sh`, where stdin *is* the script.
usage() {
    echo "Usage: install.sh [--allow-major-upgrade] [VERSION]"
    echo ""
    echo "Installs the Mika CLI into \${MIKA_INSTALL_DIR:-\$HOME/.local/bin}."
    echo "VERSION is a semver tag such as v1.2.3; the latest release is used when omitted."
    echo ""
    echo "  --allow-major-upgrade  Install even when the target version changes the major"
    echo "                         number of the version already installed. From v1.0 Mika"
    echo "                         follows strict semver, so a major change may need a"
    echo "                         migration step: without this option the install stops"
    echo "                         and prints the migration guide of the target version."
    echo "  -h, --help             Print this message and exit."
}

# Accept an optional version argument and the options above, in any order: the
# README's form is `… | sh -s -- v2.0.0`, and whoever adds the flag will write it
# as readily before the version as after. An unknown `-*` is refused here rather
# than carried to the semver validator, which would blame the version format for
# a typo in the flag name.
ALLOW_MAJOR=0
VERSION=""
while [ $# -gt 0 ]; do
    case "$1" in
        --allow-major-upgrade) ALLOW_MAJOR=1 ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "Error: unknown option '$1'" >&2; usage >&2; exit 2 ;;
        *)
            if [ -n "$VERSION" ]; then
                echo "Error: version given twice ('$VERSION' then '$1')" >&2
                exit 2
            fi
            VERSION="$1"
            ;;
    esac
    shift
done

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

# Validate VERSION format (semver)
if [ -n "$VERSION" ]; then
  case "$VERSION" in
    *[!0-9a-zA-Z._-]*) echo "Error: invalid version format: $VERSION" >&2; exit 1 ;;
  esac
  if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
    # Also accept "v" prefix: v1.2.3
    if ! echo "$VERSION" | grep -qE '^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
      echo "Error: invalid version format: $VERSION (expected semver like 1.2.3 or v1.2.3)" >&2
      exit 1
    fi
  fi
fi

# Stop before a major change, and point at the migration guide (mika#2897).
#
# From v1.0 Mika follows strict semver: a breaking change ships as a major
# version with a migration guide. Replacing the binary across a major change
# without a word is how someone finds out about the migration when their
# ~/.mika stops opening.
#
# WHY HERE. After the semver validation, because comparing majors on an
# unvalidated string means writing a second validator. After the latest-release
# lookup, because the guard must judge the version actually being installed —
# on the README's path (`curl … | sh`, no argument) that version does not exist
# any earlier. And before URL, because this is the last point at which no byte
# of an archive has been asked for: the ticket asks for a stop before the
# replacement, and stopping before the download is strictly stronger.
#
# THE COMPARISON IS SYMMETRIC. 2.1.0 → v1.0.0 changes the major too, and a
# ~/.mika migrated by a 2.x is not readable by a 1.x. Hence "major change", not
# "upgrade", everywhere but the option's name.
mika_major_of() {
    # vX.Y.Z[-pre] or X.Y.Z[-pre] → X. A pre-release suffix is no bother: the
    # major of v2.0.0-rc.1 is read well before the dash.
    mika_major_v="${1#v}"
    echo "${mika_major_v%%.*}"
}

if [ "${ALLOW_MAJOR}" -eq 0 ] && [ -e "${INSTALL_DIR}/${BINARY}" ]; then
    INSTALLED_VERSION=""
    if [ -x "${INSTALL_DIR}/${BINARY}" ]; then
        # `|| true`: under `set -eu` a failing --version would kill the script
        # with a status nobody can interpret. `mika --version` prints
        # `mika X.Y.Z (sha)`, so the version is the second field.
        INSTALLED_VERSION=$("${INSTALL_DIR}/${BINARY}" --version 2>/dev/null | awk '{print $2}') || true
    fi
    # A field that does not validate is an unreadable version, which is what
    # makes the parsing safe without depending on the shape of the parenthesised
    # suffix staying put.
    if ! echo "${INSTALLED_VERSION}" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+'; then
        # Fail open, deliberately. Refusing when the version cannot be read
        # would block reinstalling a broken binary — truncated, built for the
        # wrong architecture, half-overwritten by an interrupted mv — and
        # reinstalling *is* the repair. Falling back to today's behaviour is the
        # worst case already known and accepted; the warning is what keeps it
        # from being silent.
        echo "Warning: could not read the version installed at ${INSTALL_DIR}/${BINARY}; skipping the major-change check." >&2
    else
        INSTALLED_MAJOR=$(mika_major_of "${INSTALLED_VERSION}")
        TARGET_MAJOR=$(mika_major_of "${VERSION}")
        if [ "${INSTALLED_MAJOR}" != "${TARGET_MAJOR}" ]; then
            # The guide is anchored on the target tag, never on main: a link on
            # main describes today's version, not the one being migrated to.
            # release.yml refuses to prepare a tag whose docs/releases/vX.Y.Z.md
            # is missing, so this document exists for every version this script
            # can install. The `v` is normalised here and nowhere else — ARCHIVE
            # and URL keep the string as given.
            case "${VERSION}" in
                v*) TARGET_TAG="${VERSION}" ;;
                *)  TARGET_TAG="v${VERSION}" ;;
            esac
            echo "Error: this is a major change, and Mika follows strict semver from v1.0." >&2
            echo "  installed: ${INSTALLED_VERSION} (${INSTALL_DIR}/${BINARY})" >&2
            echo "  target:    ${VERSION}" >&2
            echo "" >&2
            echo "A major change may need a migration step. Read the migration guide first:" >&2
            echo "  https://github.com/${REPO}/blob/${TARGET_TAG}/docs/releases/${TARGET_TAG}.md" >&2
            echo "" >&2
            echo "Then install ${VERSION} explicitly:" >&2
            echo "  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh -s -- --allow-major-upgrade ${VERSION}" >&2
            echo "" >&2
            echo "Nothing was changed; ${INSTALL_DIR}/${BINARY} is still ${INSTALLED_VERSION}." >&2
            exit 1
        fi
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
