#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-landing-tokens.sh (mika#1804).
#
# House constraint, written into ci.yml since mika#2103: *a guard nobody has
# watched go red is a decoration*. The founding incident there is a lint that
# could not fail on the defect it existed to catch, and stayed green through 26
# production panics.
#
# The plan's verification contract makes the same point in its own words: "V5
# and V6 are the verifications that count — a lint you have not seen go red is
# not a lint, it is a line of CI." This file is those two checks in their
# durable form, plus the negative controls that matter just as much: the guard
# must NOT fire on the shapes this change deliberately ships (the prescribed
# `color-mix()` form, the allowlisted pips, a comment mentioning a superseded
# value, a URL, an HTML anchor).
#
# "Delete the thing the test protects; confirm the test goes red."

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-landing-tokens.sh"

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

# A throwaway tree holding one component under site/src whose body is $1.
make_component() {
    local dir
    dir="$(mktemp -d)"
    mkdir -p "$dir/site/src/components"
    printf '%s\n' "$1" > "$dir/site/src/components/Probe.tsx"
    echo "$dir"
}

# A throwaway tree holding only site/index.html, whose body is $1.
make_index_html() {
    local dir
    dir="$(mktemp -d)"
    mkdir -p "$dir/site"
    printf '%s\n' "$1" > "$dir/site/index.html"
    echo "$dir"
}

# ── 1. The real, swept tree is clean (V4). After this change the guard fires on
#      exactly zero lines of site/ — which is what makes every red case below
#      mean something. A lint that starts red proves nothing when you see it red.
assert_exit "$REPO_ROOT" 0 "real landing is clean (V4)"

# ── 2. V5: the regression this guard exists to catch. The legacy accent written
#      back into a component in good faith.
d="$(make_component '      <div className="bg-[#7c6af7] p-4" />')"
assert_exit "$d" 1 "V5: the legacy accent hex is rejected"
assert_says "$d" "Probe.tsx:1" "V5: the failure names the file and the line"
rm -rf "$d"

# ── 3. V6: channel 2 — the same colour in decimal notation. A guard that only
#      reads hex is green on the six occurrences that drift actually had.
d="$(make_component '        style={{ background: "linear-gradient(90deg, rgba(124,106,247,0.25), transparent)" }}')"
assert_exit "$d" 1 "V6: the legacy accent in decimal form is rejected"
assert_says "$d" "124,106,247" "V6: the failure names the decimal form"
rm -rf "$d"

# ── 3b. ...including with the spacing a formatter would produce. Retyping the
#       same triple with spaces must not be an escape hatch.
d="$(make_component '        style={{ background: "rgba(124, 106, 247, 0.25)" }}')"
assert_exit "$d" 1 "V6: the spaced decimal form is rejected identically"
rm -rf "$d"

# ── 4. The rule is SHAPE, not a denylist. This is the difference between this
#      guard and `theme.test.ts`'s banned-value list, and it is the realistic
#      regression: nobody retypes #7c6af7, they write a purple that looked close.
d="$(make_component '      <div className="bg-[#8b5cf6] p-4" />')"
assert_exit "$d" 1 "an off-token colour outside the legacy list is still rejected"
rm -rf "$d"

# ── 4b. ...at every accepted hex length, so a shorthand is not a way through.
d="$(make_component '      <div className="text-[#abc] p-4" />')"
assert_exit "$d" 1 "a 3-digit shorthand hex is rejected"
rm -rf "$d"

d="$(make_component '      <div className="text-[#aabbccdd] p-4" />')"
assert_exit "$d" 1 "an 8-digit hex with alpha is rejected"
rm -rf "$d"

# ── 5. The prescribed replacement must NOT fire. A guard that refuses the fix it
#      prescribes sends the next implementer back to a literal.
d="$(make_component '        style={{ background: "radial-gradient(ellipse at 50% 40%, color-mix(in srgb, var(--color-accent) 12%, transparent) 0%, transparent 70%)" }}')"
assert_exit "$d" 0 "the prescribed color-mix() form is admitted"
rm -rf "$d"

# ── 5b. ...including the Tailwind arbitrary-value spelling, where the underscores
#       are what the class needs and the parentheses are what the guard sees.
d="$(make_component '      <div className="hover:shadow-[0_0_30px_color-mix(in_srgb,var(--color-accent)_8%,transparent)]" />')"
assert_exit "$d" 0 "the Tailwind underscore spelling of color-mix() is admitted"
rm -rf "$d"

# ── 5c. Token utilities are the point of the exercise.
d="$(make_component '      <div className="bg-bg-card text-muted hover:border-accent/40" />')"
assert_exit "$d" 0 "token utilities are admitted"
rm -rf "$d"

# ── 6. The allowlist suppresses the three named pips (§ Fire-Disposition (ii)).
d="$(make_component '        <div className="h-3 w-3 rounded-full bg-[#ff5f57]/60" />
        <div className="h-3 w-3 rounded-full bg-[#febc2e]/60" />
        <div className="h-3 w-3 rounded-full bg-[#28c840]/60" />')"
assert_exit "$d" 0 "the three macOS window pips are allowlisted"
rm -rf "$d"

# ── 6b. ...and the allowlist is by VALUE, not by file. A fourth colour in the
#       same file as the pips is still refused — which is the property that
#       keeps Hero.tsx and Teams.tsx, the two files holding the exception AND
#       two of the six that carried channel 2, inside the guard's reach.
d="$(make_component '        <div className="bg-[#ff5f57]/60" />
        <div className="bg-[#7c6af7]" />')"
assert_exit "$d" 1 "a drifted colour beside an allowlisted pip is still rejected"
rm -rf "$d"

# ── 7. Comments are documentation, not regression (theme.test.ts:24-29 had to
#      adopt the same rule). Without this, writing down what was fixed is
#      impossible — including in this repo's own plan and script headers.
d="$(make_component '      // superseded: the landing used to render #7c6af7 here
      <div className="bg-accent" />')"
assert_exit "$d" 0 "a superseded value named in a line comment is not a violation"
rm -rf "$d"

d="$(make_component '      /* was #0d0f12, now --color-background #0c0e11 */
      <div className="bg-bg" />')"
assert_exit "$d" 0 "a superseded value named in a block comment is not a violation"
rm -rf "$d"

d="$(make_component '      /* channel 1 was:
           --color-accent: #7c6af7;
           --color-bg: #0d0f12;
      */
      <div className="bg-bg" />')"
assert_exit "$d" 0 "a multi-line block comment is skipped in full"
rm -rf "$d"

# ── 7b. ...and the skip must END with the block, or one comment blinds the rest
#       of the file. Same shape as the cfg(test) case in the a2a harness.
d="$(make_component '      /* historical note: #0d0f12 */
      <div className="bg-[#7c6af7]" />')"
assert_exit "$d" 1 "the comment skip ends with its block"
rm -rf "$d"

# ── 8. No false positive on the shapes the landing is actually full of. A `//`
#      inside a URL is not a line comment, and an anchor is not a colour — this
#      landing carries six of the first and five of the second.
d="$(make_component '      const GITHUB_URL = "https://github.com/senara-solutions/mika"; // ok
      <a href="#features">Features</a>
      <a href="#how-it-works">How</a>
      <div className="bg-[#7c6af7]" />')"
assert_exit "$d" 1 "a violation after a URL and an anchor is still seen"
assert_says "$d" "#7c6af7" "the URL does not swallow the rest of the line"
rm -rf "$d"

d="$(make_component '      const GITHUB_URL = "https://github.com/senara-solutions/mika";
      <a href="#features">Features</a>')"
assert_exit "$d" 0 "URLs and HTML anchors are not colour literals"
rm -rf "$d"

# ── 9. index.html is in the perimeter — channel 3 of the founding drift lived
#      there, on the page background itself, outside every component.
d="$(make_index_html '  <body class="bg-[#0d0f12] text-[#a0a8b8] antialiased">')"
assert_exit "$d" 1 "channel 3 (the <body> hexes) is inside the perimeter"
rm -rf "$d"

d="$(make_index_html '  <!-- was bg-[#0d0f12] -->
  <body class="bg-bg text-muted antialiased">')"
assert_exit "$d" 0 "an HTML comment naming the old value is not a violation"
rm -rf "$d"

# ── 10. A scan whose perimeter does not exist is a vacuous pass, and a vacuous
#       pass is the decoration mika#2103 is about.
d="$(mktemp -d)"
assert_exit "$d" 3 "an absent perimeter is refused, not silently passed"
assert_says "$d" "nothing to scan" "the empty-perimeter message names the reason"
rm -rf "$d"

echo ""
echo "check-landing-tokens anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
