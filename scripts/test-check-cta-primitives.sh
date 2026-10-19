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

# Twin of make_page_with_button, one directory over: the body lands in the
# PRODUCER package instead of a consumer surface (mika#1799 D1). $2 names the
# file, so the same content can be dropped as `Probe.tsx` or `Probe.test.tsx`
# and the perimeter's test-file filter measured by the difference.
#
# The migrated <Button> on `dashboard/src` is not decoration: without it, the
# filtered case (N8b) would have an empty CTA population and its green would be
# the anti-vacuity exit 3 rather than a clean scan — the two states this harness
# exists to keep apart.
make_ui_component() {
    local dir name
    name="${2:-Probe.tsx}"
    dir="$(mktemp -d)"
    mkdir -p "$dir/packages/ui/src/components" "$dir/dashboard/src/pages"
    printf '%s\n' "$1" > "$dir/packages/ui/src/components/$name"
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
#      The count, decomposed and measured 2026-10-19 — pinned because understood,
#      which is the difference between a harness and a clipboard:
#
#        10  the founding population across `dashboard/src` + `site/src`
#       + 3  `ErrorState.tsx`, the three actions migrated by mika#1799 D4
#       + 1  `SecretField.tsx:111`
#       ────
#         14
#
#      `packages/ui/src` holds 15 non-test `<Button` occurrences by raw grep and
#      contributes only 4. TEN are in comments — 4 in `Button.tsx`'s JSDoc, both
#      of `Spinner.tsx`, 2 of `SecretField.tsx` (one JSDoc, one JSX comment), and
#      one each in `AuthCard.tsx` and `Stepper.tsx`; comments are not code
#      (section 8 below pins that). The ELEVENTH is `Record<ButtonSize` at
#      `Button.tsx:85`, a type, which the identifier boundary rejects. So a
#      primitive documenting its own call sites does not inflate this number. The other 53 raw occurrences live in `Button.test.tsx` (26),
#      `Button.type-assertions.tsx` (25), `Spinner.test.tsx` and
#      `SecretField.test.tsx`: those are what the perimeter filter removes, and
#      what would otherwise move this assertion on every test added to <Button>.
assert_exit "$REPO_ROOT" 0 "real tree is clean (V6)"
assert_says "$REPO_ROOT" "14 CTA element(s)" "V6: all fourteen CTAs are still found, via <Button>"
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

# ── 5e. N5 (mika#1799 D2) — a gradient WITH STOPS is the most CTA fill there is.
#       `bg-gradient-to-r` alone paints nothing, which is why 5d above is green
#       and why the exclusion was written. Accompanied by a `from-*` stop it
#       paints §2's signature texture — the one the rulebook prescribes for a
#       primary — and the founding state said so itself: "Ten CTAs … none of them
#       a gradient". The case did not exist, so it fell into the neighbouring
#       bucket, and `ErrorState.tsx:46` lived there for three weeks.
#
#       The reported token is the STOP, not the word `bg-gradient-to-r`: the stop
#       is what names the colour that stopped tracking §2, and it is the string a
#       reader will search the file for.
d="$(make_page_with_button '      <button onClick={retry} className="px-4 py-1.5 rounded-lg bg-gradient-to-r from-accent to-accent-light text-white">Retry</button>')"
assert_exit "$d" 1 "N5: a gradient with colour stops is a CTA fill"
assert_says "$d" "from-accent" "N5: the failure names the stop, not the gradient utility"
rm -rf "$d"

# ── 5e'. N5b — Tailwind v4 renames `bg-gradient-to-*` to `bg-linear-*`. Both
#        spellings coexist in the wild, so the rule carries both; pinning one
#        would make the guard green on the day the package upgrades.
d="$(make_page_with_button '      <button className="px-4 py-2 bg-linear-to-br from-[#7c6af7] to-accent">Promote</button>')"
assert_exit "$d" 1 "N5b: the v4 bg-linear-* spelling is caught too"
assert_says "$d" "from-[#7c6af7]" "N5b: the stop is reported even as an arbitrary hex"
rm -rf "$d"

# ── 5f. N6 (mika#1799 D3) — a STATE ROLE is not an action. `<LiveRefreshToggle>`
#       is a canonical primitive of this very package, and its switch track
#       paints `bg-accent` to say WHICH STATE IT IS IN, not to call to action.
#       "Solid fill on a <button>" cannot tell the two apart, because the
#       distinction is not in the fill — it is in the role.
#
#       Closed by the predicate and never by the allowlist, for the reason this
#       guard's own header gives: "un faux positif ici coûte une PR bloquée et se
#       règle en mettant le lint en sourdine, ce qui est le mode de défaillance à
#       fermer en premier."
d="$(make_page_with_button '      <button role="switch" aria-checked={isLive} className="relative w-9 h-5 rounded-full bg-accent">
        <span className="absolute top-0.5 w-4 h-4 rounded-full bg-white" />
      </button>')"
assert_exit "$d" 0 "N6: a role=\"switch\" track is state, not a CTA"
rm -rf "$d"

# ── 5f'. N6b — ...and the exclusion is a CLOSED list of selected-state roles.
#        `role="button"` and `role="link"` are CTAs that gave themselves an
#        explicit role; the scan must keep seeing them. Without this case, D3
#        would be indistinguishable from "any tag carrying a role is exempt",
#        which is the form that would quietly let the next CTA through.
d="$(make_page_with_button '      <button role="button" className="px-4 py-2 rounded-lg bg-accent text-white">Promote</button>')"
assert_exit "$d" 1 "N6b: role=\"button\" is not a state role"
rm -rf "$d"

# ── 5f''. N6c — the role must be a quoted LITERAL. A real CTA whose class list
#         merely compares a role (`role === "tab"`) or whose title mentions one
#         must stay in the population: otherwise exclusion 4 is an escape hatch
#         anyone can open by accident.
d="$(make_page_with_button '      <button className={cn("px-4 bg-accent", role === "tab" && "ring")}>Go</button>
      <button title="role=tab" className="px-4 bg-accent">Go</button>')"
assert_exit "$d" 1 "N6c: a role comparison or a prose mention does not exempt a CTA"
assert_says "$d" "Found 2 hand-rolled" "N6c: both tags stay in the population"
rm -rf "$d"

# ── 5g. N7 — the gradient rule's natural false positive is TEXT in a gradient
#       (`bg-clip-text text-transparent` + stops), which is banal on a landing.
#       Measured at zero occurrences across the three surfaces before the rule
#       was written — but a scan whose restraint nobody has watched is
#       indistinguishable from a scan that fires on every `from-`, and the second
#       form would be muted within the week.
d="$(make_page_with_button '      <h1 className="bg-clip-text text-transparent bg-gradient-to-r from-accent to-accent-light">Mika</h1>
      <button className="bg-clip-text px-2">x</button>')"
assert_exit "$d" 0 "N7: gradient text on a non-interactive tag, and bg-clip-text alone, stay green"
rm -rf "$d"

# ── 5h. N8 (mika#1799 D1) — the perimeter reaches the PRODUCER package. LC.2
#       armed this scan on the two consuming surfaces and not on the package that
#       defines `<Button>` — the one place the primitive can contradict itself
#       without anything saying so, and the place it did.
d="$(make_ui_component '      <button onClick={go} className="px-3 py-2 rounded-lg bg-accent text-white">Search</button>')"
assert_exit "$d" 1 "N8: a hand-rolled CTA inside packages/ui is rejected"
assert_says "$d" "packages/ui/src/components/Probe.tsx" "N8: the failure names the producer path"
rm -rf "$d"

# ── 5h'. N8b — ...and stops at its test fixtures. A fixture is not a rendered
#        surface, and without the filter the anti-vacuity count below swings by 53
#        call sites (`Button.test.tsx` alone holds 26) and moves on every test
#        added to `<Button>`. A harness that goes red on healthy work is a harness
#        somebody disarms.
d="$(make_ui_component '      <button onClick={go} className="px-3 py-2 rounded-lg bg-accent text-white">Search</button>' 'Probe.test.tsx')"
assert_exit "$d" 0 "N8b: a test fixture inside packages/ui is outside the perimeter"
rm -rf "$d"

# ── 5i. N9 — exclusions 1 and 2 stay dominant over the new gradient case. A
#       variant-prefixed stop and a translucent stop are the same grammar they
#       always were; the gradient rule must not reach past them.
d="$(make_page_with_button '      <button className="px-4 py-2 bg-gradient-to-r hover:from-accent hover:to-accent-light">x</button>')"
assert_exit "$d" 0 "N9: a variant-prefixed gradient stop is not a fill (exclusion 2)"
rm -rf "$d"

d="$(make_page_with_button '      <button className="px-4 py-2 bg-gradient-to-r from-accent/40 to-accent-light/40">x</button>')"
assert_exit "$d" 0 "N9: a translucent gradient stop is not a fill (exclusion 1)"
rm -rf "$d"

# ── 5j. N7b — gradient TEXT on the very tags the scan evaluates. N7 puts it on
#       an <h1>, which the scan never looks at; the realistic case is a landing
#       link whose label is clipped to the gradient. `bg-clip-text` paints the
#       glyphs, not a fill, so it stays exclusion 3 whatever its stops say.
d="$(make_page_with_button '      <a href="/x" className="bg-clip-text text-transparent bg-gradient-to-r from-accent to-accent-light">Docs</a>
      <button className="bg-clip-text text-transparent bg-linear-to-br from-accent to-accent-light">x</button>')"
assert_exit "$d" 0 "N7b: gradient text on an <a> or a <button> is not a fill"
rm -rf "$d"

# ── 5k. N9b — exclusion 3 stays dominant over the stop, as it does over the
#        solid fill: a stop that names no colour or a surface tier paints no CTA
#        (a fade-out sheen on a ghost button is the realistic shape).
d="$(make_page_with_button '      <button className="bg-gradient-to-r from-transparent via-white/10 to-transparent">x</button>
      <button className="bg-gradient-to-b from-surface-container to-bg">y</button>')"
assert_exit "$d" 0 "N9b: from-transparent and surface-tier stops are not fills (exclusion 3)"
rm -rf "$d"

# ── 5l. N5c — a stop POSITION is not a colour stop. `from-10%` must not be the
#        token reported; the colour stop after it is.
d="$(make_page_with_button '      <button className="bg-gradient-to-r from-10% from-accent to-accent-light">Go</button>')"
assert_exit "$d" 1 "N5c: a gradient with a positioned stop is still a fill"
assert_says "$d" "painting from-accent" "N5c: the colour stop is reported, not the position"
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
