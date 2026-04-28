#!/bin/sh
# Handler for the dev-pilot skill (long-running exec).
# Input: JSON on stdin with skill, prompt, and task_id fields.
#        __mika_task_id and __mika_agent are injected by the executor.
# Output: Delivers result via `mika ask --task-id` callback when claude-pilot finishes.
#
# For repo#number prompts: derives branch from issue labels, creates a worktree,
# and runs claude-pilot with --command "/mika" in the target repo.
# For free-text prompts: passes prompt as-is to claude-pilot (no worktree).

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
    # Guard: skip if already delivered or no task ID
    [ "$CALLBACK_SENT" -eq 1 ] && { [ -n "$STDOUT_FILE" ] && rm -f "$STDOUT_FILE"; [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"; return; }
    [ -z "$TASK_ID" ] && { [ -n "$STDOUT_FILE" ] && rm -f "$STDOUT_FILE"; [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"; return; }
    # Try to recover result from stdout file if RESULT was never populated.
    # Captures cases where the process is killed after writing stdout but
    # before the shell captures the variable — the file survives.
    if [ -z "$RESULT" ] && [ -n "$STDOUT_FILE" ] && [ -f "$STDOUT_FILE" ]; then
        _RECOVERED_RAW=$(cat "$STDOUT_FILE" 2>/dev/null)
        # Issue #135: extract first JSON line from possible preamble (dotenvx banner)
        _RECOVERED=$(printf '%s\n' "$_RECOVERED_RAW" | grep -m1 '^{' || true)
        : "${_RECOVERED:=$_RECOVERED_RAW}"
        _STATUS=$(printf '%s\n' "$_RECOVERED" | jq -r '.status // empty' 2>/dev/null)
        if [ -n "$_STATUS" ]; then
            RESULT="claude-pilot completed (status: ${_STATUS}, recovered from crash).
Exit code: ${_EXIT_CODE}
Stdout recovered from file."
        fi
    fi
    # Capture stderr tail on crash path BEFORE deleting the file (#104)
    if [ -z "$RESULT" ] && [ -n "$STDERR_FILE" ] && [ -f "$STDERR_FILE" ]; then
        _STDERR_TAIL=$(tail -c 10000 "$STDERR_FILE" 2>/dev/null)
        if [ -n "$_STDERR_TAIL" ]; then
            RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result.

Stderr (last 10KB):
${_STDERR_TAIL}"
        fi
    fi
    # Clean up temp files AFTER capture
    [ -n "$STDOUT_FILE" ] && rm -f "$STDOUT_FILE"
    [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"
    if [ -z "$RESULT" ]; then
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result."
    fi
    # Issue #138: best-effort PR URL discovery on crash recovery path.
    # If claude-pilot created a PR before crashing, include the URL so the
    # self-dev callback can extract it without extra tool calls.
    if [ -n "$REPO" ] && [ -n "$BRANCH" ]; then
        _PR_URL=$(gh pr list --repo "senara-solutions/$REPO" --head "$BRANCH" --json url --jq '.[0].url' 2>/dev/null || true)
        if [ -n "$_PR_URL" ]; then
            RESULT="${RESULT}
PR: ${_PR_URL}"
        fi
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
SKILL=$(printf '%s\n' "$INPUT" | jq -r '.skill // empty')
PROMPT=$(printf '%s\n' "$INPUT" | jq -r '.prompt // empty')
USER_TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.task_id // empty')
DRY_RUN=$(printf '%s\n' "$INPUT" | jq -r '.dry_run // empty')
ITERATION_CTX=$(printf '%s\n' "$INPUT" | jq -r '.iteration_context // empty')

# Validate skill argument
if [ -z "$SKILL" ]; then
    echo "Error: missing required argument 'skill'; valid values: [\"dev-pilot\"]" >&2
    exit 1
fi
if [ "$SKILL" != "dev-pilot" ]; then
    echo "Error: invalid skill '${SKILL}'; valid values: [\"dev-pilot\"]" >&2
    exit 1
fi

# mika-platform root — base for sub-repo resolution
# Resolve symlinks so constructed paths use canonical form
PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
PLATFORM_DIR=$(cd "$PLATFORM_DIR" 2>/dev/null && pwd -P) || PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
PLATFORM_REPO_NAME=$(basename "$PLATFORM_DIR")

if [ -z "$PROMPT" ]; then
    echo "Error: prompt is required" >&2
    exit 1
fi

if [ -z "$USER_TASK_ID" ]; then
    echo "Error: task_id is required" >&2
    exit 1
fi

# Warn if task_id doesn't look like a UUID (non-blocking — continues with provided value)
if ! printf '%s' "$USER_TASK_ID" | grep -qiE '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'; then
    echo "WARNING: task_id '$USER_TASK_ID' does not match UUID format (expected 36-char UUID like '15383984-a3e7-41bf-ac6f-630ba9a89d63'). Logs will land at /var/log/claude-pilot/${USER_TASK_ID}.log — this may break log-to-task correlation." >&2
fi

# GitHub App installation token for gh CLI.
#
# mika#520 started injecting GH_TOKEN into exec handler env (same pattern as
# builtin run_gh) so subprocess `gh` calls authenticate as the agent identity.
# If GH_TOKEN is already present, gh CLI picks it up automatically — we
# must NOT call `gh auth login --with-token`, which refuses to run when
# GH_TOKEN is set in env ("The value of the GH_TOKEN environment variable
# is being used for authentication. To have GitHub CLI store credentials
# instead, first clear the value from the environment.") and exits 1. Under
# `set -e` with `2>/dev/null` suppressing the stderr, that kills the handler
# silently — STDERR_FILE isn't created yet, so the EXIT trap writes an
# empty-looking "HANDLER CRASH" with no diagnostic. That's the bug we hit
# after deploying #520: every claude-pilot dispatch crashed before it could
# invoke claude-pilot.
#
# Two cases:
#   1. Runtime already injected GH_TOKEN (post-#520): use it as-is — gh CLI
#      env auth is all we need.
#   2. No GH_TOKEN in env (legacy / manual invocation): fetch a token and
#      persist it into gh config via login + switch, so bot identity sticks.
if [ -z "${GH_TOKEN:-}" ]; then
    GH_APP_TOKEN=$(mika ${AGENT:+--agent "$AGENT"} token github 2>/dev/null)
    if [ -n "$GH_APP_TOKEN" ]; then
        echo "$GH_APP_TOKEN" | gh auth login --with-token 2>/dev/null
        gh auth switch --user "mika-platform-bot[bot]" 2>/dev/null || true
    else
        echo "WARNING: mika token github failed — gh CLI will fall back to host credentials" >&2
    fi
fi

# Scrub sensitive env vars
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY

# --- Parse repo#number format ---
# Matches: mika#214, mika-skills#8, mika-cloud#50
REPO=""
ISSUE_NUM=""
if printf '%s' "$PROMPT" | grep -qE '^[a-zA-Z0-9_-]+#[0-9]+$'; then
    REPO=$(printf '%s' "$PROMPT" | sed 's/#.*//')
    ISSUE_NUM=$(printf '%s' "$PROMPT" | sed 's/.*#//')
fi

if [ -n "$REPO" ] && [ -n "$ISSUE_NUM" ]; then
    # --- repo#number mode: derive everything from the issue ---

    # Use executor-injected callback UUID (always real, always in DB) for
    # claude-pilot's --task-id. Avoids the fabrication vector that bit mika#595
    # where the agent-provided task_id was a well-formed but nonexistent UUID
    # and every relay callback failed with "task not found". See mika#596.
    LOG_ID="$TASK_ID"

    # Validate repo directory exists (mika-platform itself IS PLATFORM_DIR)
    if [ "$REPO" = "$PLATFORM_REPO_NAME" ]; then
        SUB_REPO_DIR="$PLATFORM_DIR"
    else
        SUB_REPO_DIR="$PLATFORM_DIR/$REPO"
    fi
    if [ ! -d "$SUB_REPO_DIR/.git" ] && ! [ -f "$SUB_REPO_DIR/.git" ]; then
        echo "Error: $SUB_REPO_DIR is not a git repository" >&2
        exit 1
    fi

    # Fetch issue — validates it exists and is open, gets labels + title + body
    ISSUE_JSON=$(gh issue view "$ISSUE_NUM" --repo "senara-solutions/$REPO" --json state,title,labels,body 2>/dev/null) || {
        echo "Error: Issue #${ISSUE_NUM} not found in senara-solutions/${REPO}. Aborting." >&2
        exit 1
    }

    ISSUE_STATE=$(printf '%s' "$ISSUE_JSON" | jq -r '.state')
    if [ "$ISSUE_STATE" = "CLOSED" ]; then
        echo "Error: Issue #${ISSUE_NUM} in senara-solutions/${REPO} is closed. Reopen first." >&2
        exit 1
    fi

    # Check issue body for a pre-committed branch callout (from planning sessions).
    # Format: > - **Branch:** `feat/687/domain-graph-builder`
    # This takes priority over title-derived branch names so claude-pilot lands
    # on the branch where the plan file was committed.
    ISSUE_BODY=$(printf '%s' "$ISSUE_JSON" | jq -r '.body // empty')
    CALLOUT_BRANCH=$(printf '%s' "$ISSUE_BODY" | grep -oP '>\s*-?\s*\*\*Branch:\*\*\s*`\K[^`]+' | head -1)

    if [ -n "$CALLOUT_BRANCH" ]; then
        BRANCH="$CALLOUT_BRANCH"
    else
        # Derive branch type from labels
        LABELS=$(printf '%s' "$ISSUE_JSON" | jq -r '.labels[].name' 2>/dev/null)
        BRANCH_TYPE="feat"
        if printf '%s' "$LABELS" | grep -qi '^bug$'; then
            BRANCH_TYPE="fix"
        elif printf '%s' "$LABELS" | grep -qi '^chore$'; then
            BRANCH_TYPE="chore"
        fi

        # Derive branch name: type/number/kebab-title
        ISSUE_TITLE=$(printf '%s' "$ISSUE_JSON" | jq -r '.title')
        KEBAB_TITLE=$(printf '%s' "$ISSUE_TITLE" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]/-/g' | sed 's/--*/-/g' | sed 's/^-//;s/-$//' | cut -c1-40)
        BRANCH="${BRANCH_TYPE}/${ISSUE_NUM}/${KEBAB_TITLE}"
    fi

    # Sync main before branching to avoid stale worktrees.
    # Use plain `fetch origin main` (not the `main:main` refspec form) because the
    # refspec form refuses to fast-forward a checked-out branch — and the primary
    # repo checkout always has `main` checked out, so `main:main` ALWAYS fails
    # here with "refusing to fetch into branch 'refs/heads/main' checked out at ..."
    # and was silently swallowed by `2>/dev/null || true`. Result: sequential
    # tickets in the same repo were branched from stale local main, causing
    # avoidable merge conflicts on the 2nd/3rd PR in any same-repo chain.
    # Plain `fetch origin main` writes only to refs/remotes/origin/main and
    # has no checked-out-branch restriction, so we can always branch from
    # `origin/main` below and get the latest merged tip.
    git -C "$SUB_REPO_DIR" fetch origin main 2>/dev/null || true

    # Create worktree
    SANITIZED=$(printf '%s' "$BRANCH" | tr '/' '-')
    WORKTREE_DIR="${PLATFORM_DIR}/.claude/worktrees/${SANITIZED}/${REPO}"

    # Reuse existing worktree if valid
    if [ -d "$WORKTREE_DIR" ] && git -C "$WORKTREE_DIR" rev-parse --git-dir >/dev/null 2>&1; then
        git -C "$WORKTREE_DIR" checkout "$BRANCH" 2>/dev/null || true
    else
        git -C "$SUB_REPO_DIR" worktree remove --force "$WORKTREE_DIR" 2>/dev/null || true
        if ! git -C "$SUB_REPO_DIR" worktree add -b "$BRANCH" "$WORKTREE_DIR" origin/main 2>/dev/null; then
            git -C "$SUB_REPO_DIR" worktree add "$WORKTREE_DIR" "$BRANCH"
        fi
    fi

    # Rebase-or-abort guard: catch up branches pre-committed from stale main.
    # At this point origin/main is fresh (line 233 fetched it) and WORKTREE_DIR is set.
    # If the branch is behind origin/main, auto-rebase. On conflict, capture the
    # conflicted-file list BEFORE abort (abort resets the index) and exit with a
    # structured STATUS=REBASE_CONFLICT discriminator so the EXIT trap delivers it.
    BEHIND=$(git -C "$WORKTREE_DIR" rev-list --count HEAD..origin/main 2>/dev/null || echo 0)
    if [ "$BEHIND" -gt 0 ]; then
        if git -C "$WORKTREE_DIR" rebase origin/main 2>/dev/null; then
            echo "Rebased ${BRANCH} onto origin/main (${BEHIND} commits caught up)." >&2
        else
            CONFLICTS=$(git -C "$WORKTREE_DIR" diff --name-only --diff-filter=U 2>/dev/null | tr '\n' ' ')
            git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
            RESULT="STATUS=REBASE_CONFLICT
Branch ${BRANCH} is ${BEHIND} commits behind origin/main.
Conflicted files: ${CONFLICTS:-<unable to capture>}
Resolve manually before re-dispatching ${REPO}#${ISSUE_NUM}."
            exit 1
        fi
    fi

    # Copy gitignored .claude/ config into worktree (relay + permissions only)
    # NOTE: Do NOT copy .claude/commands/ — Claude Code discovers commands from
    # the repo's own .claude/commands/. Copying the meta-repo's commands overwrites
    # the repo-level /mika with the dispatcher, causing infinite recursion.
    mkdir -p "$WORKTREE_DIR/.claude"
    cp "$PLATFORM_DIR/.claude/claude-pilot.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
    cp "$PLATFORM_DIR/.claude/settings.local.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true

    CWD_ARGS="--cwd $WORKTREE_DIR"
    if [ -f "$WORKTREE_DIR/.claude/claude-pilot.json" ]; then
        CWD_ARGS="$CWD_ARGS --relay-config $WORKTREE_DIR/.claude/claude-pilot.json"
    elif [ -f "$PLATFORM_DIR/.claude/claude-pilot.json" ]; then
        CWD_ARGS="$CWD_ARGS --relay-config $PLATFORM_DIR/.claude/claude-pilot.json"
    fi

    # The prompt becomes the QUALIFIED issue reference; /mika is passed via --command.
    # Must use `${REPO}#${ISSUE_NUM}` (not bare `#${ISSUE_NUM}`) because the meta-repo
    # `/mika` command treats a bare number as "probe all repos to find the issue" and
    # halts with a disambiguation prompt when the same number exists in multiple repos.
    # Example: `#8` exists in mika, mika-cloud, mika-skills, AND mika-platform, so bare
    # `#8` triggers the "which repo?" path and claude-pilot ends with no work done
    # (observed 2026-04-11 on mika-platform#8 dispatch). The qualified form
    # `mika-platform#8` takes the direct-dispatch path unambiguously.
    PROMPT="${REPO}#${ISSUE_NUM}"

    # Append iteration context if provided (for PR iteration workflows)
    if [ -n "$ITERATION_CTX" ]; then
        # Truncate to 4KB to avoid ARG_MAX issues
        ITERATION_CTX=$(printf '%s' "$ITERATION_CTX" | head -c 4096)
        PROMPT=$(printf '%s#%s\n\nITERATION CONTEXT:\n%s' "$REPO" "$ISSUE_NUM" "$ITERATION_CTX")
    fi

    # Save pre-run HEAD SHA for post-flight diff check
    PRE_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)

    # Dry-run: output diagnostics and exit
    if [ "$DRY_RUN" = "true" ] || [ "$DRY_RUN" = "1" ]; then
        jq -n --arg repo "$REPO" --argjson issue "$ISSUE_NUM" --arg branch "$BRANCH" \
            --arg worktree "$WORKTREE_DIR" --arg prompt "$PROMPT" \
            '{dry_run:true, repo:$repo, issue:$issue, branch:$branch, worktree_dir:$worktree, prompt:$prompt}'
        git -C "$SUB_REPO_DIR" worktree remove --force "$WORKTREE_DIR" 2>/dev/null || true
        PARENT_DIR="${PLATFORM_DIR}/.claude/worktrees/${SANITIZED}"
        rmdir "$PARENT_DIR" 2>/dev/null || true
        exit 0
    fi
else
    # --- Free-text mode: pass prompt as-is, no worktree ---
    PRE_RUN_HEAD=""
    # Use executor-injected callback UUID (always real, always in DB) for
    # claude-pilot's --task-id. Avoids the fabrication vector that bit mika#595
    # where the agent-provided task_id was a well-formed but nonexistent UUID
    # and every relay callback failed with "task not found". See mika#596.
    LOG_ID="$TASK_ID"
    CWD_ARGS="--cwd $PLATFORM_DIR"

    if [ -n "$ITERATION_CTX" ]; then
        echo "Warning: iteration_context provided but prompt is not in repo#number format — ignoring" >&2
    fi

    if [ "$DRY_RUN" = "true" ] || [ "$DRY_RUN" = "1" ]; then
        jq -n --arg prompt "$PROMPT" \
            '{dry_run:true, repo:null, issue:null, branch:null, worktree_dir:null, prompt:$prompt}'
        exit 0
    fi
fi

# Run claude-pilot
# claude-pilot writes structured JSON result to stdout (~200 bytes).
# Streaming text, relay logs, and debug output go to stderr.
# Capture stdout to a FILE (not variable) so the output survives even if the
# process crashes during teardown after writing the result JSON.
STDERR_FILE=$(mktemp)
STDOUT_FILE=$(mktemp)
set +e
# CWD_ARGS is intentionally word-split (multiple flags)
# shellcheck disable=SC2086
claude-pilot --verbose --log-dir --task-id "$LOG_ID" --command "/mika" $CWD_ARGS -- "$PROMPT" >"$STDOUT_FILE" 2>"$STDERR_FILE"
PILOT_EXIT=$?
# Do NOT re-enable set -e here — keep error tolerance through result building
# and callback delivery. The EXIT trap handles failures.
# Issue #135: extract first JSON-object line from stdout — skip non-JSON preamble
# (dotenvx banner, debug output). claude-pilot emits single-line JSON via
# JSON.stringify(). Keep raw output for error-reporting fallback.
PILOT_OUTPUT_RAW=$(cat "$STDOUT_FILE" 2>/dev/null)
PILOT_OUTPUT=$(printf '%s\n' "$PILOT_OUTPUT_RAW" | grep -m1 '^{' || true)
: "${PILOT_OUTPUT:=$PILOT_OUTPUT_RAW}"
rm -f "$STDOUT_FILE"

# Build result message from structured stdout (not merged stderr).
# Defense-in-depth: try parsing stdout FIRST regardless of exit code.
# A non-zero exit with valid stdout JSON means the session completed but the
# process exited abnormally during teardown — trust the structured result.
STATUS=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.status // empty' 2>/dev/null)
SESSION_ID=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.session_id // empty' 2>/dev/null)
TURNS=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.turns // empty' 2>/dev/null)
COST=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.cost_usd // empty' 2>/dev/null)
DURATION=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.duration_ms // empty' 2>/dev/null)

if [ -n "$STATUS" ]; then
    # Stdout has valid structured result — trust it over exit code
    RESULT="claude-pilot completed (status: ${STATUS}).
Session: ${SESSION_ID:-unknown}
Turns: ${TURNS:-unknown}
Cost: \$${COST:-unknown}
Duration: ${DURATION:-unknown}ms"

    if [ "$PILOT_EXIT" -ne 0 ]; then
        RESULT="${RESULT}
Note: process exited with code ${PILOT_EXIT} after session completed — result is valid."
    fi

    # Post-flight diff check: detect zero-commit "success" in repo#number mode.
    if [ -n "$PRE_RUN_HEAD" ] && [ -n "$REPO" ]; then
        POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
        if [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ]; then
            RESULT="PIPELINE FAILURE: claude-pilot exited 0 but HEAD unchanged (pre: ${PRE_RUN_HEAD}, post: ${POST_RUN_HEAD}). Zero new commits produced.

${RESULT}"
        fi
    fi

    # Issue #138: Discover actual PR URL from the branch (repo#number mode only).
    # Appends a "PR: <url>" line so the self-dev callback can extract pr_url
    # in one step instead of wasting 2-4 tool calls searching for it.
    # gh pr list returns [] if no PR exists yet — jq yields empty string, which is fine.
    if [ -n "$REPO" ] && [ -n "$BRANCH" ]; then
        PR_URL=$(gh pr list --repo "senara-solutions/$REPO" --head "$BRANCH" --json url --jq '.[0].url' 2>/dev/null || true)
        if [ -n "$PR_URL" ]; then
            RESULT="${RESULT}
PR: ${PR_URL}"
        fi
    fi
elif [ "$PILOT_EXIT" -eq 0 ]; then
    RESULT="claude-pilot completed (exit 0) but output was not structured JSON.

Stdout:
${PILOT_OUTPUT_RAW}"
else
    RESULT="Log path: /var/log/claude-pilot/${LOG_ID}.log

claude-pilot FAILED (exit code ${PILOT_EXIT}).

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
