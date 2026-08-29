#!/usr/bin/env bash
# CI lint (mika#2039): no secret may reach the pilot sandbox through a bwrap
# `--setenv` argument.
#
# `--setenv NAME VALUE` puts VALUE in bwrap's argv, and /proc/<pid>/cmdline is
# world-readable — `pgrep -af claude-pilot` is enough to read it. Secrets must
# travel through the file-descriptor channel instead (`--perms 0600
# --ro-bind-data`), which materialises them as a 0600 file inside the sandbox.
#
# Discipline analog: `scripts/verify-egress-no-log.sh` (#1810) and
# `scripts/verify-egress-uniqueness.sh` (#1807) — construct the incapacity,
# don't promise the restraint.
#
# This is the NAME guard. Its companion is the VALUE guard at
# `skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh`, which runs
# the argv construction against a mocked bwrap. Neither subsumes the other:
# this one catches a credential-shaped variable entering the allowlist, that
# one catches a regression of the channel itself through a path that adds
# nothing to the allowlist.
#
# Two rules, in order:
#
#   1. DENY-BY-DEFAULT (primary). `_PILOT_SANDBOX_ENV_ALLOWLIST` must equal the
#      expected literal set recorded below. Any addition, removal, or rename
#      fails — regardless of how the new name looks. This is the same posture
#      as the `--clearenv` invariant the sandbox already relies on: a pattern
#      denylist would wave through `SENTRY_DSN`, `..._CREDENTIAL`, `..._AUTH`,
#      or a basic-auth URL.
#
#   2. NAME-SHAPE NET (secondary). Every literal `--setenv <NAME>` in the file
#      — which covers the `net_setenv_args` producer, outside the allowlist —
#      is rejected when NAME looks like a credential. `PAT` is matched
#      delimited, not as a substring, so the legitimate `PATH` entry passes.
#
# Exit codes:
#   0 — clean
#   1 — violation(s) found, or the source file could not be located
#
# Usage: verify-no-secret-in-setenv.sh [path-to-dispatch-lib.sh]
#   The optional argument exists for scripts/test-verify-no-secret-in-setenv.sh,
#   which runs this lint against fixture copies. CI passes no argument.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-$REPO_ROOT/skills/bundled/_shared/dispatch-lib.sh}"

# --- Expected allowlist (single source of truth for rule 1) ----------------
#
# Adding a name here is a deliberate act: confirm the variable carries no
# credential material, note why in the audit comment above
# `_PILOT_SANDBOX_ENV_ALLOWLIST` in dispatch-lib.sh, then update this set.
EXPECTED_ENV_ALLOWLIST=(
    ANTHROPIC_LOG_FILE
    HOME
    HOSTNAME
    LANG
    LC_ALL
    LOGNAME
    MIKA_LOG_PILOT_TRANSCRIPTS
    PATH
    SHELL
    TERM
    TMPDIR
    USER
)

# --- Named exception for rule 2 --------------------------------------------
#
# `ANTHROPIC_API_KEY` is passed by name but carries a literal placeholder: the
# real key is injected host-side by the egress proxy and never crosses the
# bwrap boundary (see the Q3 comment in dispatch-lib.sh). The exception is
# conditional — it holds only while the placeholder is what is actually
# written. Swapping in a real key re-opens mika#2039 and fails this lint.
EXEMPT_SETENV_NAME="ANTHROPIC_API_KEY"
EXEMPT_SETENV_VALUE="proxy-managed-no-secret"

CRED_NAME_PATTERN='TOKEN|SECRET|KEY|PASSWORD|PASSWD|(^|_)PAT(_|$)'

if [[ ! -f "$TARGET" ]]; then
    echo "ERROR: dispatch-lib.sh not found at $TARGET"
    echo "The pilot sandbox argv construction is expected at this path. If the"
    echo "file moved, update TARGET in $0 and re-run — do not delete this lint."
    exit 1
fi

VIOLATIONS=0

# Code-only view of the file. Every rule below greps this, never $TARGET
# directly: a future comment such as `never write --setenv GH_TOKEN` would
# otherwise fail CI on prose, and prose is where this file explains itself.
CODE_ONLY="$(mktemp "${TMPDIR:-/tmp}/verify-no-secret-in-setenv-code.XXXXXX")"
trap 'rm -f "$CODE_ONLY"' EXIT
sed -E 's/(^|[[:space:]])#.*$//' "$TARGET" > "$CODE_ONLY"

# Extract a bash array literal's contents by name: everything between
# `NAME=(` and the first closing `)` on its own line.
#
# This parser is deliberately narrow, and Rule 0b below makes that narrowness
# safe: it rejects any array form this function cannot model, rather than
# quietly returning a partial list. Deny-by-default only denies what the
# parser can see — an append on the next line would otherwise be invisible.
extract_array() {
    local name="$1"
    awk -v n="$name" '
        $0 ~ "^" n "=\\(" { inside = 1; next }
        inside && /^\)/    { inside = 0; exit }
        inside             { print }
    ' "$CODE_ONLY" | tr -s ' \t' '\n' | grep -v '^$' || true
}

# --- Rule 0b: the arrays must appear in exactly the one form we can parse --
for _arr in _PILOT_SANDBOX_ENV_ALLOWLIST _PILOT_SANDBOX_SECRET_ALLOWLIST; do
    _opens=$(grep -cE "^${_arr}=\\(" "$CODE_ONLY" || true)
    _touches=$(grep -cE "^[[:space:]]*(declare[[:space:]]+-[a-zA-Z]+[[:space:]]+)?${_arr}\\+?=" "$CODE_ONLY" || true)
    if [[ "$_opens" -ne 1 || "$_touches" -ne 1 ]]; then
        echo "VIOLATION: $_arr is written in a form this lint cannot audit."
        echo "  Found $_opens plain \`NAME=(\` opener(s) and $_touches total"
        echo "  assignment(s). Exactly one of each is required."
        echo "  An append (\`NAME+=(...)\`), a second assignment, a one-line"
        echo "  literal, or a \`declare\` form would leave part of the list"
        echo "  invisible to the audit — and an invisible entry is how a"
        echo "  secret gets back into the world-readable argv (mika#2039)."
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done

# --- Rule 0c: every `--setenv` must name a literal we can audit ------------
#
# Rule 2's name check is a text scan, so `--setenv "$var"` and a backslash
# line-continuation before the name are both invisible to it. Exactly one
# dynamic producer is sanctioned — the audited allowlist loop — and it is
# pinned by count. Anything else fails closed.
_DYNAMIC_SANCTIONED='setenv_args+=(--setenv "$var" "${!var}")'
_dynamic_count=$(grep -cF -- "$_DYNAMIC_SANCTIONED" "$CODE_ONLY" || true)
if [[ "$_dynamic_count" -ne 1 ]]; then
    echo "VIOLATION: expected exactly one sanctioned dynamic --setenv producer,"
    echo "  found $_dynamic_count occurrence(s) of:"
    echo "    $_DYNAMIC_SANCTIONED"
    echo "  A second loop over a different array would emit --setenv names this"
    echo "  lint cannot see. Route its secrets through"
    echo "  _PILOT_SANDBOX_SECRET_ALLOWLIST instead (mika#2039)."
    VIOLATIONS=$((VIOLATIONS + 1))
fi

while IFS= read -r _line; do
    [[ -z "$_line" ]] && continue
    if [[ "$_line" == *"$_DYNAMIC_SANCTIONED"* ]]; then
        continue
    fi
    echo "VIOLATION: a --setenv whose name is not a bare literal cannot be audited:"
    echo "    $_line"
    echo "  Use a literal name, or route the value through"
    echo "  _PILOT_SANDBOX_SECRET_ALLOWLIST (mika#2039)."
    VIOLATIONS=$((VIOLATIONS + 1))
done < <(grep -nE -- '--setenv([[:space:]]*\\$|[[:space:]]+["'"'"'$]|[[:space:]]*$)' "$CODE_ONLY" || true)

# --- Rule 0: the secret allowlist must exist -------------------------------
if ! grep -q '^_PILOT_SANDBOX_SECRET_ALLOWLIST=(' "$CODE_ONLY"; then
    echo "VIOLATION: _PILOT_SANDBOX_SECRET_ALLOWLIST is missing from $TARGET"
    echo "  Secrets must be declared there and delivered through the"
    echo "  --ro-bind-data file channel, never through --setenv (mika#2039)."
    VIOLATIONS=$((VIOLATIONS + 1))
fi

# --- Rule 1: deny-by-default on the --setenv allowlist ---------------------
ACTUAL_SORTED="$(extract_array _PILOT_SANDBOX_ENV_ALLOWLIST | sort -u)"
EXPECTED_SORTED="$(printf '%s\n' "${EXPECTED_ENV_ALLOWLIST[@]}" | sort -u)"

if [[ "$ACTUAL_SORTED" != "$EXPECTED_SORTED" ]]; then
    echo "VIOLATION: _PILOT_SANDBOX_ENV_ALLOWLIST does not match the audited set."
    while IFS= read -r name; do
        [[ -z "$name" ]] && continue
        echo "  + added, not audited: $name"
    done < <(comm -13 <(printf '%s\n' "$EXPECTED_SORTED") <(printf '%s\n' "$ACTUAL_SORTED"))
    while IFS= read -r name; do
        [[ -z "$name" ]] && continue
        echo "  - removed: $name"
    done < <(comm -23 <(printf '%s\n' "$EXPECTED_SORTED") <(printf '%s\n' "$ACTUAL_SORTED"))
    echo "  Every value in this list lands in bwrap's argv, readable by any"
    echo "  local user. If the addition carries no credential material, record"
    echo "  why in the audit comment in dispatch-lib.sh, then update"
    echo "  EXPECTED_ENV_ALLOWLIST in $0. If it does, move it to"
    echo "  _PILOT_SANDBOX_SECRET_ALLOWLIST instead."
    VIOLATIONS=$((VIOLATIONS + 1))
fi

# --- Rule 1b: a secret must not appear in both lists -----------------------
while IFS= read -r secret; do
    [[ -z "$secret" ]] && continue
    # Herestring, not a pipe: `grep -q` exits at the first match and closes the
    # pipe, `printf` takes SIGPIPE, and `pipefail` promotes 141 to the
    # pipeline's status — reporting "absent" for a value that is present.
    # See docs/solutions/test-failures/bash-assert-sigpipe-and-host-coupling-before-ci-gate-2026-08-29.md
    if grep -qx -- "$secret" <<<"$ACTUAL_SORTED"; then
        echo "VIOLATION: '$secret' is in BOTH the --setenv allowlist and the"
        echo "  secret allowlist. The --setenv copy puts it back in the argv."
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done < <(extract_array _PILOT_SANDBOX_SECRET_ALLOWLIST)

# --- Rule 2: name-shape net over every literal --setenv --------------------
while IFS= read -r name; do
    [[ -z "$name" ]] && continue
    if [[ "$name" == "$EXEMPT_SETENV_NAME" ]]; then
        # Per-occurrence, not file-global: a whole-file `grep -q` for the
        # placeholder would still pass if a SECOND `--setenv ANTHROPIC_API_KEY`
        # carrying a real key were added alongside it. Every occurrence must
        # carry the placeholder.
        _total=$(grep -cE -- "--setenv[[:space:]]+$EXEMPT_SETENV_NAME" "$CODE_ONLY" || true)
        _placeheld=$(grep -cF -- "--setenv $EXEMPT_SETENV_NAME \"$EXEMPT_SETENV_VALUE\"" "$CODE_ONLY" || true)
        if [[ "$_total" -eq "$_placeheld" && "$_total" -ge 1 ]]; then
            continue
        fi
        echo "VIOLATION: $_total --setenv $EXEMPT_SETENV_NAME occurrence(s), only"
        echo "  $_placeheld carrying the placeholder '$EXEMPT_SETENV_VALUE'."
        echo "  Its exemption held only because the real key is injected"
        echo "  host-side by the egress proxy. A real key here would be"
        echo "  readable in the sandbox argv by any local user (mika#2039)."
        VIOLATIONS=$((VIOLATIONS + 1))
        continue
    fi
    if [[ "$name" =~ $CRED_NAME_PATTERN ]]; then
        echo "VIOLATION: --setenv $name looks like a credential."
        echo "  Deliver it through _PILOT_SANDBOX_SECRET_ALLOWLIST and the"
        echo "  --ro-bind-data file channel instead (mika#2039)."
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done < <(grep -oE -- '--setenv [A-Za-z_][A-Za-z0-9_]*' "$CODE_ONLY" | awk '{print $2}' | sort -u)

if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo ""
    echo "$VIOLATIONS violation(s). See mika#2039."
    exit 1
fi

echo "verify-no-secret-in-setenv: clean — no secret reaches the sandbox argv."
exit 0
