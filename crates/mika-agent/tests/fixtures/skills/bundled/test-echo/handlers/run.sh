#!/bin/sh
# Test-only fixture handler. Never executed in production.
# Uses /bin/sh to match the convention in crates/mika-agent/templates/skills/*/handlers/.
echo "test-echo: $*"
