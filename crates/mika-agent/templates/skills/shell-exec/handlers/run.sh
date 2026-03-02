#!/bin/sh
# Shell execution handler.
# Input: JSON on stdin with "command" and optional "working_dir" fields
# Output: command output on stdout, errors on stderr
#
# SECURITY: This handler executes arbitrary commands. Use responsibly.

INPUT=$(cat)

# Parse JSON fields (jq preferred, grep/cut fallback)
if command -v jq >/dev/null 2>&1; then
    COMMAND=$(printf '%s\n' "$INPUT" | jq -r '.command // empty')
    WORKDIR=$(printf '%s\n' "$INPUT" | jq -r '.working_dir // empty')
else
    # Fallback: grep-based extraction (cannot handle embedded quotes)
    COMMAND=$(printf '%s\n' "$INPUT" | grep -o '"command":"[^"]*"' | head -1 | cut -d'"' -f4)
    WORKDIR=$(printf '%s\n' "$INPUT" | grep -o '"working_dir":"[^"]*"' | head -1 | cut -d'"' -f4)
fi

if [ -z "$COMMAND" ]; then
    echo "Error: no command provided" >&2
    exit 1
fi

if [ -n "$WORKDIR" ] && [ -d "$WORKDIR" ]; then
    cd "$WORKDIR" || exit 1
fi

eval "$COMMAND" 2>&1
