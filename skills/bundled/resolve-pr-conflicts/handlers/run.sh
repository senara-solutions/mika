#!/bin/sh
# Handler for the resolve-pr-conflicts skill (long-running exec).
# Input: JSON on stdin with task_id and pr_url (preferred) or worktree_path (deprecated).
#        __mika_task_id and __mika_agent are injected by the executor.
# Output: Delivers result via `mika ask --task-id` callback when done.
#
# Spawns claude-pilot with a focused conflict-resolution prompt (no --command).
# When pr_url is provided, derives the worktree path from the PR's branch name
# using the canonical derive-worktree-path script. Falls back to explicit worktree_path.
# The worktree must already exist — this handler does NOT create worktrees.

set -e

# Ensure ~/.local/bin is in PATH (mika CLI needed for callback delivery)
export PATH="$HOME/.local/bin:$PATH"

# Dependency checks
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }
command -v claude-pilot >/dev/null 2>&1 || { echo "Error: claude-pilot CLI is required but not in PATH" >&2; exit 1; }

# Read input JSON from stdin
INPUT=$(cat)

# Parse callback fields injected by the long-running executor
TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.__mika_task_id // empty')
AGENT=$(printf '%s\n' "$INPUT" | jq -r '.__mika_agent // empty')

if [ -z "$TASK_ID" ]; then
    echo "Error: no __mika_task_id in input (not running as long-running handler?)" >&2
    exit 1
fi

# --- Crash-recovery EXIT trap ---
# Ensures callback delivery on any exit (crash, set -e, signals).
# Uses CALLBACK_SENT guard to prevent double delivery.
CALLBACK_SENT=0

deliver_callback() {
    _EXIT_CODE=$?
    [ "$CALLBACK_SENT" -eq 1 ] && { [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"; return; }
    [ -z "$TASK_ID" ] && { [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"; return; }
    # Capture stderr tail on crash path BEFORE deleting the file (#104)
    if [ -z "$RESULT" ] && [ -n "$STDERR_FILE" ] && [ -f "$STDERR_FILE" ]; then
        _STDERR_TAIL=$(tail -c 10000 "$STDERR_FILE" 2>/dev/null)
        if [ -n "$_STDERR_TAIL" ]; then
            RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result.

Stderr (last 10KB):
${_STDERR_TAIL}"
        fi
    fi
    # Clean up stderr temp file AFTER capture
    [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"
    if [ -z "$RESULT" ]; then
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result."
    fi
    RESULT=$(printf '%s' "$RESULT" | head -c 92000)
    set +e
    if [ -n "$AGENT" ]; then
        mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
    else
        mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
    fi
    CALLBACK_SENT=1
    set -e
}
trap deliver_callback EXIT

# Parse user-provided fields
WORKTREE_PATH=$(printf '%s\n' "$INPUT" | jq -r '.worktree_path // empty')
USER_TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.task_id // empty')
PR_URL=$(printf '%s\n' "$INPUT" | jq -r '.pr_url // empty')

# mika-platform root — base for relay config resolution
# Resolve symlinks so prefix checks work regardless of which path the caller uses
PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
PLATFORM_DIR=$(cd "$PLATFORM_DIR" 2>/dev/null && pwd -P) || PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"

if [ -z "$USER_TASK_ID" ]; then
    echo "Error: task_id is required" >&2
    exit 1
fi

# Derive worktree path from PR URL using canonical branch-to-path sanitization
derive_worktree_from_pr() {
    # Parse repo and PR number from URL
    # Pattern: https://github.com/{owner}/{repo}/pull/{number}
    PR_NUMBER=$(printf '%s' "$PR_URL" | sed -n 's|.*/pull/\([0-9]*\).*|\1|p')
    REPO_FULL=$(printf '%s' "$PR_URL" | sed -n 's|https://github.com/\([^/]*/[^/]*\)/pull/.*|\1|p')

    if [ -z "$PR_NUMBER" ] || [ -z "$REPO_FULL" ]; then
        echo "Error: could not parse PR number and repo from pr_url '$PR_URL'" >&2
        return 1
    fi

    REPO_SHORT=$(basename "$REPO_FULL")

    # Get branch name from PR
    BRANCH=$(gh pr view "$PR_NUMBER" --repo "$REPO_FULL" --json headRefName -q .headRefName 2>/dev/null)
    if [ -z "$BRANCH" ]; then
        echo "Error: could not get branch name from PR $PR_NUMBER in $REPO_FULL" >&2
        return 1
    fi

    # Derive worktree path using canonical script
    DERIVED_PATH=$("$PLATFORM_DIR/scripts/derive-worktree-path" --branch "$BRANCH" --repo "$REPO_SHORT")
    if [ -z "$DERIVED_PATH" ]; then
        echo "Error: derive-worktree-path returned empty path for branch '$BRANCH' repo '$REPO_SHORT'" >&2
        return 1
    fi

    printf '%s' "$DERIVED_PATH"
}

# Two-tier worktree path resolution: pr_url (preferred) → worktree_path (deprecated fallback)
if [ -n "$PR_URL" ]; then
    # set +e so the subshell failure doesn't trigger set -e exit before we can handle it
    set +e
    DERIVED_PATH=$(derive_worktree_from_pr)
    DERIVE_EXIT=$?
    set -e
    if [ "$DERIVE_EXIT" -ne 0 ]; then
        # derive_worktree_from_pr already printed error to stderr
        exit 1
    fi

    # Mismatch validation (defense-in-depth)
    if [ -n "$WORKTREE_PATH" ]; then
        CANONICAL_DERIVED=$(cd "$DERIVED_PATH" 2>/dev/null && pwd -P) || CANONICAL_DERIVED="$DERIVED_PATH"
        CANONICAL_EXPLICIT=$(cd "$WORKTREE_PATH" 2>/dev/null && pwd -P) || CANONICAL_EXPLICIT="$WORKTREE_PATH"
        if [ "$CANONICAL_DERIVED" != "$CANONICAL_EXPLICIT" ]; then
            echo "WARN: derived worktree path ($DERIVED_PATH) != explicit worktree_path ($WORKTREE_PATH). Using derived path." >&2
        fi
    fi

    WORKTREE_PATH="$DERIVED_PATH"
elif [ -z "$WORKTREE_PATH" ]; then
    echo "Error: either pr_url or worktree_path is required" >&2
    exit 1
fi

# Validate worktree_path is under the expected worktree root (defense-in-depth)
# Canonicalize incoming path — callers may use either symlink or real path
CANONICAL_WORKTREE=$(cd "$WORKTREE_PATH" 2>/dev/null && pwd -P) || CANONICAL_WORKTREE="$WORKTREE_PATH"
EXPECTED_PREFIX="${PLATFORM_DIR}/.claude/worktrees/"
case "$CANONICAL_WORKTREE" in
    "$EXPECTED_PREFIX"*) ;; # OK — path is within the worktree directory
    *) echo "Error: worktree_path must be under $EXPECTED_PREFIX" >&2; exit 1 ;;
esac

# Validate worktree_path is a git working tree
if ! git -C "$WORKTREE_PATH" rev-parse --git-dir >/dev/null 2>&1; then
    echo "Error: worktree_path '$WORKTREE_PATH' is not a valid git working tree" >&2
    exit 1
fi

# Scrub sensitive env vars before spawning child processes
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY

# Copy relay config into worktree if missing (required for claude-pilot permissions)
# NOTE: Do NOT copy .claude/commands/ — Claude Code discovers commands from the
# repo's own .claude/commands/. See: docs/solutions/integration-issues/worktree-handler-architecture-fixes.md
mkdir -p "$WORKTREE_PATH/.claude"
cp "$PLATFORM_DIR/.claude/claude-pilot.json" "$WORKTREE_PATH/.claude/" 2>/dev/null || true
cp "$PLATFORM_DIR/.claude/settings.local.json" "$WORKTREE_PATH/.claude/" 2>/dev/null || true

# Build --cwd and --relay-config args
CWD_ARGS="--cwd $WORKTREE_PATH"
if [ -f "$WORKTREE_PATH/.claude/claude-pilot.json" ]; then
    CWD_ARGS="$CWD_ARGS --relay-config $WORKTREE_PATH/.claude/claude-pilot.json"
elif [ -f "$PLATFORM_DIR/.claude/claude-pilot.json" ]; then
    CWD_ARGS="$CWD_ARGS --relay-config $PLATFORM_DIR/.claude/claude-pilot.json"
fi

# Conflict resolution prompt — free-text mode (no --command to avoid /mika pipeline)
PROMPT="Resolve merge conflicts on this branch. Steps:
1. Run: git fetch origin
2. Detect the base branch: run gh pr list --head \$(git branch --show-current) --json baseRefName --jq '.[0].baseRefName' — if empty, default to main
3. Run: git rebase origin/<base_branch>
4. If conflicts arise, resolve each one: read both sides, choose the correct resolution, git add the file, git rebase --continue
5. After rebase completes, run the repo's test suite if one exists (check for Makefile, cargo, npm, etc.)
6. Push: git push --force-with-lease
7. If push fails, fetch and retry once. If still failing, report the error.
If the rebase has conflicts you cannot confidently resolve, run git rebase --abort and report what conflicts were found."

LOG_ID="$USER_TASK_ID"

# Run claude-pilot
# claude-pilot writes structured JSON result to stdout.
# Streaming text, relay logs, and debug output go to stderr.
STDERR_FILE=$(mktemp)
set +e
# CWD_ARGS is intentionally word-split (multiple flags)
# shellcheck disable=SC2086
PILOT_OUTPUT=$(claude-pilot --verbose --log-dir --task-id "$LOG_ID" $CWD_ARGS -- "$PROMPT" 2>"$STDERR_FILE")
PILOT_EXIT=$?
set -e

# Build result message from structured stdout
# Issue #135: extract first JSON-object line from stdout — skip non-JSON preamble
# (dotenvx banner, debug output). claude-pilot emits single-line JSON via
# JSON.stringify(). Keep raw output for error-reporting fallback.
PILOT_OUTPUT_RAW="$PILOT_OUTPUT"
PILOT_OUTPUT=$(printf '%s\n' "$PILOT_OUTPUT_RAW" | grep -m1 '^{' || true)
: "${PILOT_OUTPUT:=$PILOT_OUTPUT_RAW}"

if [ "$PILOT_EXIT" -eq 0 ]; then
    # Try to extract structured fields from JSON stdout
    STATUS=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.status // empty' 2>/dev/null)
    SESSION_ID=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.session_id // empty' 2>/dev/null)
    TURNS=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.turns // empty' 2>/dev/null)
    COST=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.cost_usd // empty' 2>/dev/null)
    DURATION=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.duration_ms // empty' 2>/dev/null)

    if [ -n "$STATUS" ]; then
        RESULT="resolve-pr-conflicts completed (status: ${STATUS}).
Session: ${SESSION_ID:-unknown}
Turns: ${TURNS:-unknown}
Cost: \$${COST:-unknown}
Duration: ${DURATION:-unknown}ms"
    else
        RESULT="resolve-pr-conflicts completed (exit 0) but output was not structured JSON.

Stdout:
${PILOT_OUTPUT_RAW}"
    fi
else
    RESULT="Log path: /var/log/claude-pilot/${LOG_ID}.log

resolve-pr-conflicts FAILED (exit code ${PILOT_EXIT}).

Stdout:
${PILOT_OUTPUT_RAW}"
fi

# Append stderr tail for debugging context (last 10KB)
if [ -s "$STDERR_FILE" ]; then
    STDERR_TAIL=$(tail -c 10000 "$STDERR_FILE")
    RESULT="${RESULT}

Logs (last 10KB):
${STDERR_TAIL}"
fi
rm -f "$STDERR_FILE"

# Truncate to ~90KB to stay within the 100KB callback limit
RESULT=$(printf '%s' "$RESULT" | head -c 92000)

# Deliver result via mika callback
set +e
if [ -n "$AGENT" ]; then
    mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
else
    mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
fi
CALLBACK_EXIT=$?
CALLBACK_SENT=1
set -e

if [ "$CALLBACK_EXIT" -ne 0 ]; then
    echo "ERROR: callback delivery failed (exit $CALLBACK_EXIT) for task $TASK_ID" >&2
fi
