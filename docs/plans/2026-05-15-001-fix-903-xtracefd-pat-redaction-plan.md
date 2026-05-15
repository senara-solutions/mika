---
ticket: mika#903
type: fix
priority: p0-critical
module: skills/bundled/_shared/dispatch-lib.sh
tags: [security, xtrace, secret-redaction, dispatch-lib]
---

# Plan: Redact secrets from BASH_XTRACEFD trace output (mika#903)

## Problem

`dispatch_claude_pilot()` enables `set -x` with `BASH_XTRACEFD=9` (mika#887 diagnostic instrumentation). The xtrace expands all command arguments before logging, so `_setup_gh_auth()` leaks the GitHub PAT in plaintext into:

1. `/tmp/dev-pilot-trace-<pid>.log` (world-readable)
2. The EXIT-trap `RESULT` payload (last 50 lines of trace tail)
3. `tasks.result` and `messages.content` in mika.db via callback delivery

## Scope

**In scope:**
- Redact secrets in xtrace output (Surface 1: `set +x` around secret-handling)
- Scrub trace tail in EXIT trap before including in RESULT (Surface 3: pattern-based redaction)
- Restrict trace file permissions to 0600 (Surface 0: file mode hardening)
- Test fixture: synthetic crash that verifies redaction

**Out of scope:**
- Removing mika#887's diagnostic instrumentation (keep the trace, redact the secrets)
- Scrubbing already-leaked PATs in mika.db (operator decides on retention/scrub/rotate)
- gh CLI host auth state (separate concern)

## Implementation

### Step 1: Restrict trace file permissions (line ~486-487)

Change the trace file creation in `dispatch_claude_pilot()` to use restrictive permissions:

```bash
# --- Diagnostic trace (mika#887) ---
TRACE_FILE="/tmp/dev-pilot-trace-$$.log"
umask_prev=$(umask)
umask 077
exec 9>>"$TRACE_FILE" 2>/dev/null || exec 9>/dev/null
umask "$umask_prev"
```

This ensures the trace file is created with mode 0600 (owner-only). The `umask` is restored immediately after the `exec` to avoid affecting subsequent file operations.

### Step 2: Wrap `_setup_gh_auth` with `set +x` / `set -x` (lines ~144-156)

Add xtrace suppression around the secret-handling section:

```bash
_setup_gh_auth() {
    # Suppress xtrace to prevent PAT from appearing in trace logs (mika#903).
    { set +x; } 2>/dev/null
    if [ -z "${GH_TOKEN:-}" ]; then
        GH_APP_TOKEN=$(mika ${AGENT:+--agent "$AGENT"} token github 2>/dev/null)
        if [ -n "$GH_APP_TOKEN" ]; then
            echo "$GH_APP_TOKEN" | gh auth login --with-token 2>/dev/null
            gh auth switch --user "mika-platform-bot[bot]" 2>/dev/null || true
        else
            echo "WARNING: mika token github failed — gh CLI will fall back to host credentials" >&2
        fi
    fi
    # Re-enable xtrace (was set by dispatch_claude_pilot before calling us).
    set -x
}
```

The `{ set +x; } 2>/dev/null` pattern suppresses the trace output of the `set +x` command itself (otherwise bash logs `+ set +x` to the trace, which is harmless but noisy).

**Why this is sufficient:** `_setup_gh_auth` is the only function in dispatch-lib.sh that handles secrets after `set -x` is enabled. `_scrub_env` (line 158) runs after `_setup_gh_auth` and only `unset`s variables — the xtrace for `unset FOO` doesn't leak values. `_parse_input_json` runs before `set -x` is enabled.

### Step 3: Scrub known secret patterns from EXIT trap's trace tail (lines ~53-65)

Add a sed-based scrub between capturing the trace tail and appending it to RESULT:

```bash
# --- Diagnostic trace tail (mika#887) ---
if [ -f "$TRACE_FILE" ]; then
    case "$RESULT" in
        "HANDLER CRASH"*)
            # Crash path: append trace tail, preserve file for forensics
            _TRACE_TAIL=$(tail -50 "$TRACE_FILE" 2>/dev/null \
                | sed -E 's/(GH_APP_TOKEN|GH_TOKEN|MIKA_[A-Z_]*TOKEN|MIKA_[A-Z_]*API_KEY|MIKA_[A-Z_]*PRIVATE_KEY)=[^ ]*/\1=<REDACTED>/g' \
                | sed -E 's/github_pat_[A-Za-z0-9_]+/<REDACTED_PAT>/g' \
                | sed -E 's/ghp_[A-Za-z0-9]+/<REDACTED_PAT>/g' \
                | sed 's/^/    /')
            if [ -n "$_TRACE_TAIL" ]; then
                RESULT="${RESULT}

Trace tail (last 50 lines):
${_TRACE_TAIL}"
            fi
            ;;
        *)
            # Success/recovery path: clean up trace file
            rm -f "$TRACE_FILE"
            ;;
    esac
fi
```

**Pattern coverage:**
- `GH_APP_TOKEN=...` — the primary leak vector
- `GH_TOKEN=...` — if gh CLI env var is set
- `MIKA_*TOKEN=...` — `MIKA_INTERNAL_TOKEN`, `MIKA_GITHUB_TOKEN`, etc.
- `MIKA_*API_KEY=...` — `MIKA_ANTHROPIC_API_KEY`, `MIKA_BRAVE_API_KEY`, etc.
- `MIKA_*PRIVATE_KEY=...` — `MIKA_GITHUB_APP_PRIVATE_KEY`
- `github_pat_...` — PAT literal (defense in depth, catches `echo $TOKEN` expansion)
- `ghp_...` — classic GitHub PAT prefix

### Step 4: Also scrub stderr tail in the crash path (lines ~38-45)

The stderr capture (line 38-45) also goes into RESULT. Apply the same scrub:

```bash
if [ -z "$RESULT" ] && [ -n "$STDERR_FILE" ] && [ -f "$STDERR_FILE" ]; then
    _STDERR_TAIL=$(tail -c 10000 "$STDERR_FILE" 2>/dev/null \
        | sed -E 's/(GH_APP_TOKEN|GH_TOKEN|MIKA_[A-Z_]*TOKEN|MIKA_[A-Z_]*API_KEY|MIKA_[A-Z_]*PRIVATE_KEY)=[^ ]*/\1=<REDACTED>/g' \
        | sed -E 's/github_pat_[A-Za-z0-9_]+/<REDACTED_PAT>/g' \
        | sed -E 's/ghp_[A-Za-z0-9]+/<REDACTED_PAT>/g')
    if [ -n "$_STDERR_TAIL" ]; then
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result.

Stderr (last 10KB):
${_STDERR_TAIL}"
    fi
fi
```

### Step 5: Extract scrub function to avoid duplication

Steps 3 and 4 share the same sed pipeline. Extract to a helper:

```bash
_scrub_secrets_from_output() {
    # Redact known secret patterns from diagnostic output before callback delivery (mika#903).
    sed -E 's/(GH_APP_TOKEN|GH_TOKEN|MIKA_[A-Z_]*TOKEN|MIKA_[A-Z_]*API_KEY|MIKA_[A-Z_]*PRIVATE_KEY)=[^ ]*/\1=<REDACTED>/g' \
        | sed -E 's/github_pat_[A-Za-z0-9_]+/<REDACTED_PAT>/g' \
        | sed -E 's/ghp_[A-Za-z0-9]+/<REDACTED_PAT>/g'
}
```

Then both call sites pipe through `_scrub_secrets_from_output`.

### Step 6: Test fixture

Create `skills/bundled/_shared/test-trace-redaction.sh`:

```bash
#!/bin/bash
# Test fixture: verifies that _scrub_secrets_from_output redacts known patterns.
# Run: bash skills/bundled/_shared/test-trace-redaction.sh
# Exit 0 on success, 1 on failure.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/dispatch-lib.sh" 2>/dev/null || true  # source the helper

# Simulate trace lines containing secrets
TEST_INPUT="+ GH_APP_TOKEN=github_pat_abc123XYZ_secret_value
+ echo github_pat_abc123XYZ_secret_value
+ MIKA_ANTHROPIC_API_KEY=sk-ant-api03-very-secret
+ MIKA_INTERNAL_TOKEN=internal-secret-token
+ ghp_1234567890abcdef
+ normal_command arg1 arg2"

EXPECTED_PATTERNS=(
    "GH_APP_TOKEN=<REDACTED>"
    "<REDACTED_PAT>"
    "MIKA_ANTHROPIC_API_KEY=<REDACTED>"
    "MIKA_INTERNAL_TOKEN=<REDACTED>"
    "normal_command arg1 arg2"
)

FORBIDDEN_PATTERNS=(
    "github_pat_abc123"
    "sk-ant-api03"
    "internal-secret-token"
    "ghp_1234567890"
)

RESULT=$(echo "$TEST_INPUT" | _scrub_secrets_from_output)

PASS=true
for pattern in "${EXPECTED_PATTERNS[@]}"; do
    if ! echo "$RESULT" | grep -qF "$pattern"; then
        echo "FAIL: expected pattern not found: $pattern"
        PASS=false
    fi
done

for pattern in "${FORBIDDEN_PATTERNS[@]}"; do
    if echo "$RESULT" | grep -qF "$pattern"; then
        echo "FAIL: secret not redacted: $pattern"
        PASS=false
    fi
done

if [ "$PASS" = true ]; then
    echo "PASS: all secret patterns redacted correctly"
    exit 0
else
    echo "FAIL: some patterns were not handled"
    echo "--- Output was ---"
    echo "$RESULT"
    exit 1
fi
```

## Change summary

| File | Change |
|------|--------|
| `skills/bundled/_shared/dispatch-lib.sh` | Add `_scrub_secrets_from_output()` helper; wrap `_setup_gh_auth` with `set +x`/`set -x`; pipe stderr and trace tails through scrub; create trace file with `umask 077` |
| `skills/bundled/_shared/test-trace-redaction.sh` | New test fixture verifying redaction |

## Risk assessment

- **Low risk:** All changes are in shell scripts, not Rust. The `set +x`/`set -x` toggle is surgical and self-contained within `_setup_gh_auth`.
- **No behavioral change:** The dispatch flow, callback delivery, and diagnostic trace all continue to work — only secret values are replaced with `<REDACTED>`.
- **Defense in depth:** Even if `set +x` fails (shouldn't happen), the EXIT trap scrub catches secrets before they reach mika.db.
- **Regex coverage gap:** Custom-format secrets without `MIKA_`/`GH_`/`github_pat_`/`ghp_` prefixes would not be caught. Acceptable — the known variable names are enumerated, and `_scrub_env` already unsets them before the main work begins (line 158-160). The exposure window is only during `_setup_gh_auth`.

## Verification

1. `bash skills/bundled/_shared/test-trace-redaction.sh` — passes
2. Manual: trigger a handler crash after `_setup_gh_auth` → inspect `/tmp/dev-pilot-trace-*.log` → no plaintext PAT
3. Manual: inspect trace file permissions → `-rw-------` (0600)
4. `ls -la /tmp/dev-pilot-trace-*.log` after a normal dispatch → file cleaned up on success path (existing behavior, unchanged)
