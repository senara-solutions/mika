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

assert_true() {
    local label="$1" ok="$2" detail="${3:-}"
    if [ "$ok" = "1" ]; then
        PASS=$((PASS + 1)); echo "  ok $label"
    else
        FAIL=$((FAIL + 1)); echo "  XX $label"
        [ -n "$detail" ] && echo "    $detail"
    fi
}

assert_mentions() {
    local label="$1" needle="$2" result="$3"
    # Herestring, NOT `printf … | grep -q`: under `pipefail` the pipe form is a
    # SIGPIPE trap (grep -q exits at the first match, printf takes 141, pipefail
    # promotes it, and a PRESENT needle reads as absent). This is the exact
    # shape the compound doc for this work argues is owed a lint; the guard's
    # own header preaches the same, so the test honours it.
    if grep -q -- "$needle" <<< "${result#*|}"; then
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
# mika#2054 rev 2 — the two blind spots #2079 left in the SAME parser.
#
# #2079 made the unmodeled `#[cfg(test)]` form fail closed. It did not touch the
# two decisions that run BEFORE that branch: how the attribute reaches its item
# (D1) and how the inline block's scope is tracked (D3). Every case below marked
# "seen red" failed against the pre-fix parser before it passed against the
# fixed one — a case that has never failed attests nothing. Cases marked "lock"
# are green on BOTH sides by construction: their value is to redden later, and
# counting them as evidence of a correction is the error this note forbids.
#
# Fixture-writing rule, and it is load-bearing: the attribute goes on its OWN
# line and the item on a following one. `/^[[:space:]]*#\[cfg\(test\)\]/` is not
# right-anchored, so `#[cfg(test)] mod m { … }` written on one line matches the
# attribute, consumes the whole line, and arms `pending` — the NEXT line then
# lands in the fail-closed branch and the guard exits 1 for the wrong reason
# ("parser", not the violation under test). Such a fixture reddens while
# attesting nothing.
# ============================================================================

echo ""
echo "Test: D1 -- an intercalary line does not consume the #[cfg(test)] state"
echo "------------------------------------------------------------------------"
# `pending_cfg_test` was consumed by the next line WHATEVER it was, so a blank
# line, a comment, or a second attribute between the attribute and its item fell
# into the fail-closed branch. The guard went red on ordinary Rust, naming a
# blank line and prescribing a `mod` wrapper for code that already had one. A
# guard that reddens on legitimate code, with a diagnostic that does not describe
# the fault, is a guard that gets disarmed.

D=$(fixture_dir "cfg-test-blank-intercalary")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]

mod v1_blank_line_before_item {
    #[test]
    fn t() {
        debug!("reached across a blank line -- still test code");
    }
}
RS
# V1 -- seen red (exit 1, "unmodeled ... form" pointing at the blank line).
assert_exit "V1 blank line between attribute and item: exit 0" "0" "$(run_lint "$D")"

D=$(fixture_dir "cfg-test-comment-intercalary")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
// why these tests live next to the production code
mod v2_comment_before_item {
    #[test]
    fn t() {
        debug!("reached across a comment line -- still test code");
    }
}
RS
# V2 -- seen red.
assert_exit "V2 comment between attribute and item: exit 0" "0" "$(run_lint "$D")"

D=$(fixture_dir "cfg-test-attribute-intercalary")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod v3_attribute_before_item {
    #[test]
    fn t() {
        debug!("reached across a second attribute -- still test code");
    }
}
RS
# V3 -- seen red.
assert_exit "V3 non-cfg attribute between attribute and item: exit 0" "0" "$(run_lint "$D")"

# ============================================================================
echo ""
echo "Test: D1 borne -- a SECOND cfg attribute is not a neutral intercalary"
echo "-----------------------------------------------------------------------"
# LOCK, green on both sides. Traversing intercalary lines must not traverse a
# second `cfg`: two cfg conditions on one item make the item undecidable for this
# parser, and `#[cfg(test)]` followed by `#[cfg(not(test))]` is outright
# contradictory. The predicate is syntactic -- "does the traversed attribute start
# with `#[cfg`?" -- so it needs no model of composed cfg forms. This case exists so
# a later widening of the traversal cannot swallow the dangerous shape silently.
D=$(fixture_dir "cfg-test-stacked-cfg")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
#[cfg(not(test))]
fn v3b_contradictory_cfg_stack() {}
RS
R=$(run_lint "$D")
assert_exit "V3b stacked contradictory cfg: exit 1" "1" "$R"
assert_mentions "V3b names the parser" "egress-no-log, parser" "$R"

# ============================================================================
echo ""
echo "Test: #[cfg(not(test))] is PRODUCTION and stays scanned"
echo "---------------------------------------------------------"
# LOCK, green on both sides. `#[cfg(not(test))]` contains the token `test` and
# means the opposite: code compiled OUTSIDE tests, i.e. production, i.e. exactly
# what this guard must scan. It is not recognized by the attribute regex, falls
# through to the default branch, and is emitted as production -- which is already
# correct today. This case attests no correction; its value is to redden the day
# someone widens the attribute regex to catch composed `cfg` forms and takes the
# trap form with it, re-introducing the fail-open through the fix meant to close it.
D=$(fixture_dir "cfg-not-test-is-production")
cat >> "$D/mod.rs" <<'RS'

#[cfg(not(test))]
fn v5_production_under_not_test() {
    debug!("cfg(not(test)) is production -- this MUST be reported");
}
RS
R=$(run_lint "$D")
assert_exit "V5 cfg(not(test)) violation: exit 1" "1" "$R"
assert_mentions "V5 names the forbidden macro" "debug!" "$R"

# ============================================================================
echo ""
echo "Test: D3a -- an inline test module closed on its own line is not a scope"
echo "--------------------------------------------------------------------------"
# THE FAIL-OPEN, and it is the ticket's own class. `mod tests { fn t() {} }`
# matched `mod NAME {`, so the parser posted brace_depth = 1 by hand and skipped
# the line -- without noticing the line carries as many closers as openers and the
# block is ALREADY closed. It then believed itself inside a test module for the
# rest of the file: every production line after it silently dropped from the scan,
# a partial audit that reads exactly like a complete one.
D=$(fixture_dir "inline-mod-closed-on-own-line")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
mod v7_closed_on_its_own_line { fn t() {} }

fn v7_production_after_the_inline_module() {
    warn!("production leak AFTER a single-line #[cfg(test)] module");
}
RS
R=$(run_lint "$D")
# V7 -- seen red (exit 0: the rest of the file was abandoned).
assert_exit "V7 violation after a self-closed inline mod: exit 1" "1" "$R"
assert_mentions "V7 names the forbidden macro" "warn!" "$R"

# ============================================================================
echo ""
echo "Test: D3b -- an unpaired brace inside a test string does not extend scope"
echo "---------------------------------------------------------------------------"
# The scope tracker counted braces on the RAW line, so a `{` living in a string
# literal pushed the depth up and the skip ran past the module's closing brace,
# swallowing the production code behind it: fail-open. (An excess `}` ends the
# skip too early and audits test code as production: false positive. Same cause,
# opposite sign.) `sanitize()` -- which #2079 shipped for the `info!` parser a
# hundred lines away -- is what this case forces onto the parser that decides what
# gets audited at all.
D=$(fixture_dir "unpaired-brace-in-test-string")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
mod v8_unpaired_brace_in_string {
    #[test]
    fn t() {
        let j = "{";
        let _ = j;
    }
}

fn v8_production_after_the_test_mod() {
    warn!("production leak AFTER a test module holding an unpaired brace");
}
RS
R=$(run_lint "$D")
# V8 -- seen red (exit 0: the skip never terminated).
assert_exit "V8 violation after an unpaired brace in a string: exit 1" "1" "$R"
assert_mentions "V8 names the forbidden macro" "warn!" "$R"

# ============================================================================
echo ""
echo "Test: an inline test block that never closes fails closed"
echo "-----------------------------------------------------------"
# The mirror of the `info!` block-undeterminable rule, applied to the scope
# parser: if the braces cannot be balanced before end-of-file, the parser cannot
# say where production resumes, so it refuses rather than silently dropping the
# tail. Distinct wording from the unmodeled-form diagnostic: this is not an
# unknown shape, it is a block left open.
D=$(fixture_dir "inline-mod-never-closed")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
mod v9_never_closed {
    fn t() {
RS
R=$(run_lint "$D")
# V9 -- seen red (exit 0: the truncated tail was abandoned in silence).
assert_exit "V9 unterminated inline test module: exit 1" "1" "$R"
assert_mentions "V9 says the block is undeterminable" "block undeterminable" "$R"

# ============================================================================
echo ""
echo "Test: a #[cfg(test)] attribute with no item fails closed"
echo "----------------------------------------------------------"
# The other borne of the intercalary traversal: traversing non-significant lines
# must not let the state leak off the end of the file. An attribute with no item
# is code that does not compile, and staying silent would make a truncated file
# indistinguishable from a healthy one.
D=$(fixture_dir "dangling-cfg-test")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
RS
R=$(run_lint "$D")
# V10 -- seen red (exit 0).
assert_exit "V10 dangling #[cfg(test)] at end of file: exit 1" "1" "$R"
assert_mentions "V10 says the attribute is dangling" "dangling" "$R"

# ============================================================================
echo ""
echo "Test: GOOD FAITH -- paired braces inside test strings still close the scope"
echo "-----------------------------------------------------------------------------"
# LOCK against over-correction, and it is the case that makes V7/V8 mean
# something. A sanitizer that returned nothing, or a counter that stopped
# counting, would ALSO pass V7 and V8 while looking correct -- here it would run
# the skip to end-of-file and trip the unterminated-block rule. "It sanitizes" and
# "it is broken" are indistinguishable without this.
D=$(fixture_dir "paired-braces-in-test-strings")
cat >> "$D/mod.rs" <<'RS'

#[cfg(test)]
mod v11_paired_braces_in_strings {
    #[test]
    fn t() {
        let s = format!("{}", 1);
        let _ = format!("{s}");
        debug!("a log call inside a #[cfg(test)] mod is fine");
    }
}

fn v11_production_after_the_test_mod() {
    let _ = 1;
}
RS
assert_exit "V11 paired braces in strings: exit 0" "0" "$(run_lint "$D")"

# ============================================================================
echo ""
echo "Test: GOOD FAITH -- an unmutated copy of the live tree is clean"
echo "-----------------------------------------------------------------"
# LOCK. The first assertion of this file scans the real directory in place; this
# one scans the COPY the mutating cases are built from, so a green above cannot be
# read as proof that the copy itself is sound. Non-regression of the live
# substrate under the hardened parser.
D=$(fixture_dir "unmutated-copy")
assert_exit "V12 unmutated copy of the live tree: exit 0" "0" "$(run_lint "$D")"

# ============================================================================
echo ""
echo "Test: sanitize() has ONE definition, and both parsers call it"
echo "---------------------------------------------------------------"
# LOCK (source scan, not a lint run). Two parsers in the guard depend on
# sanitize(): the `#[cfg(test)]` scope tracker and the `info!` block delimiter. A
# second copy breaks nothing on the day it is written -- it lets one parser stay
# correct while the other silently regresses, which is precisely the divergence
# this scan refuses. Doctrine mika#2201: when this fires you interpolate the one
# definition, you do NOT add an entry below.
#
# Exemption table shipped EMPTY and checked BOTH ways -- an entry that no longer
# matches anything in the guard reddens on the day it goes stale, not months later.
SANITIZE_SCAN_EXEMPTIONS=()

def_count=$(grep -c 'function sanitize(' "$LINT")
# Anti-vacuity, and it is not a restatement of the count above: if the scope
# parser ever stops CALLING sanitize -- someone re-inlining a raw gsub count --
# the single-definition check still passes while R4's whole point is gone. A scan
# that has gone blind reads exactly like a clean tree (mika#2205).
call_count=$(grep -c 'sanitize(\$0)' "$LINT")

assert_true "V13 exactly one \`function sanitize(\` definition" \
    "$([ "$def_count" -eq 1 ] && echo 1 || echo 0)" \
    "found $def_count definition(s), expected exactly 1"
assert_true "V13 both awk parsers call sanitize() (anti-vacuity)" \
    "$([ "$call_count" -ge 2 ] && echo 1 || echo 0)" \
    "found $call_count call site(s) of sanitize(\$0), expected at least 2"

stale=""
for ex in ${SANITIZE_SCAN_EXEMPTIONS+"${SANITIZE_SCAN_EXEMPTIONS[@]}"}; do
    grep -q -- "$ex" "$LINT" || stale="$stale $ex"
done
assert_true "V13 no stale entry in the (empty) exemption table" \
    "$([ -z "$stale" ] && echo 1 || echo 0)" \
    "exemptions matching nothing in the guard:$stale"

# ============================================================================
echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="

[ "$FAIL" -eq 0 ] || exit 1
exit 0
