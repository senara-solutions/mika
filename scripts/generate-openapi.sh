#!/usr/bin/env bash
# Generate OpenAPI YAML specs from utoipa annotations.
#
# Runs the spec-generation tests that write YAML files to docs/openapi/.
# The committed YAML files are the source of truth for include_str! in builtin_handlers.rs.
#
# Usage: ./scripts/generate-openapi.sh

set -euo pipefail
cd "$(dirname "$0")/.."

echo "Generating mika-server OpenAPI spec..."
cargo test -p mika-agent write_agent_openapi_yaml -- --ignored
echo "  → docs/openapi/mika-server.yaml"

echo "Done. Verify with: cargo build"
