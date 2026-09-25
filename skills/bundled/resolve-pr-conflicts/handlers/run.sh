#!/bin/bash
# Handler for the resolve-pr-conflicts skill (long-running exec).
# Input: JSON on stdin with task_id and pr_url (preferred) or worktree_path (deprecated).
#        __mika_task_id and __mika_agent are injected by the executor.
# Output: Delivers result via `mika ask --task-id` callback when done.
#
# Spawns claude-pilot with a focused conflict-resolution prompt (no --command).
# When pr_url is provided, derives the worktree path from the PR's branch name
# using the canonical derive-worktree-path script. Falls back to explicit worktree_path.
# The worktree must already exist — this handler does NOT create worktrees.
#
# ─────────────────────────────────────────────────────────────────────────────
# THE PILOT DOES NOT PUSH (mika#2520)
#
# Until 2026-09-24, step 6 of the prompt below was a BARE
# `git push --force-with-lease`, and the pilot that ran it REWOUND `origin/main`.
# The push now lives here, in code, behind `_shared/pr-push-guard.sh`: one site,
# an explicit destination refspec, and a lease pinned to a SHA captured BEFORE
# the session. The prompt carries no push instruction at all, and
# `scripts/check-pilot-push-sites.sh` keeps it that way.
#
# This applies the mika#1318 -> mika#1407 precedent, which removed the push from
# the dev-groom pilot for the same reason. `resolve-pr-conflicts` is the sibling
# skill that never received it.
#
# Order of operations, and the order is the design:
#
#   1. resolve     gh pr view --json headRefName,baseRefName   (one call)
#   2. REFUSE      R1-R3, BEFORE the spawn — zero LLM cost on a refusal
#   3. capture     the lease, before the session
#   4. prompt      with no push instruction
#   5. pilot       rebases, resolves, tests. It does not push.
#   6-8. push      R4-R5, re-affirm R2-R3, then the single guarded site
#   9. report
#
# A refusal goes into RESULT, never into stderr alone: stderr written before the
# pilot launches inherits the executor's `Stdio::piped()` handle, which the
# executor reads only in its `if !status.success()` branch — on a dispatch that
# succeeds those lines land in no file at all (the mika#2050 class).
# `tasks.result` is the durable surface, readable with `mika tasks get`.

set -e

# Ensure ~/.local/bin is in PATH (mika CLI needed for callback delivery)
export PATH="$HOME/.local/bin:$PATH"

# Dependency checks
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }
command -v claude-pilot >/dev/null 2>&1 || { echo "Error: claude-pilot CLI is required but not in PATH" >&2; exit 1; }

# shellcheck source=../../_shared/pr-push-guard.sh
source "$(dirname "$0")/../../_shared/pr-push-guard.sh"

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
RESULT=""
STDERR_FILE=""

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
#
# `PLATFORM_DIR` is relayed by the executor (mika#2532 L4a), replacing
# `${MIKA_PLATFORM_DIR:-…}` — a dead branch, since `sandboxed_pilot_env` rebuilds
# the child env from a positive allowlist that refuses every `MIKA_*` name. The
# expression is self-referential with a default, and that is correct: the same
# name is the relayed variable and the local holding its resolved form.
PLATFORM_DIR="${PLATFORM_DIR:-$HOME/workspace/mika-platform}"
PLATFORM_DIR=$(cd "$PLATFORM_DIR" 2>/dev/null && pwd -P) || PLATFORM_DIR="${PLATFORM_DIR:-$HOME/workspace/mika-platform}"

if [ -z "$USER_TASK_ID" ]; then
    echo "Error: task_id is required" >&2
    exit 1
fi

# ── Step 1-2: resolve the push target from the PR head, and refuse early ─────
#
# The branch is read from the PR, never from the worktree's `git branch
# --show-current` or its push configuration (AC4). This is also where the
# worktree path comes from, so there is exactly one `gh pr view` for both.
#
# The refusal happens BEFORE the spawn, which is stronger than AC3's letter
# ("refused, not pushed"): no LLM turn is spent at all.
PUSH_ELIGIBLE=0

if [ -n "$PR_URL" ]; then
    # Pattern: https://github.com/{owner}/{repo}/pull/{number}
    PR_NUMBER=$(printf '%s' "$PR_URL" | sed -n 's|.*/pull/\([0-9]*\).*|\1|p')
    REPO_FULL=$(printf '%s' "$PR_URL" | sed -n 's|https://github.com/\([^/]*/[^/]*\)/pull/.*|\1|p')

    # Composed through the guard, never hand-rolled: the ticket prefix and the
    # reason vocabulary have one source, and an operator's `GROUP BY` on
    # `tasks.result` sees one spelling of `no_pr_url`.
    if [ -z "$PR_NUMBER" ] || [ -z "$REPO_FULL" ]; then
        pr_push_guard_refuse "no_pr_url" \
            "could not parse a PR number and repo from pr_url '$PR_URL'" \
            "Re-dispatch with a full GitHub PR URL of the form https://github.com/{owner}/{repo}/pull/{number}." || true
        RESULT="$PR_PUSH_GUARD_REFUSAL"
        exit 1
    fi

    if ! pr_push_guard_resolve_target "$REPO_FULL" "$PR_NUMBER"; then
        RESULT="$PR_PUSH_GUARD_REFUSAL"
        exit 1
    fi

    if ! pr_push_guard_check_target; then
        RESULT="$PR_PUSH_GUARD_REFUSAL"
        exit 1
    fi

    PUSH_ELIGIBLE=1

    # The worktree comes from the SAME branch the push will target — one `gh pr
    # view`, one branch, no second derivation that could disagree with the first.
    REPO_SHORT=$(basename "$REPO_FULL")
    set +e
    DERIVED_PATH=$("$PLATFORM_DIR/scripts/derive-worktree-path" --branch "$PR_PUSH_GUARD_BRANCH" --repo "$REPO_SHORT")
    DERIVE_EXIT=$?
    set -e
    if [ "$DERIVE_EXIT" -ne 0 ] || [ -z "$DERIVED_PATH" ]; then
        echo "Error: derive-worktree-path failed (exit $DERIVE_EXIT) for branch '$PR_PUSH_GUARD_BRANCH' repo '$REPO_SHORT'" >&2
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
# `worktree_path` without `pr_url` resolves neither headRefName nor baseRefName,
# so AC4 is unreachable there by construction. Conflict resolution still runs;
# the PUSH is refused under a named reason. Measured population: zero callers.

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

# ── Step 3: capture the lease BEFORE the session ─────────────────────────────
#
# A literal SHA, not `origin/<branch>`. Step 1 of the pilot's own prompt is
# `git fetch origin`, which refreshes the remote-tracking ref — a lease resolved
# after that compares the remote against a snapshot of the remote taken after the
# fact, which is the second axis of the founding defect.
if [ "$PUSH_ELIGIBLE" -eq 1 ]; then
    if ! pr_push_guard_capture_lease "$WORKTREE_PATH"; then
        RESULT="$PR_PUSH_GUARD_REFUSAL"
        exit 1
    fi
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

# ── Step 4: the prompt, with no push instruction ─────────────────────────────
#
# Two LLM derivations are gone. The branch is interpolated (the pilot used to
# re-derive it with `git branch --show-current`, which prints an empty line on a
# detached HEAD and `main` when HEAD is on main), and so is the base (the pilot
# used to look it up with `gh pr list --head <that value>`).
#
# THE STOP IS TOPICAL, NOT ENUMERATIVE, and that is a deliberate divergence from
# the plan's § 3.3 draft, for two reasons that point the same way:
#
#   * The plan's own scan (§ 4) refuses the string `git push` anywhere under
#     `skills/bundled/*/handlers/*.sh`, and that predicate is lexical on purpose
#     — a semantic one would have to tell a prohibition from a prescription, and
#     the founding defect ("6. Push: git push --force-with-lease") is not in
#     command position either, so no positional rule catches it. Spelling the
#     forbidden commands out here would force a special case into the one guard
#     that must stay simple.
#   * Enumerating the forms hands the model the very command template the plan
#     says it wants to remove. Same shape as the mika#2292 rule: a prompt that
#     lists what it forbids teaches it.
#
# The half that actually holds is structural anyway — the prompt contains no
# push, and the handler owns the one that happens
# (feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate).
#
# On the deprecated `worktree_path` path there is no PR to read, so neither the
# branch nor the base is known here. The prompt says so instead of asserting a
# resolved branch it does not have — a "do NOT re-derive it" line above an empty
# value would be the mika#2304 shape: a field asserting, with authority, a
# resolution that did not happen.
if [ "$PUSH_ELIGIBLE" -eq 1 ]; then
    TARGET_BLOCK="Branch: ${PR_PUSH_GUARD_BRANCH}   (resolved from the PR head — do NOT re-derive it)
Base:   ${PR_PUSH_GUARD_BASE}"
    REBASE_ONTO="origin/${PR_PUSH_GUARD_BASE}"
else
    TARGET_BLOCK="Branch: not resolved — this dispatch supplied no pr_url, so work on whatever
        branch the worktree is already checked out on. Do not switch branches.
Base:   not resolved — detect it from the PR if you can, otherwise use main."
    REBASE_ONTO="origin/main"
fi

PROMPT="Resolve merge conflicts on this branch.

${TARGET_BLOCK}

Steps:
1. git fetch origin
2. git rebase ${REBASE_ONTO}
3. If conflicts arise, resolve each one: read both sides, choose the correct
   resolution, git add the file, git rebase --continue
4. After the rebase completes, run the repo's test suite if one exists
   (check for Makefile, cargo, npm, etc.)
5. Stop there, on the branch, with a clean tree.

Publishing is NOT part of your scope. Do not send anything to the remote, and do
not change where a publish would land (no edits to remotes, refspecs, upstreams
or push defaults). Read-only remote access such as fetch is fine. The handler
publishes after this session exits, using an explicit destination refspec and a
lease captured before you started — anything you sent yourself would break that
lease and the dispatch would fail.

If the rebase has conflicts you cannot confidently resolve, run
git rebase --abort and report what conflicts were found."

LOG_ID="$USER_TASK_ID"

# ── Step 5: run claude-pilot ─────────────────────────────────────────────────
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

# ── Steps 6-8: the push, after the session, at the single guarded site ───────
#
# `pr_push_guard_push` re-checks the worktree (R4, R5), re-affirms the target on
# shell variables the pilot cannot touch (R2, R3), and only then pushes with the
# explicit refspec and the pre-session lease.
#
# There is no retry, deliberately: the one legitimate failure here is "somebody
# pushed to the branch during the session", and re-leasing on the new SHA would
# overwrite their work. A transient failure costs one dispatch — the rebase
# commits stay in the worktree, and the line below says so.
if [ "$PUSH_ELIGIBLE" -eq 1 ]; then
    # NOT a command substitution. `pr_push_guard_push` sets
    # PR_PUSH_GUARD_REFUSAL, and `$( … )` runs it in a SUBSHELL, so that
    # variable would never reach this shell — the refusal branch below would be
    # dead code and every post-session refusal would be reported to the
    # operator as a concurrent-push failure, under a reason it never had.
    # Output goes through a temp file instead.
    PUSH_LOG=$(mktemp)
    set +e
    pr_push_guard_push "$WORKTREE_PATH" >"$PUSH_LOG" 2>&1
    PUSH_EXIT=$?
    set -e
    PUSH_OUTPUT=$(cat "$PUSH_LOG")
    rm -f "$PUSH_LOG"

    if [ "$PUSH_EXIT" -eq 0 ]; then
        RESULT="${RESULT}

Push: OK — ${PR_PUSH_GUARD_BRANCH} (explicit refspec HEAD:refs/heads/${PR_PUSH_GUARD_BRANCH}, lease pinned to ${PR_PUSH_GUARD_EXPECTED_SHA})."
    elif [ -n "$PR_PUSH_GUARD_REFUSAL" ]; then
        # The refusal LEADS the result. Two reasons, both measured: the operator
        # query published in CLAUDE.md anchors on `REFUSED (mika#2520)%`, and
        # `RESULT` is truncated at 92000 bytes below — a refusal appended after
        # a large pilot log is a refusal that can be cut off.
        RESULT="${PR_PUSH_GUARD_REFUSAL}
The rebase work, if any, is intact in ${WORKTREE_PATH}.

${RESULT}"
    else
        RESULT="${RESULT}

Push: FAILED (exit ${PUSH_EXIT}) — not retried, deliberately: the legitimate failure here is a concurrent push, and re-leasing would overwrite it. The rebase work is intact in ${WORKTREE_PATH}.

${PUSH_OUTPUT}"
    fi
else
    pr_push_guard_refuse "no_pr_url" \
        "this dispatch supplied only the deprecated worktree_path, which resolves no PR head, so the push target could not be derived from the PR" \
        "Nothing was pushed; the rebase work is intact in ${WORKTREE_PATH}. Re-dispatch with pr_url to publish." || true
    RESULT="${PR_PUSH_GUARD_REFUSAL}

${RESULT}"
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
