#!/bin/sh
# GitHub CLI execution handler.
# Input: JSON on stdin with "command" and optional "repo" fields
# Output: gh command output on stdout, errors on stderr
#
# SECURITY: Does not use eval. Arguments are passed directly to gh.
# Only allowlisted top-level subcommands are permitted.

set -f  # Disable globbing for safe word splitting

# Disable interactive prompts (prevents hangs in piped mode)
export GH_PROMPT_DISABLED=1

# Scrub sensitive env vars so gh subprocesses cannot leak them
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY MIKA_INVESTIGATE_GITHUB_TOKEN

INPUT=$(cat)
COMMAND=$(printf '%s\n' "$INPUT" | jq -r '.command // empty')
REPO=$(printf '%s\n' "$INPUT" | jq -r '.repo // empty')

if [ -z "$COMMAND" ]; then
    echo "Error: no command provided" >&2
    exit 1
fi

# Validate gh is installed
if ! command -v gh >/dev/null 2>&1; then
    echo "Error: gh CLI is not installed. Install it from https://cli.github.com/" >&2
    exit 1
fi

# Allowlist of permitted top-level subcommands
SUBCOMMAND=$(printf '%s\n' "$COMMAND" | awk '{print $1}')
case "$SUBCOMMAND" in
    pr|issue|run|workflow|release|repo|search|label|milestone|project)
        ;;
    *)
        echo "Error: gh subcommand '$SUBCOMMAND' is not allowed. Permitted: pr, issue, run, workflow, release, repo, search, label, milestone, project." >&2
        exit 1
        ;;
esac

# Execute gh with the provided arguments (no eval — prevents shell injection)
# shellcheck disable=SC2086
if [ -n "$REPO" ]; then
    gh $COMMAND --repo "$REPO" 2>&1
else
    gh $COMMAND 2>&1
fi
