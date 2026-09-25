#!/usr/bin/env bash
# Anti-vacuity harness for scripts/check-sandboxed-handler-env.sh (mika#2532).
#
# A guard nobody has watched go red is a decoration. This drives the scan over
# fixture trees and asserts it goes RED on each shape it exists to catch, GREEN
# on each shape it must not accuse, and RED when it has nothing to scan.
#
# The good-faith cases are the load-bearing half: the four handlers the ticket
# repaired all EXPLAIN in a comment why they no longer read the variable, so a
# scan that accused comments would be permanently red on the very tree it was
# written for — and would be disarmed at the first inconvenience.
#
# Run: bash scripts/test-check-sandboxed-handler-env.sh
# Expected: all cases pass, exit 0.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-sandboxed-handler-env.sh"

PASS=0
FAIL=0

TMPROOT="$(mktemp -d /tmp/sandboxed-handler-env-test-XXXXXX)"
trap 'rm -rf "$TMPROOT"' EXIT

ok() { PASS=$((PASS + 1)); echo "  ✓ $1"; }
ko() {
    FAIL=$((FAIL + 1))
    echo "  ✗ $1"
    shift
    for line in "$@"; do echo "    $line"; done
}

# `mktemp -d` rather than a counter: `make_tree` is called inside a command
# substitution, so a `CASE_N=$((CASE_N + 1))` would be incremented in a subshell
# and lost — every fixture would land in the same tree and the cases would
# contaminate each other.
make_tree() {
    local root
    root="$(mktemp -d "$TMPROOT/tree-XXXXXX")"
    mkdir -p "$root/skills/bundled"
    printf '%s' "$root"
}

#   add_handler <root> <skill> <body>
add_handler() {
    local root="$1" skill="$2" body="$3"
    mkdir -p "$root/skills/bundled/$skill/handlers"
    printf '%s\n' "$body" >"$root/skills/bundled/$skill/handlers/run.sh"
}

expect_red() {
    local root="$1" label="$2"
    if bash "$GUARD" "$root" >/dev/null 2>&1; then
        ko "$label — expected RED, got green"
    else
        ok "$label"
    fi
}

expect_green() {
    local root="$1" label="$2"
    local out
    if out="$(bash "$GUARD" "$root" 2>&1)"; then
        ok "$label"
    else
        ko "$label — expected GREEN, got red" "$out"
    fi
}

echo "mika#2532 — check-sandboxed-handler-env: negative controls"
echo

# ── N1: the measured shape ──────────────────────────────────────────────────
root="$(make_tree)"
add_handler "$root" build-mika '#!/bin/sh
_DEFAULT="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}/mika"'
expect_red "$root" "N1 — the measured \${MIKA_PLATFORM_DIR:-…} read is refused"

# ── N2: any other MIKA_* name, not just the measured one ────────────────────
#
# Without this the scan could have been written against one variable and would
# read as a class guard while covering a single line.
root="$(make_tree)"
add_handler "$root" deploy-mika '#!/bin/sh
LOG="${MIKA_SOME_FUTURE_SETTING:-/var/log/x}"'
expect_red "$root" "N2 — a NEW MIKA_* name is refused too (it is a class, not a line)"

# ── N3: a bare read with no default ─────────────────────────────────────────
root="$(make_tree)"
add_handler "$root" self-check '#!/bin/sh
echo "${MIKA_INTERNAL_TOKEN}"'
expect_red "$root" "N3 — a read with no default is refused as well"

# ── N4: an indented read ────────────────────────────────────────────────────
#
# The comment filter anchors on `#` after leading whitespace; an indented CODE
# line must not be swept up by it.
root="$(make_tree)"
add_handler "$root" build-mika '#!/bin/sh
if true; then
    D="${MIKA_PLATFORM_DIR:-/tmp}"
fi'
expect_red "$root" "N4 — an indented read is still refused"

# ── G1: good faith — a comment EXPLAINING the removed read is not a read ────
#
# This is the shape the four repaired handlers actually carry. Accusing it
# would make the scan permanently red on the tree it was written for.
root="$(make_tree)"
add_handler "$root" build-mika '#!/bin/sh
# `PLATFORM_DIR` replaces `${MIKA_PLATFORM_DIR:-…}`, which was a dead branch.
_DEFAULT="${PLATFORM_DIR:-$HOME/workspace/mika-platform}/mika"'
expect_green "$root" "G1 — a comment mentioning the old read is not accused"

# ── G2: good faith — the unprefixed relay, and an unrelated MIKA_ literal ───
root="$(make_tree)"
add_handler "$root" deploy-mika '#!/bin/sh
for _var in $(env | grep -o "^MIKA_[^=]*"); do unset "$_var"; done
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN
PLATFORM_DIR="${PLATFORM_DIR:-$HOME/workspace/mika-platform}"'
expect_green "$root" "G2 — the scrub loop and the unprefixed relay are not reads"

# ── V: the scan refuses to pass on an empty population ──────────────────────
root="$(make_tree)"
expect_red "$root" "V — an empty population is RED, not a clean tree (mika#2205)"

# ── The real tree must be green, and it must have scanned something ─────────
out="$(bash "$GUARD" 2>&1)"
rc=$?
if [[ $rc -eq 0 ]] && [[ "$out" == *"handlers scanned"* ]] && [[ "$out" != *"0 handlers scanned"* ]]; then
    ok "the repository tree is clean, and the scan says how many handlers it read"
else
    ko "expected the real tree to be green with a non-empty population" "$out"
fi

echo
echo "  passed: $PASS   failed: $FAIL"
[[ "$FAIL" -eq 0 ]] || exit 1
