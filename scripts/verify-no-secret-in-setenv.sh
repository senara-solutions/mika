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
# Three rules, in order:
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
#   3. PROVENANCE of each exempted VARIABLE (mika#2716). Rule 2 accepts a value
#      passed through one closed-list variable as well as through the literal
#      placeholder, because the egress sentinel is drawn per spawn and cannot be
#      a literal. A variable, unlike a literal, cannot be inspected by value —
#      so the concession is paid for by three grep-visible facts about where
#      that variable's value may come from, and where it may go. Without them,
#      `--setenv GH_TOKEN "$ANY_VARIABLE"` would pass rule 2 unexamined.
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

# --- Named exceptions for rule 2 -------------------------------------------
#
# Each name below is passed by `--setenv` but carries a literal placeholder:
# the real credential is injected host-side by the egress proxy and never
# crosses the bwrap boundary. The client only needs SOME local value so it
# does not short-circuit before sending the request the proxy authenticates.
#
#   ANTHROPIC_API_KEY — Claude Code prints "Not logged in" without it (see the
#                       Q3 comment in dispatch-lib.sh).
#   GH_TOKEN          — the gh CLI stops locally on `gh auth login` without it,
#                       before any network call (mika#2572). The GitHub addon
#                       strips and replaces Authorization host-side (mika#2056).
#
# The exception is conditional and per-occurrence: it holds only while EVERY
# `--setenv <NAME>` carries the placeholder. Swapping in a real value re-opens
# mika#2039 and fails this lint. The list is closed: the placeholder value does
# not exempt any other credential-shaped name.
EXEMPT_SETENV_NAMES=(ANTHROPIC_API_KEY GH_TOKEN)
EXEMPT_SETENV_VALUE="proxy-managed-no-secret"

# mika#2716: GH_TOKEN no longer carries a literal. It carries the per-spawn
# EGRESS SENTINEL `<placeholder>-<32 hex>`, which the host draws and registers
# so the egress proxy can tell a pilot's own `gh` from a `cargo test` that
# overwrote GH_TOKEN with a fake (mika#2713). A per-spawn value cannot be a
# literal, so rule 2 also accepts EXACTLY `"$<VAR>"` for a VAR on this closed
# list — one accepted spelling, no braced variant, no other variable.
#
# A variable is a hole in a by-value lint, and rule 3 below is the patch: each
# VAR here must have a producer function, and the value may neither come from a
# credential nor become one.
EXEMPT_SETENV_VARS=(_PILOT_EGRESS_SENTINEL)
EXEMPT_SETENV_VAR_PRODUCERS=(_stage_pilot_egress_sentinel)

# The credential names rule 3 keeps away from the exempted variables, in both
# directions. `_PILOT_GH_TOKEN_FILE` is included because it names the host file
# the egress addon reads the real token from: a producer that reads it is a
# producer that can copy the token into the argv.
EXEMPT_SETENV_VAR_CREDENTIALS='GH_TOKEN|MIKA_GITHUB_TOKEN|_PILOT_GH_TOKEN_FILE'

is_exempt_name() {
    local candidate="$1" n
    for n in "${EXEMPT_SETENV_NAMES[@]}"; do
        [[ "$candidate" == "$n" ]] && return 0
    done
    return 1
}

# ERE alternation of every value rule 2 accepts for an exempted NAME: the
# literal placeholder, or exactly one of the exempted variables.
exempt_value_alternation() {
    local alt v
    alt="\"$EXEMPT_SETENV_VALUE\""
    for v in "${EXEMPT_SETENV_VARS[@]}"; do
        alt="$alt|\"\\\$$v\""
    done
    printf '(%s)' "$alt"
}

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
    if is_exempt_name "$name"; then
        # Per-occurrence, not file-global: a whole-file `grep -q` for the
        # placeholder would still pass if a SECOND `--setenv <NAME>` carrying a
        # real value were added alongside it. Every occurrence must carry the
        # placeholder. The name is bounded on the right (space or end of line)
        # so GH_TOKEN never also counts a hypothetical GH_TOKEN_EXTRA, while a
        # `--setenv GH_TOKEN` whose value sits on the next array line is still
        # counted — and then fails, since it does not carry the placeholder.
        # Both counts are OCCURRENCES (`grep -o | wc -l`), never lines
        # (`grep -c`): a decoy placeholder and a real value on the same line
        # are two occurrences. The accepted value's closing quote is bounded, so
        # `"proxy-managed-no-secret""$GH_TOKEN"` is not counted as conformant.
        #
        # mika#2716 widened the accepted set from the literal placeholder alone
        # to the placeholder OR an exempted variable. Both spellings are counted
        # here together, because the live file and a fixture may legitimately
        # carry one each: scripts/test-verify-no-secret-in-setenv.sh plants the
        # literal when the source no longer has it, so the fixture holds two
        # occurrences and only a rule accepting both keeps that case green.
        _total=$({ grep -oE -- "--setenv[[:space:]]+$name([[:space:]]|\$)" "$CODE_ONLY" || true; } | wc -l)
        _conformes=$({ grep -oE -- "--setenv[[:space:]]+${name}[[:space:]]+$(exempt_value_alternation)([[:space:]]|\$)" "$CODE_ONLY" || true; } | wc -l)
        _total=$((_total)); _conformes=$((_conformes))
        if [[ "$_total" -eq "$_conformes" && "$_total" -ge 1 ]]; then
            continue
        fi
        echo "VIOLATION: $_total --setenv $name occurrence(s), only"
        echo "  $_conformes carrying the placeholder '$EXEMPT_SETENV_VALUE'"
        echo "  or one of the exempted variables: ${EXEMPT_SETENV_VARS[*]}."
        echo "  Its exemption held only because the real credential is injected"
        echo "  host-side by the egress proxy. A real value here would be"
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

# --- Rule 3: provenance of each exempted variable (mika#2716) --------------
#
# Three facts, each a single grep, together re-establishing statically what the
# literal placeholder gave for free — the value reaching the argv is DRAWN,
# never copied from a credential, and never becomes one:
#
#   3a. no line that sets the variable may mention a credential name. This is
#       the direct data-flow statement and it holds wherever the assignment
#       lives, so `_PILOT_EGRESS_SENTINEL="$GH_TOKEN"` fails from anywhere in
#       the file, not only from inside the producer.
#   3b. the producer function must exist, must set the variable at least once,
#       and its body must name no credential. This pins the producer as the
#       real source and forbids it reading the host token file "to derive
#       something the proxy can match".
#   3c. no line that assigns a credential name may mention the variable. This
#       is the one failure mode worse than the 403 the ticket guards against:
#       `export GH_TOKEN="$_PILOT_EGRESS_SENTINEL"` host-side would have
#       _stage_pilot_gh_token stage the SENTINEL as the real token, the addon
#       would inject it, and every pilot call would 401 — a refusal that comes
#       from GitHub and names nothing of mika#2716. Rule 3a cannot see it: that
#       line assigns the credential, not the variable.
#
# A function body, extracted from the code-only view: from `name() {` to the
# first closing brace in column 0.
extract_function_body() {
    awk -v fn="$1" '
        $0 ~ "^" fn "\\(\\)[[:space:]]*\\{" { inside = 1; next }
        inside && /^\}/                     { inside = 0 }
        inside                              { print }
    ' "$CODE_ONLY"
}

for _i in "${!EXEMPT_SETENV_VARS[@]}"; do
    _var="${EXEMPT_SETENV_VARS[$_i]}"
    _producer="${EXEMPT_SETENV_VAR_PRODUCERS[$_i]}"
    _set_re="(^|[[:space:]]|;|\\\$\\{)${_var}(=|:=)"

    # 3a — every setting of the variable is credential-free.
    while IFS= read -r _line; do
        [[ -z "$_line" ]] && continue
        echo "VIOLATION: $_var is set from a credential, at line ${_line%%:*}."
        echo "  The egress sentinel is exempted from rule 2 only because it is"
        echo "  DRAWN, never copied. Seeding it from a token makes it the token"
        echo "  it stands for, in an argv any local user can read (mika#2716)."
        VIOLATIONS=$((VIOLATIONS + 1))
    done < <({ grep -nE -- "$_set_re" "$CODE_ONLY" || true; } \
                 | { grep -E -- "$EXEMPT_SETENV_VAR_CREDENTIALS" || true; })

    # 3b — the producer exists, sets the variable, and names no credential.
    _body="$(extract_function_body "$_producer")"
    if [[ -z "$_body" ]]; then
        echo "VIOLATION: $_var is exempted from rule 2 but its producer"
        echo "  $_producer() was not found in $TARGET. The exemption is on the"
        echo "  variable NAME, so it survives the deletion of what fills it —"
        echo "  which is exactly when it would wave a credential through."
        VIOLATIONS=$((VIOLATIONS + 1))
    else
        if ! grep -qE -- "$_set_re" <<<"$_body"; then
            echo "VIOLATION: $_producer() never sets $_var, yet rule 2 exempts"
            echo "  that variable on the strength of this function (mika#2716)."
            VIOLATIONS=$((VIOLATIONS + 1))
        fi
        if grep -qE -- "$EXEMPT_SETENV_VAR_CREDENTIALS" <<<"$_body"; then
            echo "VIOLATION: $_producer() names a credential"
            echo "  ($EXEMPT_SETENV_VAR_CREDENTIALS)."
            echo "  The producer of an exempted --setenv value must not be able"
            echo "  to read one; drawing is the whole exemption (mika#2716)."
            VIOLATIONS=$((VIOLATIONS + 1))
        fi
    fi

    # 3c — the variable never flows INTO a credential name, host-side.
    while IFS= read -r _line; do
        [[ -z "$_line" ]] && continue
        echo "VIOLATION: a credential is assigned from $_var, at line ${_line%%:*}."
        echo "  Exporting the sentinel into the host's GH_TOKEN would have"
        echo "  _stage_pilot_gh_token stage it AS the real token: the addon"
        echo "  would inject it and every pilot call would 401 (mika#2716 R7)."
        VIOLATIONS=$((VIOLATIONS + 1))
    done < <({ grep -nE -- "(^|[[:space:]]|;|\\\$\\{)($EXEMPT_SETENV_VAR_CREDENTIALS)(=|:=)" "$CODE_ONLY" || true; } \
                 | { grep -F -- "$_var" || true; })
done

if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo ""
    echo "$VIOLATIONS violation(s). See mika#2039."
    exit 1
fi

echo "verify-no-secret-in-setenv: clean — no secret reaches the sandbox argv."
exit 0
