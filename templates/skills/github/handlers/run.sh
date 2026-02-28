#!/bin/sh
# GitHub CLI execution handler.
# Input: JSON on stdin with "command" and optional "repo" fields
# Output: gh command output on stdout, errors on stderr
#
# SECURITY: Does not use eval. Arguments are passed directly to gh.
# Credential-leaking subcommands are blocked.

set -f  # Disable globbing for safe word splitting

# Disable interactive prompts (prevents hangs in piped mode)
export GH_PROMPT_DISABLED=1

INPUT=$(cat)
COMMAND=$(printf '%s\n' "$INPUT" | jq -r '.command // empty' 2>/dev/null)
REPO=$(printf '%s\n' "$INPUT" | jq -r '.repo // empty' 2>/dev/null)

# Fallback if jq is not available
if [ -z "$COMMAND" ]; then
    COMMAND=$(printf '%s\n' "$INPUT" | grep -o '"command":"[^"]*"' | head -1 | cut -d'"' -f4)
    REPO=$(printf '%s\n' "$INPUT" | grep -o '"repo":"[^"]*"' | head -1 | cut -d'"' -f4)
fi

if [ -z "$COMMAND" ]; then
    echo "Error: no command provided" >&2
    exit 1
fi

# Validate gh is installed
if ! command -v gh >/dev/null 2>&1; then
    echo "Error: gh CLI is not installed. Install it from https://cli.github.com/" >&2
    exit 1
fi

# Block credential-leaking commands
case "$COMMAND" in
    auth\ token*|auth\ refresh*|auth\ setup-git*)
        echo "Error: credential management commands are blocked for security." >&2
        exit 1
        ;;
esac

# Validate gh is authenticated
if ! gh auth status >/dev/null 2>&1; then
    echo "Error: gh is not authenticated. Run 'gh auth login' or set the GH_TOKEN environment variable." >&2
    exit 1
fi

# Execute gh with the provided arguments (no eval — prevents shell injection)
# shellcheck disable=SC2086
if [ -n "$REPO" ]; then
    gh $COMMAND --repo "$REPO" 2>&1
else
    gh $COMMAND 2>&1
fi
