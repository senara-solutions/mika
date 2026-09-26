#!/bin/sh
# Deploy handler for the deploy-mika skill.
# Deploys all 3 mika binaries (mika, mika-spirit, mika-gateway) and restarts
# services via rc-service (OpenRC).
#
# Input: JSON on stdin with __mika_task_id, __mika_agent, and cwd
# Output: Delivers result via `mika ask --task-id` callback

set -e

# Ensure ~/.local/bin is in PATH (mika CLI needed for callback delivery)
export PATH="$HOME/.local/bin:$PATH"

# Dependency checks
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }

# Read input JSON from stdin
INPUT=$(cat)

# Scrub all MIKA_* env vars so child processes cannot leak secrets
for _var in $(env | grep -o '^MIKA_[^=]*'); do unset "$_var"; done

# Parse callback fields from enriched input
TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.__mika_task_id // empty')
AGENT=$(printf '%s\n' "$INPUT" | jq -r '.__mika_agent // empty')
CWD=$(printf '%s\n' "$INPUT" | jq -r '.cwd // empty')

if [ -z "$TASK_ID" ]; then
    echo "Error: no __mika_task_id in input (not running as long-running handler?)" >&2
    exit 1
fi

# Use provided cwd or default to the main mika repo root (same pattern as build-mika).
# `PLATFORM_DIR` is the name the child receives — relayed from the operator's
# `MIKA_PLATFORM_DIR` by `inject_platform_dir_env` (mika#2536). Two independent
# reasons the prefixed form was a DEAD branch here: `sandboxed_pilot_env` refuses
# every `MIKA_*`, AND the scrub loop above has already unset them all.
if [ -z "$CWD" ]; then
    _DEFAULT="${PLATFORM_DIR:-$HOME/workspace/mika-platform}/mika"
    CWD=$(cd "$_DEFAULT" 2>/dev/null && pwd -P) || CWD="$_DEFAULT"
fi

# --- Crash-recovery EXIT trap ---
CALLBACK_SENT=0

deliver_callback() {
    _EXIT_CODE=$?
    [ "$CALLBACK_SENT" -eq 1 ] && return
    [ -z "$TASK_ID" ] && return
    if [ -z "$RESULT" ]; then
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Deploy failed before building result."
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

# --- cwd composability guard (mika#2536) ---
# Sourced AFTER the trap: a failure here must still deliver a callback.
# Fail-closed and NAMED, same rationale as build-mika.
_CWD_GUARD="$(dirname "$0")/../../_shared/cwd-guard.sh"
if [ -r "$_CWD_GUARD" ]; then
    # shellcheck source=../../_shared/cwd-guard.sh
    . "$_CWD_GUARD"
else
    RESULT="REFUSED (cwd-guard, mika#2536): shared guard unreadable at ${_CWD_GUARD}.
\`skills/bundled/_shared/\` is a projection of the BINARY, not of the checkout: rebuild
(\`make deploy\`) then re-seed, and read \`~/.mika/skills/.manifest-writer\`."
    exit 1
fi

# --- Mutual exclusion ---
LOCKFILE="/tmp/deploy-mika.lock"
exec 9>"$LOCKFILE"
flock -n 9 || { RESULT="FAILED: another deploy is in progress"; exit 1; }

# --- Path validation ---
# Self-referential assignment with a default IS the canonical shape of an
# operator knob (`VAR="${VAR:-default}"`), and it is what the mika#2508 scan's
# term 4bis recognizes: the read is not evicted by the write, so `PLATFORM_DIR`
# stays in the population the relay must cover.
PLATFORM_DIR="${PLATFORM_DIR:-$HOME/workspace/mika-platform}"
PLATFORM_DIR=$(cd "$PLATFORM_DIR" 2>/dev/null && pwd -P) || PLATFORM_DIR="${PLATFORM_DIR:-$HOME/workspace/mika-platform}"

# Refuse an uncomposable cwd BY NAMING IT (mika#2536 R6). This ADDS to the
# allowed-prefix `case` below; it does not replace it — that one is a safety
# perimeter, this one is composability, and weakening the former would be a side
# effect of an observability ticket.
if ! validate_cwd "$CWD"; then
    RESULT="$CWD_REFUSAL"
    exit 1
fi

CWD=$(cd "$CWD" 2>/dev/null && pwd -P) || {
    RESULT="FAILED: path does not exist: $CWD"
    exit 1
}

case "$CWD" in
    "$PLATFORM_DIR"/.claude/worktrees/* ) ;; # worktree OK
    "$PLATFORM_DIR"/mika )                   ;; # main checkout OK
    * ) RESULT="FAILED: path not under allowed prefix: $CWD"; exit 1 ;;
esac

# --- All 3 binaries ---
BINARIES="mika mika-spirit mika-gateway"
INSTALL_DIR="$HOME/.local/bin"
DEPLOY_LOG=""
FAILED=0

for bin in $BINARIES; do
    SRC="$CWD/target/release/$bin"
    DST="$INSTALL_DIR/$bin"

    # Verify source exists
    if [ ! -f "$SRC" ]; then
        DEPLOY_LOG="${DEPLOY_LOG}$bin: SKIPPED (not found at $SRC)
"
        continue
    fi

    # Pre-deploy health check (--version or --help)
    set +e
    VER=$("$SRC" --version 2>&1 | head -1)
    VER_EXIT=$?
    set -e

    if [ "$VER_EXIT" -ne 0 ]; then
        DEPLOY_LOG="${DEPLOY_LOG}$bin: FAILED health check (exit $VER_EXIT)
"
        FAILED=1
        continue
    fi

    # Backup current binary
    BACKUP=$(mktemp "/tmp/${bin}.backup.XXXXXXXX")
    if [ -f "$DST" ]; then
        cp "$DST" "$BACKUP" 2>/dev/null || true
    fi

    # Install new binary (atomic inode replacement — no ETXTBSY when target is running)
    set +e
    install -m 755 "$SRC" "$DST"
    CP_EXIT=$?
    set -e

    if [ "$CP_EXIT" -ne 0 ]; then
        # Restore on failure
        [ -f "$BACKUP" ] && cp "$BACKUP" "$DST" 2>/dev/null || true
        rm -f "$BACKUP"
        DEPLOY_LOG="${DEPLOY_LOG}$bin: FAILED copy (exit $CP_EXIT), backup restored
"
        FAILED=1
        continue
    fi

    rm -f "$BACKUP"
    DEPLOY_LOG="${DEPLOY_LOG}$bin: OK ($VER)
"
done

# --- Restart services via rc-service (OpenRC) ---
RESTART_LOG=""

# Restart mika-spirit and mika-gateway via rc-service if the init scripts exist
for svc in mika-spirit mika-gateway; do
    if [ -f "/etc/init.d/$svc" ]; then
        set +e
        sudo rc-service "$svc" restart 2>&1
        SVC_EXIT=$?
        set -e

        if [ "$SVC_EXIT" -eq 0 ]; then
            RESTART_LOG="${RESTART_LOG}$svc: restarted via rc-service
"
        else
            RESTART_LOG="${RESTART_LOG}$svc: rc-service restart failed (exit $SVC_EXIT)
"
            FAILED=1
        fi
    else
        RESTART_LOG="${RESTART_LOG}$svc: no init script (skipped restart)
"
    fi
done

# --- Build result ---
if [ "$FAILED" -eq 1 ]; then
    RESULT="Deploy completed with errors.

BINARIES:
${DEPLOY_LOG}
SERVICES:
${RESTART_LOG}"
else
    RESULT="Deploy succeeded.

BINARIES:
${DEPLOY_LOG}
SERVICES:
${RESTART_LOG}"
fi

# EXIT trap delivers the callback
