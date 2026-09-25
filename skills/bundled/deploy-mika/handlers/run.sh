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

# Use provided cwd or default to the main mika repo root (same pattern as build-mika)
#
# `PLATFORM_DIR` is relayed by the executor (mika#2532 L4a). It replaces
# `${MIKA_PLATFORM_DIR:-…}`, a DEAD BRANCH: `sandboxed_pilot_env` rebuilds the
# child env from a positive allowlist refusing every `MIKA_*` name — and this
# handler additionally unsets them all at line 22 — so the left-hand side could
# never be read. The read merely LOOKED like it honoured an operator setting.
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

# --- Mutual exclusion ---
LOCKFILE="/tmp/deploy-mika.lock"
exec 9>"$LOCKFILE"
flock -n 9 || { RESULT="FAILED: another deploy is in progress"; exit 1; }

# --- Path validation ---
# Self-referential with a default, and that is correct: `PLATFORM_DIR` is the
# relayed variable (mika#2532 L4a) AND the local that holds its resolved form.
PLATFORM_DIR="${PLATFORM_DIR:-$HOME/workspace/mika-platform}"
PLATFORM_DIR=$(cd "$PLATFORM_DIR" 2>/dev/null && pwd -P) || PLATFORM_DIR="${PLATFORM_DIR:-$HOME/workspace/mika-platform}"

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
