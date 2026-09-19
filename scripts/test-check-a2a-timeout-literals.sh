#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-a2a-timeout-literals.sh (mika#2309).
#
# House constraint, written into ci.yml since mika#2103: *a guard nobody has
# watched go red is a decoration*. The founding incident there is exactly a lint
# that could not fail on the defect it existed to catch, and stayed green
# through 26 production panics.
#
# This suite proves the guard bites on each rule, on the arithmetic bypass that
# § M2 of the plan measured as already-written, on the allowlist in both
# directions, and on the vacuous-scan case — and, just as load-bearing, that it
# does *not* bite on the compliant shapes § Fire-Disposition surface 1 measured.
#
# "Delete the thing the test protects; confirm the test goes red."

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-a2a-timeout-literals.sh"

PASS=0
FAIL=0

# Run the guard against scan root $1; assert its exit equals $2. $3 = case name.
assert_exit() {
    local root="$1" want="$2" name="$3"
    local got=0
    bash "$GUARD" "$root" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

# Assert the guard's output on root $1 contains $2. $3 = case name.
#
# The output is captured and matched with `case`, not piped into `grep -q`:
# under `pipefail` a failing producer poisons the pipeline's status even when
# grep matches, and `echo | grep -q` is itself refused by the sigpipe lint
# (mika#2055).
assert_says() {
    local root="$1" want="$2" name="$3"
    local out
    out="$(bash "$GUARD" "$root" 2>&1 || true)"
    if [[ "$out" == *"$want"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not mention '$want')"
        FAIL=$((FAIL + 1))
    fi
}

# A throwaway tree holding one perimeter file whose body is $1.
make_fixture() {
    local dir
    dir="$(mktemp -d)"
    mkdir -p "$dir/crates/mika-a2a/src"
    printf '%s\n' "$1" > "$dir/crates/mika-a2a/src/probe.rs"
    echo "$dir"
}

# ── 1. The real, swept tree is clean (V1).
assert_exit "$REPO_ROOT" 0 "real repo perimeter is clean"

# ── 2. Rule A1: the regression this guard exists to catch. A budget written at
#      the site that bounds the call instead of read from a resolver.
d="$(make_fixture '            .timeout(Duration::from_secs(900))')"
assert_exit "$d" 1 "A1: a literal duration at a bounding site is rejected"
rm -rf "$d"

# ── 3. Rule A1 + § M2: the arithmetic bypass. `10 * 60` and `600` are the same
#      budget, and a guard evaded by retyping it is worse than no guard (V3).
d="$(make_fixture '            .timeout(Duration::from_secs(10 * 60))')"
assert_exit "$d" 1 "A1: the arithmetic rewrite (10 * 60) is rejected identically"
rm -rf "$d"

# ── 4. ...and the same value under the threshold is still rejected, because A1
#      never looks at the value. This is what makes the 60 s threshold harmless.
d="$(make_fixture '            .timeout(Duration::from_secs(5))')"
assert_exit "$d" 1 "A1 does not consult the threshold"
rm -rf "$d"

# ── 5. The compliant shapes must NOT fire. Measured in the real tree at
#      openai.rs:185 and ollama.rs:233 — a rule reading "no inline
#      Duration::from_* at a bounding site" would accuse both on its first day
#      (plan § Fire-Disposition, surface 1; AC9).
d="$(make_fixture '            .timeout(Duration::from_secs(budget.http_timeout_secs()))')"
assert_exit "$d" 0 "A1 admits a resolver-derived budget (AC9)"
rm -rf "$d"

d="$(make_fixture '            .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))')"
assert_exit "$d" 0 "A1 admits a named-constant budget"
rm -rf "$d"

d="$(make_fixture '            .timeout(timeout)')"
assert_exit "$d" 0 "A1 admits a variable budget"
rm -rf "$d"

# ── 6. Test code is not production budget. `with_timeout(…, from_secs(2))` in a
#      `#[cfg(test)]` block asserts the override works; accusing it would make
#      the guard red at birth on client.rs's own test module.
d="$(make_fixture '#[cfg(test)]
mod tests {
    #[test]
    fn t() {
        let c = A2aClient::with_timeout("http://x", None, Duration::from_secs(120));
    }
}')"
assert_exit "$d" 0 "#[cfg(test)] blocks are out of the live population"
rm -rf "$d"

# ── 6b. ...and the skip must END with the block, or one test module blinds the
#       rest of the file.
d="$(make_fixture '#[cfg(test)]
mod tests {
    fn t() { let _ = Duration::from_secs(120); }
}

fn live() {
    let http = builder().timeout(Duration::from_secs(900)).build();
}')"
assert_exit "$d" 1 "the cfg(test) skip ends with its block"
rm -rf "$d"

# ── 7. Rule A2: a const budget >= 60 s with no env behind it.
d="$(make_fixture 'const SOME_BUDGET: Duration = Duration::from_secs(600);')"
assert_exit "$d" 1 "A2: a non-DEFAULT const >= threshold is rejected"
rm -rf "$d"

d="$(make_fixture 'const SOME_BUDGET: Duration = Duration::from_secs(10 * 60);')"
assert_exit "$d" 1 "A2: the arithmetic rewrite is evaluated, not matched (AC3)"
rm -rf "$d"

# ── 8. ...and a DEFAULT_* whose file declares its env var is the compliant
#      shape — client.rs:24 is the model.
d="$(make_fixture 'pub const TIMEOUT_ENV: &str = "MIKA_PROBE_TIMEOUT_SECS";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);')"
assert_exit "$d" 0 "A2 admits DEFAULT_* backed by a declared env var"
rm -rf "$d"

# ── 8b. DEFAULT_* alone is not enough: the name without the env var is exactly
#       the "hardcoded budget wearing a default's clothes" this rule is about.
d="$(make_fixture 'pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);')"
assert_exit "$d" 1 "A2: DEFAULT_* without a declared env var is rejected"
rm -rf "$d"

# ── 9. Below the threshold is out of A2's population (RECOVERY_TIMEOUT = 30 s).
d="$(make_fixture 'pub const RECOVERY_TIMEOUT: Duration = Duration::from_secs(30);')"
assert_exit "$d" 0 "A2 ignores a const below the threshold"
rm -rf "$d"

# ── 10. An expression the guard cannot evaluate fails EXPLICITLY (AC3). A guard
#       that silently skips what it cannot read has a hole shaped like it.
d="$(make_fixture 'const DERIVED: Duration = Duration::from_secs(BASE_SECS * 60);')"
assert_exit "$d" 1 "A2: an unevaluable expression fails explicitly, not silently"
assert_says "$d" "cannot evaluate" "A2: the unevaluable-expression message names the reason"
rm -rf "$d"

# ── 11. A Duration constructor outside this guard's reach is refused rather
#       than admitted in silence (plan § refusals, point 8).
d="$(make_fixture 'const SHORT: Duration = Duration::from_millis(500);')"
assert_exit "$d" 1 "A2: an uncovered Duration constructor is refused explicitly"
rm -rf "$d"

# ── 12. The allowlist suppresses a real violation...
d="$(make_fixture 'const SOME_BUDGET: Duration = Duration::from_secs(600);')"
mkdir -p "$d/scripts"
printf '%s\n' 'crates/mika-a2a/src/probe.rs:SOME_BUDGET  # measured: pinned by an external protocol' \
    > "$d/scripts/a2a-timeout-allowlist.txt"
assert_exit "$d" 0 "allowlist suppresses a justified A2 violation"
rm -rf "$d"

# ── 13. ...and is compared in BOTH directions: an entry outliving its site
#       fails the build (V4, AC4). One-way comparison is how an allowlist
#       becomes the junk drawer of § M1.
d="$(make_fixture 'pub const TIMEOUT_ENV: &str = "MIKA_PROBE_TIMEOUT_SECS";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);')"
mkdir -p "$d/scripts"
printf '%s\n' 'crates/mika-a2a/src/probe.rs:GONE_LONG_AGO  # stale' \
    > "$d/scripts/a2a-timeout-allowlist.txt"
assert_exit "$d" 1 "a stale allowlist entry fails the build (AC4)"
assert_says "$d" "matches no A2 violation" "the stale-entry message names the reason"
rm -rf "$d"

# ── 14. A scan whose perimeter does not exist is a vacuous pass, and a vacuous
#       pass is the decoration mika#2103 is about.
d="$(mktemp -d)"
assert_exit "$d" 1 "an empty perimeter is refused, not silently passed"
rm -rf "$d"

echo ""
echo "check-a2a-timeout-literals anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
