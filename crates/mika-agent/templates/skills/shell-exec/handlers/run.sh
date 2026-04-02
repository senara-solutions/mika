#!/bin/sh
# Shell execution handler.
# Input: JSON on stdin with "command" and optional "working_dir" fields
# Output: command output on stdout, errors on stderr
#
# SECURITY: This handler executes arbitrary commands. Use responsibly.

command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }

INPUT=$(cat)

# Scrub all MIKA_* env vars so subprocesses cannot leak secrets
# Mirrors the Rust executor's scrub_mika_env_vars() wildcard approach
for _mika_var in $(env | grep '^MIKA_' | cut -d= -f1); do unset "$_mika_var"; done
# Scrub GH_TOKEN to prevent identity collision if leaked from .env (#380)
unset GH_TOKEN 2>/dev/null

# Parse JSON fields
COMMAND=$(printf '%s\n' "$INPUT" | jq -r '.command // empty')
WORKDIR=$(printf '%s\n' "$INPUT" | jq -r '.working_dir // empty')

if [ -z "$COMMAND" ]; then
    echo "Error: no command provided" >&2
    exit 1
fi

# Block commands that have dedicated skill handlers (security: force use of controlled wrappers)
FIRST_WORD=$(printf '%s\n' "$COMMAND" | awk '{print $1}')
case "$FIRST_WORD" in
    gws)  echo "Error: Use the dedicated run_gws skill instead of run_shell for security." >&2; exit 1 ;;
    gh)   echo "Error: Use the dedicated run_gh skill instead of run_shell for security." >&2; exit 1 ;;
esac

if [ -n "$WORKDIR" ] && [ -d "$WORKDIR" ]; then
    cd "$WORKDIR" || exit 1
fi

eval "$COMMAND" 2>&1
