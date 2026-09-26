#!/usr/bin/env bash
# Anti-vacuity harness for scripts/check-pilot-push-sites.sh (mika#2520).
#
# A guard nobody has watched go red is a decoration. This drives the scan over
# fixture trees and asserts it goes RED on each shape it exists to catch, GREEN
# on each shape it must not accuse, and RED when it has nothing to scan.
#
# The good-faith controls (N6, N7, and the comment case) are the load-bearing
# half: without them a scan that had become permanently red would be disarmed at
# the first inconvenience, and one that had become permanently green would read
# exactly like a clean tree.
#
# Run: bash scripts/test-check-pilot-push-sites.sh
# Expected: all cases pass, exit 0.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-pilot-push-sites.sh"

PASS=0
FAIL=0

TMPROOT="$(mktemp -d /tmp/pilot-push-scan-test-XXXXXX)"
trap 'rm -rf "$TMPROOT"' EXIT

ok() { PASS=$((PASS + 1)); echo "  ✓ $1"; }
ko() {
    FAIL=$((FAIL + 1))
    echo "  ✗ $1"
    shift
    for line in "$@"; do echo "    $line"; done
}

# Builds a fixture tree and returns its root on stdout.
#   make_tree <name>
make_tree() {
    local root="$TMPROOT/$1"
    mkdir -p "$root/skills/bundled" "$root/scripts"
    printf '%s' "$root"
}

#   add_handler <root> <skill> <body>
add_handler() {
    local root="$1" skill="$2" body="$3"
    mkdir -p "$root/skills/bundled/$skill/handlers"
    printf '%s\n' "$body" > "$root/skills/bundled/$skill/handlers/run.sh"
}

#   expect <label> <expected-rc> <root>
expect() {
    local label="$1" expected="$2" root="$3"
    local out rc=0
    out="$(bash "$GUARD" "$root" 2>&1)" || rc=$?
    if [ "$rc" -eq "$expected" ]; then
        ok "$label (rc=$rc)"
    else
        ko "$label" "expected rc: $expected" "actual rc:   $rc" "output:" "$out"
    fi
}

# ── N4 — a bare `git push` in a handler is refused ───────────────────────────
echo "N4: bare git push"
root="$(make_tree n4)"
add_handler "$root" "some-skill" 'PROMPT="5. Push with: git push"'
expect "N4 — a bare git push goes RED" 1 "$root"

# ── N5 — the "nearly right" form gets NO special case ────────────────────────
#
# A handler writing the lease inline is still a second push site: the refusals
# live in the shared helper, not in the argv.
echo "N5: bare force-with-lease"
root="$(make_tree n5)"
add_handler "$root" "some-skill" 'PROMPT="6. Push: git push --force-with-lease"'
expect "N5 — a bare --force-with-lease goes RED" 1 "$root"

root="$(make_tree n5b)"
add_handler "$root" "some-skill" 'git push --force-with-lease=b:sha origin HEAD:refs/heads/b'
expect "N5b — even a fully-formed inline push goes RED" 1 "$root"

# ── The option-carrying form is covered too ──────────────────────────────────
#
# `git -C "$WT" push` is the shape the guarded helper itself uses, so a predicate
# reading only the literal `git push` would leave the obvious bypass open.
echo "option-carrying invocations"
root="$(make_tree opts)"
add_handler "$root" "some-skill" 'git -C "$WORKTREE" push origin HEAD'
expect "git -C <dir> push goes RED" 1 "$root"

# ── N6 — good faith: a handler with no push is accepted ──────────────────────
echo "N6: clean handler"
root="$(make_tree n6)"
add_handler "$root" "some-skill" 'PROMPT="1. git fetch origin
2. git rebase origin/main
3. Stop there."'
expect "N6 — a handler that does not push goes GREEN" 0 "$root"

# ── Good faith: a COMMENT explaining the absence must not accuse ─────────────
#
# Comment lines are stripped before the examination, which is what lets a
# handler's header explain why it no longer pushes (this repo's own handler does
# exactly that).
echo "comments"
root="$(make_tree comments)"
add_handler "$root" "some-skill" '# Until mika#2520 this ran a bare git push --force-with-lease.
#     6. Push: git push
PROMPT="Stop there."'
expect "a commented-out push goes GREEN" 0 "$root"

# ── N7 — good faith: system_prompt.md is out of the population ───────────────
#
# dev-groom's prompt spells its PROHIBITIONS out in full. A semantic scan would
# have to tell a prohibition from a prescription; this one does not have to ask,
# because that file is not in the file set.
echo "N7: system_prompt.md is out of population"
root="$(make_tree n7)"
add_handler "$root" "some-skill" 'PROMPT="Stop there."'
printf '%s\n' 'The pilot MUST NOT execute any of: git push --force, git push --force-with-lease, git push -f' \
    > "$root/skills/bundled/some-skill/system_prompt.md"
expect "N7 — prohibitions in system_prompt.md go GREEN" 0 "$root"

# ── N8 — a stale allowlist entry goes red (two-way comparison) ───────────────
echo "N8: stale allowlist entry"
root="$(make_tree n8)"
add_handler "$root" "some-skill" 'PROMPT="Stop there."'
printf '%s\n' 'skills/bundled/gone-skill/handlers/run.sh' \
    > "$root/scripts/pilot-push-allowlist.txt"
expect "N8 — an entry matching nothing goes RED" 1 "$root"

# ── An allowlisted, still-real site is accepted ──────────────────────────────
echo "allowlist positive control"
root="$(make_tree allowed)"
add_handler "$root" "legacy-skill" 'PROMPT="5. Push with: git push"'
printf '%s\n' '# why: tracked in senara-solutions/mika#2521
skills/bundled/legacy-skill/handlers/run.sh' \
    > "$root/scripts/pilot-push-allowlist.txt"
expect "an allowlisted real site goes GREEN" 0 "$root"

# ── Anti-vacuity: an empty population is refused, not reported clean ─────────
#
# A scan with nothing to scan reads exactly like a clean tree (the mika#2205
# class), and the population can empty itself silently through a directory
# rename or an extension change.
echo "anti-vacuity"
root="$(make_tree empty)"
expect "an empty population goes RED" 1 "$root"

root="$(make_tree renamed)"
mkdir -p "$root/skills/bundled/some-skill/hooks"
printf '%s\n' 'git push' > "$root/skills/bundled/some-skill/hooks/run.sh"
expect "a renamed handler directory goes RED rather than silently empty" 1 "$root"

# ── The real tree is clean, with exactly the documented residue ──────────────
echo "real tree"
expect "the repository itself goes GREEN" 0 "$REPO_ROOT"

real_out="$(bash "$GUARD" "$REPO_ROOT" 2>&1)"
if [[ "$real_out" == *"1 allowlisted"* ]]; then
    ok "the repository carries exactly one allowlisted residue"
else
    ko "the repository carries exactly one allowlisted residue" "output: $real_out"
fi
if [[ "$real_out" == *"7 scanned"* ]]; then
    ok "the population is the seven handler scripts"
else
    ko "the population is the seven handler scripts" \
       "output: $real_out" \
       "If a handler was added or removed, update this count deliberately —" \
       "a population that shrinks unnoticed is how the scan goes quiet."
fi

echo ""
echo "─────────────────────────────────────────"
echo "Passed: $PASS"
echo "Failed: $FAIL"
if [ "$FAIL" -eq 0 ]; then
    echo "ALL TESTS PASS ✓"
    exit 0
else
    echo "SOME TESTS FAILED ✗"
    exit 1
fi
