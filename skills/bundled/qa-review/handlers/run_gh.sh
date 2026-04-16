#!/usr/bin/env bash
set -euo pipefail

# run_gh — restricted gh CLI wrapper for qa-review.
#
# Allowlist: pr review, pr diff, pr list, issue view
# Blocked: pr view (use qa_pr_view), pr checks (CI out of scope),
#          api (no direct API calls), everything else
#
# Input: JSON on stdin with "command" (array) and optional "repo" (string)
# Output: gh CLI output on stdout
#
# See: #115, feedback_prompt_enforcement_fragile

command -v gh &>/dev/null || { echo "ERROR: gh CLI not found on PATH" >&2; exit 1; }
command -v jq &>/dev/null || { echo "ERROR: jq not found on PATH" >&2; exit 1; }

INPUT=$(cat)

# Parse command array and repo from JSON input
CMD_LENGTH=$(printf '%s\n' "$INPUT" | jq -r '.command | length')
if [[ "$CMD_LENGTH" == "null" ]] || [[ "$CMD_LENGTH" -lt 1 ]]; then
  echo "ERROR: command array is required and must not be empty" >&2
  exit 1
fi

# Extract first two tokens as the command prefix
CMD_FIRST=$(printf '%s\n' "$INPUT" | jq -r '.command[0] // empty')
CMD_SECOND=$(printf '%s\n' "$INPUT" | jq -r '.command[1] // empty')
CMD_PREFIX="${CMD_FIRST} ${CMD_SECOND}"

# Check against allowlist
case "$CMD_PREFIX" in
  "pr review")
    ;; # allowed — verdict posting
  "pr diff")
    ;; # allowed — diff reading
  "pr list")
    ;; # allowed — cross-repo companion PR search
  "issue view")
    ;; # allowed — reading linked issues
  "pr view")
    echo "ERROR: Use qa_pr_view for PR metadata. run_gh is restricted to: pr review, pr diff, pr list, issue view" >&2
    exit 1
    ;;
  "pr checks")
    echo "ERROR: CI status is not in scope for QA review per system prompt line 44. run_gh is restricted to: pr review, pr diff, pr list, issue view" >&2
    exit 1
    ;;
  "api "*)
    echo "ERROR: Direct API calls not permitted. run_gh is restricted to: pr review, pr diff, pr list, issue view" >&2
    exit 1
    ;;
  *)
    echo "ERROR: Command '${CMD_PREFIX}' not in qa-review allowlist. run_gh is restricted to: pr review, pr diff, pr list, issue view" >&2
    exit 1
    ;;
esac

# Build the gh command from the JSON command array
REPO=$(printf '%s\n' "$INPUT" | jq -r '.repo // empty')

# Reconstruct command args as a bash array (safe: uses NUL-delimited jq output)
CMD_ARGS=()
while IFS= read -r -d '' arg; do
  CMD_ARGS+=("$arg")
done < <(printf '%s\n' "$INPUT" | jq -j '.command[] | . + "\u0000"')

if [[ -n "$REPO" ]]; then
  gh "${CMD_ARGS[@]}" --repo "$REPO"
else
  gh "${CMD_ARGS[@]}"
fi
