#!/bin/bash
# Test suite for scripts/verify-egress-no-log.sh (mika#2054).
#
# A guard that has never been seen failing is not a guard. This suite pins the
# lint's NEGATIVE behaviour so a later refactor that makes it exit 0 for a file
# it has stopped modelling — the exact fail-open this ticket closed — is caught
# instead of shipping as a green check.
#
# The lint takes the egress_search directory as its first argument (CI passes
# none and scans the real tree). Each case copies the REAL source tree into a
# fixture directory and mutates it in exactly one way, then runs the lint
# against that copy. Fixtures are synthesized from the live tree, never fetched
# from git history — the property under test is a statement about the SHAPE of
# the source, not about where a ref happens to be standing (mika#2039 fixture
# lesson: an anti-vacuity case that reads the broken state out of history
# inverts the day the branch merges).
#
# Run: bash scripts/test-verify-egress-no-log.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LINT="$REPO_ROOT/scripts/verify-egress-no-log.sh"
LIVE_DIR="$REPO_ROOT/crates/mika-gateway/src/egress_search"

PASS=0
FAIL=0
TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2054-lint-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT

# Run the lint on a fixture directory; echo "<exit>|<combined output>".
run_lint() {
    local dir="$1" out rc=0
    out=$(bash "$LINT" "$dir" 2>&1) || rc=$?
    printf '%s|%s' "$rc" "$out"
}

assert_exit() {
    local label="$1" expected="$2" result="$3"
    local actual="${result%%|*}"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ok $label"
    else
        FAIL=$((FAIL + 1)); echo "  XX $label"
        echo "    expected exit: $expected"
        echo "    actual exit:   $actual"
        echo "    output: ${result#*|}"
    fi
}

assert_mentions() {
    local label="$1" needle="$2" result="$3"
    if printf '%s' "${result#*|}" | grep -q -- "$needle"; then
        PASS=$((PASS + 1)); echo "  ok $label"
    else
        FAIL=$((FAIL + 1)); echo "  XX $label -- output does not mention '$needle'"
        echo "    output: ${result#*|}"
    fi
}

# Copy the real egress_search tree into a fresh fixture directory and echo its
# path. Callers mutate the copy; the live tree is never touched.
fixture_dir() {
    local name="$1"
    local dir="$TMPROOT/$name"
    mkdir -p "$dir"
    cp "$LIVE_DIR"/*.rs "$dir/"
    printf '%s' "$dir"
}

# ============================================================================
echo ""
echo "Test: the live, corrected tree is clean"
echo "----------------------------------------"
# The default path (no argument) is the real substrate. It uses only the two
# modelled `#[cfg(test)]` forms (`mod NAME;`, `mod NAME {`) and only the two
# allowlisted audit events, so it must pass.
assert_exit "real tree (no arg): exit 0" "0" "$(run_lint "")"

# ============================================================================
echo ""
echo "Test: ANTI-VACUITY -- a violation AFTER an unmodeled #[cfg(test)] is caught"
echo "---------------------------------------------------------------------------"
# This is the fail-open the ticket closed. Before the fix, `production_lines`
# hit the unmodeled `#[cfg(test)] fn`, called awk `exit`, and silently dropped
# the rest of the file -- so the `debug!` below it was never scanned and the
# guard reported the tree clean. After the fix the parser fails CLOSED on the
# form it cannot model, so the guard exits non-zero and names the line.
D=$(fixture_dir "violation-after-cfg-test")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
fn helper_next_to_prod() {}

fn production_code_after_the_cfg_test() {
    debug!("egress violation placed AFTER the unmodeled #[cfg(test)] item");
}
RS
R=$(run_lint "$D")
assert_exit "unmodeled #[cfg(test)] fn + later debug!: exit 1" "1" "$R"
assert_mentions "names the offending line + attribute" "unmodeled \`#\[cfg(test)\]\` form" "$R"
assert_mentions "says what to add to the parser" "extend production_lines()" "$R"

# ============================================================================
echo ""
echo "Test: a legitimate test-only block is handled correctly (corrected form)"
echo "-------------------------------------------------------------------------"
# The sanctioned way to add test code next to production: wrap it in a
# `#[cfg(test)] mod NAME { ... }` block. The parser models this form, skips it
# by brace depth, and the tree stays clean even though the block contains a
# `debug!` (allowed inside tests -- the discipline is enforced at the emit
# side, not the visit side; see the script header).
D=$(fixture_dir "legit-test-mod")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
mod extra_unit_tests {
    #[test]
    fn helper_next_to_prod() {
        debug!("a log call inside a #[cfg(test)] mod is fine");
    }
}
RS
assert_exit "wrapped in #[cfg(test)] mod: exit 0" "0" "$(run_lint "$D")"

# ============================================================================
echo ""
echo "Test: a #[cfg(test)] impl (another unmodeled item form) also fails closed"
echo "-------------------------------------------------------------------------"
# The exit branch is not specific to `fn`; any item form the parser does not
# model must fail rather than abandon the file.
D=$(fixture_dir "cfg-test-impl")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
impl Default for SearchRequest {
    fn default() -> Self { unimplemented!() }
}
RS
R=$(run_lint "$D")
assert_exit "unmodeled #[cfg(test)] impl: exit 1" "1" "$R"
assert_mentions "impl case names the parser" "egress-no-log, parser" "$R"

# ============================================================================
echo ""
echo "Test: info! allowlist attaches to the CALL BLOCK, not a fixed 8-line window"
echo "---------------------------------------------------------------------------"
# Before the fix, the check read `line_no + 8` lines below the `info!(` opener
# and passed the call if any allowlisted `event = "..."` token sat anywhere in
# that window. So a non-allowlisted info! passed whenever an allowlisted token
# happened to appear a few lines below it, in an unrelated statement. After the
# fix the check reads the macro's own parenthesised block; the token below no
# longer rescues it.
D=$(fixture_dir "info-window-bypass")
cat >> "$D/mod.rs" <<'RS'

fn leaky_info_with_allowed_token_below() {
    info!(
        event = "tenant_query_leak",
        query = "SECRET",
        "not an allowlisted event"
    );
    let _padding_a = 1;
    let _padding_b = 2;
    // An allowlisted token sits within 8 lines below the info!( opener but
    // belongs to a different statement -- the old window would have matched it.
    let _decoy = "event = \"search_egress\"";
}
RS
R=$(run_lint "$D")
assert_exit "non-allowlisted info! + allowed token 8 lines below: exit 1" "1" "$R"
assert_mentions "flags the non-allowlisted info!" "not in Q4 allowlist" "$R"

# ============================================================================
echo ""
echo "Test: a legitimately-allowlisted info! still passes"
echo "----------------------------------------------------"
# Guard against over-correction: the block-attachment must still recognise a
# real audit event whose `event = "..."` token lives inside the call block.
D=$(fixture_dir "info-allowed")
cat >> "$D/mod.rs" <<'RS'

fn extra_allowed_audit_line() {
    info!(
        event = "search_requested",
        upstream = "brave",
        "a second, legitimately-allowlisted audit line"
    );
}
RS
assert_exit "allowlisted info! block: exit 0" "0" "$(run_lint "$D")"

# ============================================================================
echo ""
echo "Test: an info! whose block never closes fails closed (undeterminable)"
echo "----------------------------------------------------------------------"
# If the macro's parentheses cannot be balanced before end-of-file, the guard
# refuses to judge the call on an incomplete block rather than guessing.
D=$(fixture_dir "info-unbalanced")
cat >> "$D/mod.rs" <<'RS'

fn truncated_info() {
    info!(
        event = "search_requested",
        upstream = "brave",
RS
R=$(run_lint "$D")
assert_exit "unbalanced info! block: exit 1" "1" "$R"
assert_mentions "says the block is undeterminable" "block undeterminable" "$R"

# ============================================================================
echo ""
echo "Test: a plain production log violation is still caught (baseline sanity)"
echo "------------------------------------------------------------------------"
# The parser fix must not disturb the ordinary path: a forbidden macro in
# production code, with no #[cfg(test)] anywhere near it, still fails.
D=$(fixture_dir "plain-violation")
cat >> "$D/mod.rs" <<'RS'

fn ordinary_production_leak() {
    warn!("a plain forbidden log call in production code");
}
RS
R=$(run_lint "$D")
assert_exit "plain warn! in production: exit 1" "1" "$R"
assert_mentions "names the forbidden macro" "warn!" "$R"

# ============================================================================
echo ""
echo "Test: a moved or missing substrate directory fails loudly, never green"
echo "-----------------------------------------------------------------------"
R=$(run_lint "$TMPROOT/does-not-exist")
assert_exit "missing directory: exit 1" "1" "$R"
assert_mentions "missing dir: says what to update" "update EGRESS_DIR" "$R"

# ============================================================================
echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="

[ "$FAIL" -eq 0 ] || exit 1
exit 0
