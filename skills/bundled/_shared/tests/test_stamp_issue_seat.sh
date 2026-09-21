#!/bin/bash
# Test suite for _stamp_issue_seat / _release_issue_seat (mika#2155).
#
# The loop is a dispatch seat like ssc and mpc (webhook_dispatch.rs
# CURRENT_DISPATCH_SEAT), and until mika#2155 it was the only seat that never
# said so on the ticket. This suite pins the producer half, symmetric to
# test_stamp_pr_origin.sh: dispatch-lib labels the ISSUE `dispatch:loop` when it
# takes the ticket, never writes over another seat's claim (AC3), never aborts a
# dispatch when the stamp fails (AC2), and releases the claim on exit (AC4).
#
# `gh` is stubbed on PATH; every call is journalled to $GH_LOG so the assertions
# can read the exact argv the function produced. T4–T7 are assertions of
# ABSENCE — a write that must not happen — and T1 is their positive control in
# the same suite: it proves the journal captures `--add-label dispatch:loop`
# when it IS emitted, so an absence is a fact and not a blind grep.
#
# Source isolation audit: dispatch-lib.sh has no top-level imperative code —
# all `set -e`, `trap`, and env var references are inside function bodies.
# Safe to source directly without a guard variable.
#
# Run: bash skills/bundled/_shared/tests/test_stamp_issue_seat.sh
# Expected: all assertions pass, exit 0.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

PASS=0
FAIL=0

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

assert_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    missing: '$needle'"
        echo "    in:      '$haystack'"
    fi
}

assert_not_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    unexpectedly present: '$needle'"
    else
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    fi
}

assert_lt() {
    local label="$1" a="$2" b="$3"
    if [ -n "$a" ] && [ -n "$b" ] && [ "$a" -lt "$b" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected: '$a' < '$b'"
    fi
}

STUB_DIR=$(mktemp -d)
export GH_LOG="$STUB_DIR/gh.log"
trap 'rm -rf "$STUB_DIR"' EXIT

# `gh` stub. Behaviour is driven by $GH_MODE:
#   ok            — everything succeeds
#   always-fails  — every call fails (network down, no write scope, label
#                   undeclared on a repo outside label-sync, …)
cat > "$STUB_DIR/gh" <<'STUB'
#!/bin/bash
printf '%s\n' "$*" >> "$GH_LOG"
case "${GH_MODE:-ok}" in
    ok) exit 0 ;;
    always-fails) exit 1 ;;
esac
STUB
chmod +x "$STUB_DIR/gh"
export PATH="$STUB_DIR:$PATH"

reset_stub() {
    : > "$GH_LOG"
    ISSUE_SEAT_CLAIMED=0
}

STAMP_ARGV="issue edit 2155 --repo senara-solutions/mika --add-label dispatch:loop"

echo "=== _stamp_issue_seat / _release_issue_seat (mika#2155) ==="

# ── T1. Nominal: no seat label → one `issue edit --add-label dispatch:loop` ──
# This is ALSO the positive control for T4–T7 below: it proves the journal
# captures the exact string those tests assert is absent.
echo "-- T1 nominal stamp (positive control) --"
reset_stub
export GH_MODE=ok
rc=0
err=$(_stamp_issue_seat "mika" "2155" "bug,ready" 2>&1 >/dev/null) || rc=$?
log=$(cat "$GH_LOG")
assert_eq "T1 returns 0" "0" "$rc"
assert_contains "T1 edits the issue with dispatch:loop" "$log" "$STAMP_ARGV"
assert_eq "T1 exactly one gh call" "1" "$(wc -l < "$GH_LOG")"
assert_contains "T1 names the stamp on stderr" "$err" "dispatch_seat.stamped"

# ── T2. Idempotent: already dispatch:loop → no write, rc 0 (AC2) ────────────
echo "-- T2 already owned --"
reset_stub
rc=0
err=$(_stamp_issue_seat "mika" "2155" "ready,dispatch:loop" 2>&1 >/dev/null) || rc=$?
assert_eq "T2 returns 0" "0" "$rc"
assert_eq "T2 no gh call" "0" "$(wc -l < "$GH_LOG")"
assert_contains "T2 names the existing claim" "$err" "dispatch_seat.already_owned"

# ── T3. Same, case-insensitive (the engine folds case too) ──────────────────
echo "-- T3 already owned, mixed case --"
reset_stub
rc=0
err=$(_stamp_issue_seat "mika" "2155" "ready,Dispatch:LOOP" 2>&1 >/dev/null) || rc=$?
assert_eq "T3 returns 0" "0" "$rc"
assert_eq "T3 no gh call" "0" "$(wc -l < "$GH_LOG")"
assert_contains "T3 names the existing claim" "$err" "dispatch_seat.already_owned"

# ── T4–T7. Another seat is there → NO write, rc 1, named (AC3 / C-2) ────────
# Four populations, one assertion each — a fix that drops the seat check must
# turn all four red, not just one (term by term).
for tc in \
    "T4|ready,dispatch:ssc" \
    "T5|dispatch:mpc,ready" \
    "T6|dispatch:ssc,dispatch:loop" \
    "T7|dispatch:zorglub" \
    "T7b|dispatch:ssc,dispatch:mpc"; do
    name="${tc%%|*}"; labels="${tc#*|}"
    echo "-- $name owned by other: '$labels' --"
    reset_stub
    rc=0
    err=$(_stamp_issue_seat "mika" "2155" "$labels" 2>&1 >/dev/null) || rc=$?
    log=$(cat "$GH_LOG")
    assert_eq "$name returns 1" "1" "$rc"
    assert_not_contains "$name never writes dispatch:loop" "$log" "--add-label dispatch:loop"
    assert_eq "$name no gh call at all" "0" "$(wc -l < "$GH_LOG")"
    assert_contains "$name names the other claim" "$err" "dispatch_seat.owned_by_other"
done

# ── T8. gh fails → tried once, rc 1, named, and NO `label create` (AC2, C-1) ─
# No create-on-the-fly: dispatch:loop is a SEAT label whose vocabulary is
# guarded Rust↔YAML on mika (mika#2092). Creating it on a repo outside that
# guard would fabricate an unguarded seat.
echo "-- T8 gh unavailable --"
reset_stub
export GH_MODE=always-fails
rc=0
err=$(_stamp_issue_seat "mika-cloud" "7" "bug" 2>&1 >/dev/null) || rc=$?
log=$(cat "$GH_LOG")
assert_eq "T8 returns 1" "1" "$rc"
assert_contains "T8 tried the edit" "$log" "issue edit 7 --repo senara-solutions/mika-cloud --add-label dispatch:loop"
assert_not_contains "T8 never creates the label" "$log" "label create"
assert_eq "T8 exactly one attempt" "1" "$(wc -l < "$GH_LOG")"
assert_contains "T8 names the failure" "$err" "dispatch_seat.stamp_failed"
assert_contains "T8 says the dispatch proceeds" "$err" "dispatch proceeds"

# ── T9. Free-text mode: empty repo or issue → rc 0, no call ─────────────────
echo "-- T9 missing arguments --"
reset_stub
export GH_MODE=always-fails
rc=0; _stamp_issue_seat "" "2155" "bug" || rc=$?
assert_eq "T9 empty repo: rc 0" "0" "$rc"
rc=0; _stamp_issue_seat "mika" "" "bug" || rc=$?
assert_eq "T9 empty issue: rc 0" "0" "$rc"
assert_eq "T9 no gh call made" "0" "$(wc -l < "$GH_LOG")"

# ── T10. Release: claimed → one `--remove-label dispatch:loop`, nothing else ─
echo "-- T10 release --"
reset_stub
export GH_MODE=ok
ISSUE_SEAT_CLAIMED=1
rc=0
# Called in THIS shell (not a $(...) subshell): T10 reads the flag afterwards.
_release_issue_seat "mika" "2155" 2>"$STUB_DIR/err" >/dev/null || rc=$?
err=$(cat "$STUB_DIR/err")
log=$(cat "$GH_LOG")
assert_eq "T10 returns 0" "0" "$rc"
assert_contains "T10 removes dispatch:loop" "$log" \
    "issue edit 2155 --repo senara-solutions/mika --remove-label dispatch:loop"
assert_eq "T10 exactly one gh call" "1" "$(wc -l < "$GH_LOG")"
assert_eq "T10 no other --remove-label value" "1" \
    "$(grep -o -- '--remove-label [^ ]*' "$GH_LOG" | grep -c '^--remove-label dispatch:loop$' || true)"
assert_not_contains "T10 never touches ready" "$log" "ready"
assert_not_contains "T10 never touches dispatch:mpc" "$log" "dispatch:mpc"
assert_not_contains "T10 never touches dispatch:ssc" "$log" "dispatch:ssc"
assert_contains "T10 names the release" "$err" "dispatch_seat.released"
assert_eq "T10 a successful release ends the claim (flag -> 0)" "0" "${ISSUE_SEAT_CLAIMED:-unset}"
rc=0; _release_issue_seat "mika" "2155" || rc=$?
assert_eq "T10 a second release after success is a no-op: rc 0" "0" "$rc"
assert_eq "T10 a second release after success makes no gh call" "1" "$(wc -l < "$GH_LOG")"

# ── T11. Release without a claim → no call ──────────────────────────────────
echo "-- T11 release, never claimed --"
reset_stub
ISSUE_SEAT_CLAIMED=0
rc=0; _release_issue_seat "mika" "2155" || rc=$?
assert_eq "T11 returns 0" "0" "$rc"
assert_eq "T11 no gh call" "0" "$(wc -l < "$GH_LOG")"
unset ISSUE_SEAT_CLAIMED
rc=0; _release_issue_seat "mika" "2155" || rc=$?
assert_eq "T11 unset flag: rc 0" "0" "$rc"
assert_eq "T11 unset flag: no gh call" "0" "$(wc -l < "$GH_LOG")"
ISSUE_SEAT_CLAIMED=1
rc=0; _release_issue_seat "" "2155" || rc=$?
assert_eq "T11 empty repo: rc 0" "0" "$rc"
assert_eq "T11 empty repo: no gh call" "0" "$(wc -l < "$GH_LOG")"

# ── T12. Release fails → rc 1, named; the caller's `|| true` carries on ─────
echo "-- T12 release, gh unavailable --"
reset_stub
export GH_MODE=always-fails
ISSUE_SEAT_CLAIMED=1
rc=0
_release_issue_seat "mika" "2155" 2>"$STUB_DIR/err" >/dev/null || rc=$?
err=$(cat "$STUB_DIR/err")
assert_eq "T12 returns 1" "1" "$rc"
assert_contains "T12 names the failure" "$err" "dispatch_seat.release_failed"
assert_eq "T12 exactly one attempt" "1" "$(wc -l < "$GH_LOG")"
assert_eq "T12 a failed release keeps the claim (flag stays 1) so the next site retries" "1" "${ISSUE_SEAT_CLAIMED:-unset}"
rc=0
( set -e; _release_issue_seat "mika" "2155" 2>/dev/null || true; echo "still-here" ) > "$STUB_DIR/cont" || rc=$?
assert_eq "T12 a '|| true' caller continues" "still-here" "$(cat "$STUB_DIR/cont")"

# ── T13. Stale residue heals as a sequence: already_owned → claim → release ─
# A dispatch killed without its trap leaves dispatch:loop behind. The next
# dispatch on the ticket reads it already_owned (no write), still counts as a
# claim (the callsite sets the flag regardless of rc), and its release removes
# it exactly once. T2 and T10 pin the halves; this pins the sequence.
echo "-- T13 stale residue heals across stamp then release --"
reset_stub
export GH_MODE=ok
_stamp_issue_seat "mika" "2155" "ready,dispatch:loop" 2>/dev/null || true
ISSUE_SEAT_CLAIMED=1
_release_issue_seat "mika" "2155" 2>/dev/null || true
assert_eq "T13 exactly one gh call across the sequence" "1" "$(wc -l < "$GH_LOG")"
assert_contains "T13 and it is the release" "$(cat "$GH_LOG")" "--remove-label dispatch:loop"
assert_not_contains "T13 no re-stamp happened" "$(cat "$GH_LOG")" "--add-label"

# ── T14. The DRY_RUN guard at the callsite, evaluated for real ──────────────
# The guard is three lines of production shell that no unit test reaches:
# a structural grep only pins that the token DRY_RUN sits nearby. Extract the
# real block from dispatch-lib and evaluate it with a spy in place of
# _stamp_issue_seat, so an inverted guard (stamp on dry run, skip on real
# dispatch) turns this red instead of merging clean.
echo "-- T14 DRY_RUN guard, evaluated from the real source --"
guard_block=$(awk '/mika#2155: claim the ticket for the loop BEFORE/,/^        fi$/' "$DISPATCH_LIB")
assert_contains "T14 extracted the guard block (positive control on the extraction)" "$guard_block" '_stamp_issue_seat "$REPO" "$ISSUE_NUM" "$LABELS"'
for tc in "true|skip" "1|skip" "false|stamp" "|stamp" "0|stamp"; do
    val="${tc%%|*}"; want="${tc#*|}"
    SPY_LOG="$STUB_DIR/spy.log"; : > "$SPY_LOG"
    out=$(
        REPO=mika; ISSUE_NUM=2155; LABELS="bug,ready"; DRY_RUN="$val"; ISSUE_SEAT_CLAIMED=0
        _stamp_issue_seat() { printf 'spy %s %s %s\n' "$1" "$2" "$3" >> "$SPY_LOG"; return 0; }
        eval "$guard_block"
        printf 'claimed=%s' "$ISSUE_SEAT_CLAIMED"
    )
    if [ "$want" = "skip" ]; then
        assert_eq "T14 DRY_RUN='$val' skips the stamp" "0" "$(wc -l < "$SPY_LOG")"
        assert_eq "T14 DRY_RUN='$val' leaves the claim flag down" "claimed=0" "$out"
    else
        assert_contains "T14 DRY_RUN='$val' stamps with the callsite's arguments" "$(cat "$SPY_LOG")" "spy mika 2155 bug,ready"
        assert_eq "T14 DRY_RUN='$val' raises the claim flag" "claimed=1" "$out"
    fi
done

# ── Every gh call is bounded ────────────────────────────────────────────────
# One callsite is the crash/cancel exit trap, whose job is to get RESULT back to
# mika-dev. A hanging GitHub API must not delay the news that a dispatch died.
echo "-- bounded network calls --"
for fn in _stamp_issue_seat _release_issue_seat; do
    fn_body=$(awk "/^${fn}\\(\\) \\{/,/^\\}/" "$DISPATCH_LIB")
    n_gh=$(grep -c '[^_]gh issue ' <<<"$fn_body" || true)
    n_timeout=$(grep -c 'timeout [0-9]* gh ' <<<"$fn_body" || true)
    assert_eq "$fn: every gh invocation carries a timeout" "$n_gh" "$n_timeout"
    assert_lt "$fn: at least one gh call" "0" "$n_gh"
done

# ── Structural: callsites are fail-open ─────────────────────────────────────
# The label is a signal for the other seats, not a barrier for this one (AC2).
echo "-- structural: callsites are fail-open --"
stamp_sites=$(grep -n '_stamp_issue_seat "' "$DISPATCH_LIB" | grep -v '^[0-9]*: *#' || true)
release_sites=$(grep -n '_release_issue_seat "' "$DISPATCH_LIB" | grep -v '^[0-9]*: *#' || true)
assert_eq "exactly one stamp callsite" "1" "$(printf '%s\n' "$stamp_sites" | grep -c . || true)"
assert_eq "exactly two release callsites (callback, then exit trap)" "2" "$(printf '%s\n' "$release_sites" | grep -c . || true)"
assert_contains "the stamp callsite ends in '|| true'" "$stamp_sites" "|| true"
assert_eq "both release callsites end in '|| true'" "2" "$(printf '%s\n' "$release_sites" | grep -c '|| true' || true)"

# ── Structural: the stamp site sits after the last no-dispatch exit and before
#    the first mutation (AC1 at the site, D-1) ────────────────────────────────
# Both bounds are facts of the code, not choices: a dispatch refused by the
# #2012 gate must not claim; a claim after `git fetch origin main` is a lie of
# one second. If this turns red, move the SITE back — never the bounds.
echo "-- structural: the stamp site is between the groom gate and the first mutation --"
setup_start=$(grep -n '^_set_up_worktree() {' "$DISPATCH_LIB" | cut -d: -f1)
stamp_line=$(printf '%s\n' "$stamp_sites" | cut -d: -f1 | head -1)
gate_line=$(grep -n 'dispatch_gate_groom_refused' "$DISPATCH_LIB" | grep -v '^[0-9]*: *#' | cut -d: -f1 | head -1)
fetch_line=$(grep -n 'git -C "\$SUB_REPO_DIR" fetch origin main' "$DISPATCH_LIB" | cut -d: -f1 | head -1)
dry_run_line=$(grep -n '^ *if \[ "\$DRY_RUN" != "true" \] && \[ "\$DRY_RUN" != "1" \]; then' "$DISPATCH_LIB" | cut -d: -f1 | awk -v s="$stamp_line" '$1 < s' | tail -1)
assert_lt "stamp site is inside _set_up_worktree" "$setup_start" "$stamp_line"
assert_lt "stamp site is after the #2012 groom gate" "$gate_line" "$stamp_line"
assert_lt "stamp site is before the first mutation (git fetch origin main)" "$stamp_line" "$fetch_line"
assert_eq "stamp site is the line right under the real DRY_RUN if-guard" "$((stamp_line - 1))" "$dry_run_line"

# ── Structural: the release is the first useful line of the EXIT trap, before
#    the CALLBACK_SENT early return (C-4) ───────────────────────────────────
echo "-- structural: the release runs on every exit path --"
# The claim must be gone BEFORE `mika ask --task-complete`: that message is
# what lets mika-dev start the next dispatch on this ticket, and a next
# dispatch that reads a label its predecessor is about to remove would run
# unclaimed for its whole life (review finding #2 on mika#2155).
cb_start=$(grep -n '^_deliver_callback() {' "$DISPATCH_LIB" | cut -d: -f1)
cb_release=$(printf '%s\n' "$release_sites" | cut -d: -f1 | awk -v s="$cb_start" '$1 > s' | head -1)
cb_ask=$(awk -v s="$cb_start" 'NR > s && /mika ask --task-id "\$TASK_ID" --task-complete/ { print NR; exit }' "$DISPATCH_LIB")
assert_lt "release is inside _deliver_callback" "$cb_start" "$cb_release"
assert_lt "release runs before mika ask --task-complete" "$cb_release" "$cb_ask"
trap_start=$(grep -n '^_dispatch_lib_exit_trap() {' "$DISPATCH_LIB" | cut -d: -f1)
release_line=$(printf '%s\n' "$release_sites" | cut -d: -f1 | awk -v s="$trap_start" '$1 > s' | head -1)
guard_line=$(awk -v s="$trap_start" 'NR > s && /"\$CALLBACK_SENT" -eq 1/ { print NR; exit }' "$DISPATCH_LIB")
assert_lt "release is inside the exit trap" "$trap_start" "$release_line"
assert_lt "release runs before the CALLBACK_SENT early return" "$release_line" "$guard_line"
init_line=$(grep -n '^    ISSUE_SEAT_CLAIMED=0' "$DISPATCH_LIB" | cut -d: -f1 | head -1)
dcp_start=$(grep -n '^dispatch_claude_pilot() {' "$DISPATCH_LIB" | cut -d: -f1)
assert_lt "ISSUE_SEAT_CLAIMED is initialised in dispatch_claude_pilot" "$dcp_start" "$init_line"

echo
echo "PASS: $PASS  FAIL: $FAIL"
[ "$FAIL" -eq 0 ] || exit 1
