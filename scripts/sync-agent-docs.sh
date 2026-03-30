#!/usr/bin/env bash
# Sync workspace-root docs to crate-local fallback copies for crates.io publishing.
# Run before `cargo publish -p mika-agent`.
# Keep DOCS list in sync with crates/mika-agent/build.rs DOCS constant.
# CI enforces this via the docs-sync job in .github/workflows/ci.yml.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$SCRIPT_DIR/.."
SRC="$ROOT/docs"
DST="$ROOT/crates/mika-agent/docs"

DOCS=(
    architecture.md
    browser-control.md
    configuration.md
    deployment.md
    getting-started.md
    runtime-structure.md
    skills.md
    slash-commands.md
    task-system.md
)

for doc in "${DOCS[@]}"; do
    cp "$SRC/$doc" "$DST/$doc"
    echo "  copied $doc"
done

mkdir -p "$DST/openapi"
cp "$SRC/openapi/mika-server.yaml" "$DST/openapi/mika-server.yaml"
echo "  copied openapi/mika-server.yaml"

echo "Done. Crate-local docs are in sync."
