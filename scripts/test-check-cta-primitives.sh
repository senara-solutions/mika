#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-cta-primitives.sh (mika#1801, LC.2).
#
# House constraint, written into ci.yml since mika#2103: *a guard nobody has
# watched go red is a decoration*. The founding incident there is a lint that
# could not fail on the defect it existed to catch, and stayed green through 26
# production panics.
#
# The plan's Fire-Disposition names four fixtures (N1-N4) and says why two of them
# must be seen GREEN: "sans elles, « le scan attrape les CTA » est indistinguable
# de « le scan rougit sur toute occurrence de bg-accent », et la seconde forme
# serait mise en sourdine dans la semaine." This file is those four plus the
# controls the implementation found it needed — the brace-depth case, the
# comment-stripping case, the allowlist's two directions, and the two ways the
# scan can end up looking at nothing.
#
# "Delete the thing the test protects; confirm the test goes red."

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-cta-primitives.sh"
REAL_ALLOWLIST="$REPO_ROOT/scripts/cta-primitives-allowlist.txt"

PASS=0
FAIL=0

# Run the guard against scan root $1 (allowlist $4, optional); assert exit == $2.
assert_exit() {
    local root="$1" want="$2" name="$3" allow="${4:-$REAL_ALLOWLIST}"
    local got=0
    bash "$GUARD" "$root" "$allow" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

# Assert the guard's output on root $1 contains $2.
#
# Captured and matched with `case`, not piped into `grep -q`: under `pipefail` a
# failing producer poisons the pipeline's status even when grep matches, and
# `echo | grep -q` is itself refused by the sigpipe lint (mika#2055).
assert_says() {
    local root="$1" want="$2" name="$3" allow="${4:-$REAL_ALLOWLIST}"
    local out
    out="$(bash "$GUARD" "$root" "$allow" 2>&1 || true)"
    if [[ "$out" == *"$want"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not mention '$want')"
        FAIL=$((FAIL + 1))
    fi
}

# A throwaway tree holding one dashboard component whose body is $1.
make_page() {
    local dir
    dir="$(mktemp -d)"
    mkdir -p "$dir/dashboard/src/pages"
    printf '%s\n' "$1" > "$dir/dashboard/src/pages/Probe.tsx"
    echo "$dir"
}

# Same, plus a second file holding one migrated <Button> so the tree has a
# non-empty CTA population and the anti-vacuity exit cannot mask a missed
# violation. Every red case below uses this, or the failure would be ambiguous
# between "violation found" (1) and "nothing found" (3).
make_page_with_button() {
    local dir
    dir="$(make_page "$1")"
    printf '%s\n' '        <Button onClick={() => {}}>Search</Button>' \
        > "$dir/dashboard/src/pages/Migrated.tsx"
    echo "$dir"
}

make_allowlist() {
    local f
    f="$(mktemp)"
    printf '%s\n' "# fixture allowlist" "$1" > "$f"
    echo "$f"
}

# ── 1. The real, migrated tree is clean, and the count is not zero (V6). This is
#      what makes every red case below mean something: a lint that starts red
#      proves nothing when you see it red.
assert_exit "$REPO_ROOT" 0 "real tree is clean (V6)"
assert_says "$REPO_ROOT" "10 CTA element(s)" "V6: all ten CTAs are still found, via <Button>"
assert_says "$REPO_ROOT" "0 hand-rolled" "V6: none of them is hand-rolled"

# ── 2. N1 — the base case. A solid fill on a <button>, all on one line.
d="$(make_page_with_button '      <button onClick={go} className="px-3 py-2 rounded-lg bg-accent text-white">Search</button>')"
assert_exit "$d" 1 "N1: a hand-rolled CTA on one line is rejected"
assert_says "$d" "Probe.tsx:1" "N1: the failure names the file and the line"
assert_says "$d" "bg-accent" "N1: the failure names the offending token"
rm -rf "$d"

# ── 3. N2 — the same button with its className on a LATER line. This is the shape
#      most of the tree actually had: `Traces.tsx` opened its `<button` on line 38
#      and wrote its `className` on line 41. A line-anchored predicate would have
#      missed the majority of its own population (mika#2496 term 1).
d="$(make_page_with_button '      <button
        onClick={go}
        className="px-3 py-2 rounded-lg bg-accent text-white"
      >
        Search
      </button>')"
assert_exit "$d" 1 "N2: the unit of analysis is the element, not the line"
assert_says "$d" "Probe.tsx:1" "N2: the violation is reported at the tag's opening line"
rm -rf "$d"

# ── 3b. ...and the walk to the closing `>` must survive a `>` inside an attribute.
#       Without brace-depth tracking the tag "ends" at the arrow of `() =>` and the
#       className is never seen — a silent false negative on the single most common
#       attribute in the tree.
d="$(make_page_with_button '      <button
        onClick={() => {
          setTraceSearch("")
          setSearchParams(new URLSearchParams())
        }}
        className="px-3 py-2 rounded-lg bg-accent text-white"
      >
        Clear
      </button>')"
assert_exit "$d" 1 "N2b: an arrow function in an attribute does not end the tag"
rm -rf "$d"

# ── 4. N3 — the non-interactive population must stay GREEN. These are the real
#      shapes: `Nav.tsx`'s brand dot, `HowItWorks.tsx`'s step number,
#      `Features.tsx`'s icon tile. Without this case, "the scan catches CTAs" is
#      indistinguishable from "the scan fires on every `bg-accent`", and the second
#      form would be muted within the week.
d="$(make_page_with_button '      <span className="inline-block h-1.5 w-1.5 rounded-full bg-accent" />
      <div className="flex h-16 w-16 items-center justify-center rounded-full bg-accent text-2xl font-black text-white" />
      <div className="h-3 w-3 rounded-full bg-blue-400" />')"
assert_exit "$d" 0 "N3: non-interactive elements are outside the population"
rm -rf "$d"

# ── 5. N4 — a translucent fill is a chip, not a CTA (`Tasks.tsx:250`).
d="$(make_page_with_button '      <span className="text-xs bg-accent/10 text-accent px-2 py-0.5 rounded-full">3</span>
      <button className="p-1 rounded hover:bg-accent/10 text-muted/30">x</button>')"
assert_exit "$d" 0 "N4: bg-accent/N is a chip fill, not a CTA"
rm -rf "$d"

# ── 5b. Exclusion 2: a variant-prefixed fill is the icon-button and menu-item
#       grammar. ~20 of the dashboard's 34 non-CTA buttons have this shape.
d="$(make_page_with_button '      <button className="p-1 rounded hover:bg-accent transition-colors">x</button>
      <button className="px-2 group-hover:bg-accent disabled:bg-accent">y</button>')"
assert_exit "$d" 0 "exclusion 2: a hover-only fill is not a CTA"
rm -rf "$d"

# ── 5c. Exclusion 3: a control painted at a surface tier is a card-like
#       affordance. `LlmCallDetail.tsx:148` is exactly this.
d="$(make_page_with_button '      <button className="flex items-center justify-between p-3 rounded-xl bg-bg hover:bg-white/[0.03]">row</button>
      <button className="bg-bg-card border border-white/[0.05] rounded-2xl">card</button>
      <button className="bg-surface-container-high p-2">tier</button>
      <button className="bg-transparent p-2">none</button>')"
assert_exit "$d" 0 "exclusion 3: surface tiers and bg-transparent are not CTA fills"
rm -rf "$d"

# ── 5d. Non-colour `background-*` utilities start with `bg-` too. Without the
#       shape exclusions these would each read as a fill.
d="$(make_page_with_button '      <button className="bg-clip-text bg-gradient-to-r bg-cover bg-no-repeat bg-center bg-blend-multiply">x</button>')"
assert_exit "$d" 0 "non-colour background-* utilities are not fills"
rm -rf "$d"

# ── 6. The rule is SHAPE, not a denylist of the ten measured classes. This is the
#      realistic regression: nobody retypes `bg-green-600`, they write a colour
#      that looked close enough on a new button.
d="$(make_page_with_button '      <button className="px-4 py-2 rounded-lg bg-indigo-600 text-white">Promote</button>')"
assert_exit "$d" 1 "an off-token colour never seen before is still rejected"
rm -rf "$d"

d="$(make_page_with_button '      <button className="px-4 py-2 rounded-lg bg-[#7c6af7] text-white">Promote</button>')"
assert_exit "$d" 1 "an arbitrary hex fill is rejected"
rm -rf "$d"

# ── 6b. ...and `bg-white`, which is what the plan's own eight-row table missed in
#       `OpenSource.tsx` and which §7 forbids by name.
d="$(make_page_with_button '      <a href={GITHUB_URL} className="rounded-xl bg-white px-7 py-3 font-semibold text-bg">Star on GitHub</a>')"
assert_exit "$d" 1 "a solid bg-white fill on a link is rejected"
rm -rf "$d"

# ── 7. `<a>` is in the population, `<article>` and `<aside>` are not. Without the
#      boundary term the scan would treat every article wrapper as a link.
d="$(make_page_with_button '      <article className="bg-accent p-4">post</article>
      <aside className="bg-accent p-4">note</aside>')"
assert_exit "$d" 0 "<article> and <aside> do not match <a"
rm -rf "$d"

# ── 8. Comments are not code. `SkillVariants.tsx` explains its own migration in a
#      JSX comment that names <Button>; counting that would inflate the population
#      the anti-vacuity check reads, and quoting a violation in a comment would
#      accuse the documentation (the mika#2050 false positive, one guard over).
d="$(make_page '      {/* Was <button className="bg-accent">, now a <Button> — see mika#1801. */}
      <Button onClick={() => {}}>Search</Button>')"
assert_exit "$d" 0 "a violation quoted in a JSX comment is not a violation"
assert_says "$d" "1 CTA element(s)" "the <Button> named in a comment is not counted twice"
rm -rf "$d"

# ── 8b. ...and the comment skip must END with its block, or one comment blinds the
#       rest of the file. Same shape as the cases in test-check-landing-tokens.sh.
d="$(make_page_with_button '      {/* historical: this used to be bg-green-600 */}
      <button className="px-4 py-2 bg-accent text-white">Search</button>')"
assert_exit "$d" 1 "the comment skip ends with its block"
rm -rf "$d"

# ── 9. The allowlist suppresses a named (path, token) pair...
d="$(make_page_with_button '      <button className="px-4 py-2 rounded-lg bg-indigo-600 text-white">Promote</button>')"
a="$(make_allowlist 'dashboard/src/pages/Probe.tsx  bg-indigo-600  # fixture')"
assert_exit "$d" 0 "an allowlisted (path, token) pair is suppressed" "$a"

# ── 9b. ...by TOKEN as well as path, so allowlisting one site cannot blanket the
#       file. This is the hole check-landing-tokens.sh refuses by name.
a2="$(make_allowlist 'dashboard/src/pages/Probe.tsx  bg-teal-500  # wrong token')"
assert_exit "$d" 1 "the allowlist does not match on path alone" "$a2"
rm -f "$a" "$a2"
rm -rf "$d"

# ── 9c. BOTH DIRECTIONS. An entry matching no real violation fails the build, so
#       an exemption cannot outlive its cause and silently cover a namesake later.
d="$(make_page_with_button '      <Button onClick={() => {}}>Clean</Button>')"
a="$(make_allowlist 'dashboard/src/pages/Probe.tsx  bg-indigo-600  # already migrated')"
assert_exit "$d" 1 "a stale allowlist entry fails the build" "$a"
assert_says "$d" "Stale allowlist" "the stale-entry failure says so" "$a"
rm -f "$a"
rm -rf "$d"

# ── 10. The shipped allowlist is EMPTY, and pinned empty. Adding the first entry
#       means deleting this assertion first — taking the decision explicitly, in
#       the same diff, rather than accumulating it (§ Fire-Disposition (3)).
ALLOW_LINES="$(sed 's/#.*//' "$REAL_ALLOWLIST" | grep -c '[^[:space:]]' || true)"
if [ "$ALLOW_LINES" -eq 0 ]; then
    echo "PASS: V8 — the shipped allowlist has zero non-comment lines"
    PASS=$((PASS + 1))
else
    echo "FAIL: V8 — the shipped allowlist has $ALLOW_LINES entr(y|ies), expected 0"
    FAIL=$((FAIL + 1))
fi

# ── 11. A scan whose perimeter does not exist is a vacuous pass, and a vacuous
#       pass is the decoration mika#2103 is about.
d="$(mktemp -d)"
assert_exit "$d" 3 "an absent perimeter is refused, not silently passed"
assert_says "$d" "nothing to scan" "the empty-perimeter message names the reason"
rm -rf "$d"

# ── 11b. ...and so is a perimeter that exists but holds no CTA at all. This is the
#        state a renamed directory, a changed extension or a predicate narrowed
#        until it matches nothing all produce, and it exits 0 on every other check
#        here — exactly how a silently inert guard reads like a healthy one
#        (mika#2205, mika#2496 term 5).
d="$(make_page '      <div className="p-4">nothing interactive here</div>')"
assert_exit "$d" 3 "a perimeter with zero CTA elements is refused"
assert_says "$d" "ZERO CTA elements" "the zero-population message names the reason"
rm -rf "$d"

# ── 12. The failure text must name the fix, or it sends the next implementer back
#       to a literal — and it must name the allowlist's doctrine, or the first
#       person to hit it adds a line.
d="$(make_page_with_button '      <button className="px-4 py-2 bg-accent text-white">Search</button>')"
assert_says "$d" "as=\"link\"" "the failure text names the link form of the fix"
assert_says "$d" "DO NOT add a line" "the failure text names the allowlist doctrine"
rm -rf "$d"

echo ""
echo "check-cta-primitives anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
