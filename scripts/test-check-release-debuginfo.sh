#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-release-debuginfo.sh (mika#1719).
#
# "Delete the thing the test protects; confirm the test goes red."
#
# The guard asserts two clauses of `[profile.release]`. A two-term conjunction
# passes for the wrong reason in several ways, and each has a fixture here:
#
#   - by only testing `strip != true`          → N4 (`strip = "symbols"`)
#   - by treating a missing `debug` as neutral → N2 (the naive-predicate trap)
#   - by only refusing `debug = false`         → N3 (`debug = 0`)
#   - by reddening on ANY `strip = true`       → N5 (`[profile.bench]`)
#   - by parsing no block at all               → N6 (profile absent)
#
# N5 and N6 are the two that matter most: they are the only ones that separate a
# guard that LOOKS AT SOMETHING from a guard that passes by accident. Without N5,
# "the scan reads the right block" is indistinguishable from "the scan reddens on
# every `strip = true`". Without N6, a scan gone blind returns 0.
#
# Fixtures are shaped on the REAL manifest — the same `lto`/`codegen-units` keys,
# the same comment style — never on an invented form.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-release-debuginfo.sh"

PASS=0
FAIL=0

FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

EMPTY_ALLOWLIST="$FIXTURE_ROOT/empty-allowlist.txt"
printf '# no entries\n' > "$EMPTY_ALLOWLIST"

# Write a fixture manifest named `Cargo.toml` (the basename is the allowlist key,
# so it must be the real one) and echo its path.
make_manifest() {
    local body="$1" dir
    dir="$(mktemp -d -p "$FIXTURE_ROOT")"
    printf '%s\n' "$body" > "$dir/Cargo.toml"
    echo "$dir/Cargo.toml"
}

assert_exit() {
    local manifest="$1" allowlist="$2" want="$3" name="$4"
    local got=0
    bash "$GUARD" "$manifest" "$allowlist" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

assert_output_contains() {
    local manifest="$1" allowlist="$2" needle="$3" name="$4"
    local out
    out="$(bash "$GUARD" "$manifest" "$allowlist" 2>&1 || true)"
    if [[ "$out" == *"$needle"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not contain: $needle)"
        FAIL=$((FAIL + 1))
    fi
}

# The conforming shape, as shipped.
CONFORMING='[workspace]
members = ["crates/mika-agent"]

[profile.release]
lto = true
codegen-units = 1
strip = "none"
debug = "line-tables-only"'

echo "── Positive control: the real tree ─────────────────────────────────────"

# The guard must be green on the manifest this repo actually ships. If this
# fails, everything below is measuring a fixture and nothing else.
assert_exit "$REPO_ROOT/Cargo.toml" "$REPO_ROOT/scripts/release-debuginfo-allowlist.txt" 0 \
    "P0: the repo's own Cargo.toml is conforming"

assert_exit "$(make_manifest "$CONFORMING")" "$EMPTY_ALLOWLIST" 0 \
    "P1: conforming fixture passes"

echo "── Negative controls: the six fixtures of the plan ─────────────────────"

# N1 — term A bites. `strip = true` removes what `debug` emitted.
N1="$(make_manifest '[profile.release]
lto = true
codegen-units = 1
strip = true
debug = "line-tables-only"')"
assert_exit "$N1" "$EMPTY_ALLOWLIST" 1 "N1: strip = true is refused"
assert_output_contains "$N1" "$EMPTY_ALLOWLIST" 'strip = "none"' \
    "N1: the refusal names its remedy, not only its fault"

# N2 — term B bites. THE TRAP: `strip = "none"` alone looks sufficient and is
# not. A release profile defaults to `debug = 0`, so there is nothing to keep.
N2="$(make_manifest "$(printf '[profile.release]\nlto = true\ncodegen-units = 1\nstrip = "none"')")"
assert_exit "$N2" "$EMPTY_ALLOWLIST" 1 "N2: strip alone, no debug key, is refused"
assert_output_contains "$N2" "$EMPTY_ALLOWLIST" 'defaults to `debug = 0`' \
    "N2: the refusal explains WHY the absent key is the defect"

# N3 — term B is positive, not a denylist of `false`.
N3="$(make_manifest '[profile.release]
strip = "none"
debug = 0')"
assert_exit "$N3" "$EMPTY_ALLOWLIST" 1 "N3: debug = 0 is refused"

# N4 — term A is not an equality test against `true`. `strip = "symbols"` is
# equivalent to `true` and would sail past a denylist.
N4="$(make_manifest '[profile.release]
strip = "symbols"
debug = "line-tables-only"')"
assert_exit "$N4" "$EMPTY_ALLOWLIST" 1 "N4: strip = \"symbols\" is refused"

# `strip = "debuginfo"` is the same class: it leaves `.symtab` (names) but no
# `file:line`, which is the half of the mika#1719 chain that stayed open longest.
N4b="$(make_manifest '[profile.release]
strip = "debuginfo"
debug = "line-tables-only"')"
assert_exit "$N4b" "$EMPTY_ALLOWLIST" 1 "N4b: strip = \"debuginfo\" is refused (names without file:line)"

# N5 — THE BOUND. A conforming release block plus a `[profile.bench]` carrying
# `strip = true` must be GREEN. Without this fixture, "the scan reads the right
# block" cannot be told apart from "the scan reddens on any `strip = true`".
N5="$(make_manifest '[profile.release]
lto = true
codegen-units = 1
strip = "none"
debug = "line-tables-only"

[profile.bench]
strip = true

[profile.dev]
debug = 0')"
assert_exit "$N5" "$EMPTY_ALLOWLIST" 0 "N5: a stripped [profile.bench] does not redden a conforming release"

# N6 — ANTI-VACUITY. No `[profile.release]` at all must exit 3, never 0.
N6="$(make_manifest '[workspace]
members = ["crates/mika-agent"]

[profile.bench]
strip = "none"
debug = "line-tables-only"')"
assert_exit "$N6" "$EMPTY_ALLOWLIST" 3 "N6: a missing [profile.release] exits 3, never 0"
assert_output_contains "$N6" "$EMPTY_ALLOWLIST" "nothing to check" \
    "N6: the refusal says the guard is blind rather than clean"

echo "── Shapes the guard must refuse rather than half-audit ─────────────────"

# An empty block carries neither key: passing on it would report the absence of
# a measurement as the presence of the setting.
EMPTY_BLOCK="$(make_manifest '[profile.release]

[profile.bench]
strip = true')"
assert_exit "$EMPTY_BLOCK" "$EMPTY_ALLOWLIST" 3 "V1: an empty [profile.release] exits 3"

# Two tables, or two assignments of one key: TOML rejects both, and a scan
# reading two values for one key cannot say which builds the binary.
DUP_TABLE="$(make_manifest '[profile.release]
strip = "none"
debug = "line-tables-only"

[profile.release]
strip = true')"
assert_exit "$DUP_TABLE" "$EMPTY_ALLOWLIST" 3 "V2: a duplicated [profile.release] exits 3"

DUP_KEY="$(make_manifest '[profile.release]
strip = "none"
strip = true
debug = "line-tables-only"')"
assert_exit "$DUP_KEY" "$EMPTY_ALLOWLIST" 3 'V3: a duplicated strip key exits 3'

assert_exit "$FIXTURE_ROOT/does-not-exist/Cargo.toml" "$EMPTY_ALLOWLIST" 2 \
    "V4: an unreadable manifest exits 2, never 0"

echo "── Accepted spellings (a guard too narrow gets disarmed) ───────────────"

# `strip` absent is CONFORMING: cargo strips nothing by default. A guard that
# demanded the literal key would redden a correct manifest.
assert_exit "$(make_manifest '[profile.release]
debug = "line-tables-only"')" "$EMPTY_ALLOWLIST" 0 "A1: strip absent (cargo default) passes"

for spelling in "'none'" '"none"' 'false'; do
    assert_exit "$(make_manifest "[profile.release]
strip = $spelling
debug = \"line-tables-only\"")" "$EMPTY_ALLOWLIST" 0 "A2: strip = $spelling passes"
done

for spelling in '"line-tables-only"' "'line-tables-only'" '"limited"' '"full"' 'true' '1' '2'; do
    assert_exit "$(make_manifest "[profile.release]
strip = \"none\"
debug = $spelling")" "$EMPTY_ALLOWLIST" 0 "A3: debug = $spelling passes"
done

# Trailing comments are the shipped shape: the real manifest carries one on each
# clause. A parser that swallowed the comment into the value would redden it.
assert_exit "$(make_manifest '[profile.release]
strip = "none"   # mika#1719 — keep what debug emits
debug = "line-tables-only"  # mika#1719 — emit it')" "$EMPTY_ALLOWLIST" 0 \
    "A4: trailing comments on both clauses pass"

echo "── The allowlist, in both directions ───────────────────────────────────"

# Direction 1: an entry naming a real violation exempts it. Without this, the
# mechanism could be inert and the empty shipped list would hide it.
ALLOW_STRIP="$FIXTURE_ROOT/allow-strip.txt"
printf 'Cargo.toml:strip  # mika#0000 — fixture\n' > "$ALLOW_STRIP"
assert_exit "$(make_manifest '[profile.release]
strip = true
debug = "line-tables-only"')" "$ALLOW_STRIP" 0 "L1: an entry naming a real violation exempts it"

# Direction 2 — THE SELF-CLEANING ASSERTION. The same entry against a CONFORMING
# manifest must go red: an exemption that outlives its motive silently exempts a
# future namesake.
assert_exit "$(make_manifest "$CONFORMING")" "$ALLOW_STRIP" 1 \
    "L2: a stale entry reddens a conforming manifest"
assert_output_contains "$(make_manifest "$CONFORMING")" "$ALLOW_STRIP" "stale allowlist entry" \
    "L2: the refusal names the entry as stale"

# The Fire-Disposition of the plan: the SHIPPED allowlist is empty. An entry
# added here without a reviewer seeing it is the failure mode this pins.
SHIPPED_ENTRIES="$(grep -vE '^[[:space:]]*(#|$)' "$REPO_ROOT/scripts/release-debuginfo-allowlist.txt" | grep -c . || true)"
if [ "$SHIPPED_ENTRIES" -eq 0 ]; then
    echo "PASS: L3: the shipped allowlist is empty (Fire-Disposition (a), mika#2306)"
    PASS=$((PASS + 1))
else
    echo "FAIL: L3: the shipped allowlist carries $SHIPPED_ENTRIES entry(ies); it ships EMPTY by construction"
    echo "      When the guard fires, repair the profile — do not add a line (mika#2201)."
    FAIL=$((FAIL + 1))
fi

echo ""
echo "── Result ──────────────────────────────────────────────────────────────"
echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
