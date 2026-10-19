#!/usr/bin/env bash
# CI lint: no hand-rolled CTA in `dashboard/src`, `site/src` or `packages/ui/src`
# (mika#1801, LC.2; perimeter and predicate extended by mika#1799).
#
# THE PROPERTY: every call-to-action on every surface is rendered by
#   `<Button>` from `@samidarko/ui`. A `<button>` or `<a href>` that paints its
#   own solid background is a CTA that has stopped tracking rulebook §5 — and,
#   since §2's gradient is the one texture §5 prescribes for a primary, it is
#   also a CTA that silently opts out of the one visual signature the design
#   system has.
#
# THE FOUNDING STATE, measured 2026-09-27. Ten CTAs across two surfaces, all
#   hand-rolled, none of them a gradient:
#
#     dashboard/src/pages/Timeline.tsx        bg-accent            primary
#     dashboard/src/pages/Traces.tsx          bg-accent            primary
#     dashboard/src/pages/SkillVariants.tsx   bg-green-600         primary (off-token)
#     dashboard/src/pages/SkillVariants.tsx   border-white/[0.1]   secondary
#     dashboard/src/pages/SkillVariants.tsx   (text only)          tertiary
#     site/src/components/Nav.tsx             bg-accent            primary
#     site/src/components/Hero.tsx            bg-accent            primary
#     site/src/components/Hero.tsx            bg-white/[0.03]      secondary
#     site/src/components/OpenSource.tsx      bg-white             primary (§7 violation)
#     site/src/components/OpenSource.tsx      bg-white/[0.03]      secondary
#
#   The plan's own table had eight rows and called itself exhaustive; the last
#   two were found by writing this scan. That is the argument for the scan
#   existing at all, and it is why its count is checked rather than assumed.
#
# THE UNIT OF ANALYSIS IS THE JSX ELEMENT, NEVER THE LINE. Before migration,
#   `Traces.tsx` opened its `<button` on line 38 and wrote its `className` on
#   line 41; the same shape holds for most of the tree. A line-anchored predicate
#   would have missed the majority of its own population — the trap mika#2496
#   had to name as term 1 for `dispatch-lib.sh`. So the scan joins each opening
#   tag to its attributes before evaluating, and it walks forward tracking quote
#   state AND brace depth, because real attribute lists contain `>`:
#
#       <button
#         onClick={() => {          <-- a naive "scan to the first >" stops HERE
#           setTraceSearch('')
#         }}
#         className="..."
#       >
#
# WHY THE PERIMETER INCLUDES THE PRODUCER PACKAGE (mika#1799 D1). LC.2 armed this
#   scan on the two *consuming* surfaces and not on `packages/ui/src`, the package
#   that defines `<Button>`. That is the natural choice while migrating ten CTAs
#   out of consumers — and it is also the one place the primitive could contradict
#   itself with nothing to say so. It did: `ErrorState.tsx:46` shipped a
#   hand-rolled gradient button inside the package whose whole job is to make that
#   impossible, and the rulebook (§5.5 l.272) names *that* button as the place of
#   its own visual signature.
#
#   TEST AND TYPE-ASSERTION FILES ARE OUT OF THE PERIMETER — `*.test.tsx`,
#   `*.test.jsx`, `*.type-assertions.tsx`. Two reasons, in this order. A fixture is
#   not a rendered surface, and this scan exists to hold what a user sees. And
#   without the filter the anti-vacuity count below swings by 53 call sites
#   (`Button.test.tsx` holds 26, `Button.type-assertions.tsx` 25) and then moves
#   on every test added to `<Button>` — a harness that goes red on healthy work is
#   a harness somebody disarms. Measured neutral on the pre-existing perimeter:
#   the five `.test.tsx` under `dashboard/src` hold zero CTA elements.
#
# WHY THE PREDICATE IS "SOLID FILL", AND WHAT THAT DELIBERATELY LEAVES OUT.
#   A CTA is defined by the fill it paints at rest. Four exclusions, each
#   costing a false negative rather than buying a false positive:
#
#     1. Translucent backgrounds (`bg-accent/10`, `bg-white/[0.03]`) are out.
#        They are the chip, hover and ghost grammar the dashboard is full of —
#        and also the shape of a hand-rolled *secondary*, which therefore passes.
#        Catching it would mean firing on `border` + padding, which is
#        indistinguishable from the 34 non-CTA buttons in the dashboard.
#     2. Variant-prefixed backgrounds (`hover:bg-accent`, `group-hover:bg-accent`)
#        are out. They are icon buttons and menu items; a CTA whose fill exists
#        only on hover would pass, which is implausible enough to accept.
#     3. Surface tokens (`bg-bg-card`, `bg-surface-container`) are out. A control
#        painted at a surface tier is a card-like affordance, not a CTA.
#     4. A SELECTED-STATE ROLE is out, whatever it paints (mika#1799 D3).
#        `role="switch"`, `checkbox`, `radio`, `tab`, `menuitemcheckbox`,
#        `menuitemradio`. These ARIA roles carry a selected state (`aria-checked`,
#        `aria-selected`), and such a control paints its background to say WHICH
#        STATE IT IS IN, not to call to action. The population measured on the
#        three surfaces is one — `LiveRefreshToggle.tsx`'s switch track, itself a
#        canonical primitive of this package — and the distinction it needs is not
#        in the fill, so no fill-shaped predicate could ever have made it.
#        `role="button"` and `role="link"` are emphatically NOT on the list: those
#        are CTAs that gave themselves an explicit role, and the scan must keep
#        seeing them. Written as a closed list of roles rather than as
#        `# LiveRefreshToggle exception`, because the realistic regression is not
#        somebody rewriting that toggle — it is somebody writing the next
#        `role="tab"` with its active tab in `bg-accent`.
#
#   Exclusion 1 is the one that matters: a false positive here costs a blocked PR
#   and is settled by muting the lint, which is the failure mode to close first.
#   Exclusion 4 exists for the same reason, paid in advance.
#
# ...AND WHY A GRADIENT IS A FILL AFTER ALL (mika#1799 D2). Exclusion 3 lumps
#   `bg-gradient-*` / `bg-linear-*` in with `bg-clip-text`, `bg-origin-border` and
#   `bg-blend-multiply` — utilities that genuinely paint nothing. For three of
#   them that is permanently true. For the gradient it is true only while it has
#   no stops: `bg-gradient-to-r` ACCOMPANIED BY a `from-*` stop paints the single
#   most CTA fill there is — it is literally the texture §2 prescribes for a
#   primary. The exclusion was born of a founding state that says so in its own
#   words (line 12 above): "Ten CTAs … none of them a gradient". The case did not
#   exist, so it fell into the neighbouring bucket and stayed there.
#
#   So a gradient utility leaves the exclusion the moment the same tag carries a
#   colour stop, and THE TOKEN REPORTED IS THE STOP (`from-accent`), never the
#   word `bg-gradient-to-r`: the stop is what names the colour that stopped
#   tracking §2, and it is the string a reader will search the file for.
#
#   Exclusions 1, 2 and 3 stay dominant over it — `from-accent/40` is still
#   translucent, `hover:from-accent` is still variant-prefixed, a stop that names
#   no colour or a surface tier (`from-transparent`, `from-surface-container`)
#   still paints no CTA, and `bg-clip-text` — alone, or clipping a gradient to the
#   glyphs of a link or button — is still nothing. A stop position (`from-10%`)
#   is skipped so the colour stop after it is the one reported. `bg-radial-*` / `bg-conic-*` stay unconditionally
#   excluded: their population here is zero and mika#1799 scoped them out
#   deliberately rather than ship a shape no fixture has watched go red.
#
#   The false positive this rule naturally invites is text in a gradient
#   (`bg-clip-text text-transparent` + stops), banal on a landing page. Measured
#   at zero occurrences across the three surfaces before the rule was written, and
#   held green by fixture N7 — because a scan whose restraint nobody has watched is
#   indistinguishable from a scan that fires on every `from-`.
#
# WHY THE RULE IS "NO SOLID FILL" AND NOT "NOT THESE TEN CLASSES". Same reasoning
#   as `check-landing-tokens.sh`: the realistic regression is nobody retyping
#   `bg-green-600` — it is someone writing `bg-indigo-600`, or `bg-[#7c6af7]`, in
#   good faith on a new button. A denylist is green on that commit. So the rule is
#   shape, minus a named allowlist that ships EMPTY.
#
# COMMENTS ARE NOT CODE. `SkillVariants.tsx` explains its own migration in a JSX
#   comment that names `<Button>`; counting that as a call site would inflate the
#   population the anti-vacuity check reads. Stripping comments is also what keeps
#   this file's own header out of any scan that ever points at `scripts/`.
#
# Usage: check-cta-primitives.sh [repo-root]
#   Defaults to the repo's real root. An explicit argument lets the anti-vacuity
#   harness point the scan at fixtures (see scripts/test-check-cta-primitives.sh).
#
# Exit 0 clean, 1 on a violation or a stale allowlist entry, 3 when the scan has
# checked nothing (absent perimeter, or zero CTA elements found — a scan with an
# empty population must say so rather than pass).

set -euo pipefail

DEFAULT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCAN_ROOT="${1:-$DEFAULT_ROOT}"
ALLOWLIST="${2:-$DEFAULT_ROOT/scripts/cta-primitives-allowlist.txt}"

SCAN_DIRS="dashboard/src site/src packages/ui/src"

# ── Build the file list. Test and type-assertion files are excluded: a fixture is
# not a rendered surface, and leaving them in makes the anti-vacuity count below
# move on every test added to `<Button>` (header § WHY THE PERIMETER…).
FILES=""
for d in $SCAN_DIRS; do
    if [[ -d "$SCAN_ROOT/$d" ]]; then
        FILES="$FILES
$(find "$SCAN_ROOT/$d" -type f \( -name '*.tsx' -o -name '*.jsx' \) \
    ! -name '*.test.tsx' ! -name '*.test.jsx' ! -name '*.type-assertions.tsx' \
    | sort)"
    fi
done
FILES="$(printf '%s\n' "$FILES" | grep . || true)"

if [[ -z "$FILES" ]]; then
    echo "ERROR: nothing to scan — none of $SCAN_DIRS exists under $SCAN_ROOT." >&2
    echo "A guard with an empty perimeter has checked nothing, which is not the same" >&2
    echo "as a guard that found nothing wrong." >&2
    exit 3
fi

# ── Scan. One awk invocation per file; emits two kinds of line:
#      V<TAB>path:line<TAB>token<TAB>element   a violation candidate
#      C<TAB>n                                 count of <Button> call sites
SCAN_OUTPUT="$(printf '%s\n' "$FILES" | while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    rel="${f#"$SCAN_ROOT"/}"
    awk -v fname="$rel" '
        # ── Phase 1: strip comments, preserving line structure.
        #
        # One left-to-right pass carrying `in_block` across lines, for the reason
        # `check-landing-tokens.sh` had to adopt it: successive substitutions must
        # choose an order, and every order is wrong somewhere. Stripping `/* */`
        # first reads the `/*` in `// see /* below` as a block opener and swallows
        # the rest of the file — a scan that silently stops looking, which is the
        # one failure worse than accusing wrongly.
        #
        # A `//` is only a line comment when not preceded by `:` — this tree is
        # full of `https://` URLs.
        {
            raw = $0
            out = ""
            i = 1
            n = length(raw)
            while (i <= n) {
                if (in_block) {
                    if (substr(raw, i, 2) == "*/") { in_block = 0; i += 2 } else { i++ }
                    continue
                }
                if (substr(raw, i, 2) == "/*") { in_block = 1; i += 2; continue }
                if (substr(raw, i, 2) == "//" && !(i > 1 && substr(raw, i - 1, 1) == ":")) {
                    break
                }
                out = out substr(raw, i, 1)
                i++
            }
            S[NR] = out
            LINES = NR
        }

        # ── Phase 2: walk the stripped text as one stream, tracking the line each
        # character came from, and pull out whole JSX opening tags.
        END {
            # Flatten, recording the line number of every character.
            stream = ""
            pos = 0
            for (ln = 1; ln <= LINES; ln++) {
                s = S[ln]
                for (k = 1; k <= length(s); k++) {
                    pos++
                    lineof[pos] = ln
                }
                stream = stream s
                # A newline stands in for the break so tokens cannot fuse across
                # lines (`<a` at end of line + `href` at start of the next).
                pos++
                lineof[pos] = ln
                stream = stream "\n"
            }
            total = pos

            # Exclusion 4 (mika#1799 D3). A dynamic regex rather than a literal,
            # so the single quote can be written as "\047" — this whole awk
            # program is a single-quoted shell string. The leading boundary keeps
            # `data-role=` out, the trailing one keeps `role="tabpanel"` and
            # `role="switcher"` out, and the loose run between `=` and the role
            # word admits `role={"switch"}` and `role = "switch"` alike. A
            # computed role (`role={x ? "switch" : "button"}`) does NOT match, so
            # it stays in the population — the conservative direction.
            STATE_ROLE = "[^A-Za-z0-9_-]role[ \t]*=[ \t]*[^A-Za-z0-9_]*" \
                "(menuitemcheckbox|menuitemradio|checkbox|switch|radio|tab)" \
                "[^A-Za-z0-9_-]"

            buttons = 0
            i = 1
            while (i <= total) {
                c = substr(stream, i, 1)
                if (c != "<") { i++; continue }

                rest = substr(stream, i)

                # `<Button` — the primitive. Counted, never a violation.
                if (rest ~ /^<Button[^A-Za-z0-9_]/) { buttons++; i++; continue }

                # `<button` / `<a` — the hand-rolled population. The trailing
                # character class is what keeps `<article>` and `<aside>` out.
                elem = ""
                if (rest ~ /^<button[^A-Za-z0-9_-]/) { elem = "button" }
                else if (rest ~ /^<a[^A-Za-z0-9_-]/)  { elem = "a" }
                if (elem == "") { i++; continue }

                startline = lineof[i]

                # Walk to the `>` that closes this opening tag, tracking quote
                # state and brace depth. Without the depth, `onClick={() => {}}`
                # ends the tag at the arrow.
                dq = 0; sq = 0; bt = 0; depth = 0
                j = i + 1
                tag = "<"
                while (j <= total) {
                    ch = substr(stream, j, 1)
                    tag = tag ch
                    # "\047" is the single quote. Written as an octal escape
                    # because this whole awk program is a single-quoted shell
                    # string, and a literal apostrophe would end it.
                    if (dq)      { if (ch == "\"")   dq = 0 }
                    else if (sq) { if (ch == "\047") sq = 0 }
                    else if (bt) { if (ch == "`")    bt = 0 }
                    else if (ch == "\"")   { dq = 1 }
                    else if (ch == "\047") { sq = 1 }
                    else if (ch == "`")    { bt = 1 }
                    else if (ch == "{")  { depth++ }
                    else if (ch == "}")  { if (depth > 0) depth-- }
                    else if (ch == ">" && depth == 0) { break }
                    j++
                }

                # ── Exclusion 4: a selected-state role carries state, not an
                # action, and leaves the population whatever it paints.
                if (tag ~ STATE_ROLE) { i = j + 1; continue }

                # ── A gradient utility paints nothing by itself — which is why
                # exclusion 3 below still skips it — but paints the signature
                # texture of §2 as soon as the same tag carries a colour stop.
                # Reported by its STOP, the token that names the colour
                # (mika#1799 D2). No apostrophe anywhere in this program: it is
                # one single-quoted shell string (see the "\047" note above).
                gradient = ""
                gscan = tag
                while (match(gscan, /bg-(gradient|linear)-[A-Za-z0-9_.,%#()\[\]-]+/)) {
                    gbefore = (RSTART > 1) ? substr(gscan, RSTART - 1, 1) : " "
                    gscan = substr(gscan, RSTART + RLENGTH)
                    # Exclusion 2 — `hover:bg-gradient-to-r` — and the tail of a
                    # longer identifier.
                    if (gbefore ~ /[:A-Za-z0-9_-]/) { continue }
                    gradient = "yes"
                    break
                }
                # Gradient TEXT paints no fill: an unprefixed `bg-clip-text` on
                # the same tag clips the gradient to the glyphs, so the tag is
                # still exclusion 3 whatever its stops say (fixture N7b).
                if (gradient != "" && tag ~ /(^|[^:A-Za-z0-9_-])bg-clip-text([^A-Za-z0-9_-]|$)/) {
                    gradient = ""
                }
                if (gradient != "") {
                    sscan = tag
                    while (match(sscan, /from-[A-Za-z0-9_.,%#()\[\]\/-]+/)) {
                        stop = substr(sscan, RSTART, RLENGTH)
                        sbefore = (RSTART > 1) ? substr(sscan, RSTART - 1, 1) : " "
                        sscan = substr(sscan, RSTART + RLENGTH)
                        # Exclusions 2 and 1 stay dominant: `hover:from-accent` is
                        # variant-prefixed, `from-accent/40` is translucent.
                        if (sbefore ~ /[:A-Za-z0-9_-]/) { continue }
                        if (stop ~ /\//) { continue }
                        # Exclusion 3, mirrored from the solid pass below: a
                        # stop that names no colour, or a surface tier, paints no
                        # CTA. A stop POSITION (`from-10%`) is not a colour stop
                        # at all — skip it so the colour stop after it is the one
                        # reported.
                        if (stop ~ /^from-(transparent|inherit|current|none)$/) { continue }
                        if (stop ~ /^from-(bg|bg-card|card|background|surface)(-[a-z-]+)?$/) { continue }
                        if (stop ~ /^from-[0-9.]+%$/) { continue }
                        printf "V\t%s:%d\t%s\t%s\n", fname, startline, stop, elem
                        break
                    }
                }

                # ── Evaluate the tag for a solid background fill.
                scan = tag
                while (match(scan, /bg-[A-Za-z0-9_.,%#()\[\]\/-]+/)) {
                    tok = substr(scan, RSTART, RLENGTH)
                    before = (RSTART > 1) ? substr(scan, RSTART - 1, 1) : " "
                    scan = substr(scan, RSTART + RLENGTH)

                    # A `:` immediately before is a Tailwind variant
                    # (`hover:`, `group-hover:`, `disabled:`) — exclusion 2.
                    # A letter or `-` before means this is the tail of a longer
                    # identifier, not a class of its own.
                    if (before ~ /[:A-Za-z0-9_-]/) { continue }

                    # Translucent — exclusion 1.
                    if (tok ~ /\//) { continue }

                    # Non-colour `background-*` utilities, and surface tiers —
                    # exclusion 3. Matched by shape, so a new surface tier needs
                    # no edit here.
                    if (tok ~ /^bg-(clip|origin|blend|gradient|linear|radial|conic)-/) { continue }
                    if (tok ~ /^bg-(auto|cover|contain|fixed|local|scroll|center|top|bottom|left|right)$/) { continue }
                    if (tok ~ /^bg-(no-)?repeat(-x|-y|-round|-space)?$/) { continue }
                    if (tok ~ /^bg-(transparent|inherit|current|none)$/) { continue }
                    if (tok ~ /^bg-(bg|bg-card|card|background|surface)(-[a-z-]+)?$/) { continue }

                    printf "V\t%s:%d\t%s\t%s\n", fname, startline, tok, elem
                }

                i = j + 1
            }
            printf "C\t%d\n", buttons
        }
    ' "$f"
done)"

RAW_VIOLATIONS="$(printf '%s\n' "$SCAN_OUTPUT" | grep '^V' || true)"
BUTTON_COUNT="$(printf '%s\n' "$SCAN_OUTPUT" | awk -F'\t' '$1 == "C" { n += $2 } END { print n + 0 }')"
FILE_COUNT="$(printf '%s\n' "$FILES" | grep -c .)"
RAW_COUNT="$(printf '%s\n' "$RAW_VIOLATIONS" | grep -c . || true)"

# ── Apply the allowlist, and hold it to account in BOTH directions.
#
# `<path>  <bg-token>  # reason` — path AND token, never a bare path. Allowlisting
# a whole file is precisely the hole `check-landing-tokens.sh` refuses: the file
# holding the exception is usually also the file that will carry the next drift.
ALLOW_ENTRIES=""
if [[ -f "$ALLOWLIST" ]]; then
    ALLOW_ENTRIES="$(sed 's/#.*//' "$ALLOWLIST" | awk 'NF >= 2 { print $1 "\t" $2 }' || true)"
fi
ALLOW_ENTRIES="$(printf '%s\n' "$ALLOW_ENTRIES" | grep . || true)"

VIOLATIONS=""
while IFS= read -r v; do
    [[ -z "$v" ]] && continue
    vpath="$(printf '%s' "$v" | awk -F'\t' '{ split($2, p, ":"); print p[1] }')"
    vtok="$(printf '%s' "$v" | awk -F'\t' '{ print $3 }')"
    matched=0
    while IFS= read -r a; do
        [[ -z "$a" ]] && continue
        apath="$(printf '%s' "$a" | awk -F'\t' '{ print $1 }')"
        atok="$(printf '%s' "$a" | awk -F'\t' '{ print $2 }')"
        if [[ "$vpath" == "$apath" && "$vtok" == "$atok" ]]; then
            matched=1
            USED_ENTRIES="${USED_ENTRIES:-}$apath	$atok
"
            break
        fi
    done <<< "$ALLOW_ENTRIES"
    if [[ "$matched" -eq 0 ]]; then
        VIOLATIONS="$VIOLATIONS$v
"
    fi
done <<< "$RAW_VIOLATIONS"
VIOLATIONS="$(printf '%s' "$VIOLATIONS" | grep . || true)"

# The self-cleaning half. An entry that matches no real violation is an exemption
# that outlived its cause: on the day the site is migrated the build goes red and
# the line must go — not months later (mika#1574 (a), mika#2520's precedent).
STALE=""
while IFS= read -r a; do
    [[ -z "$a" ]] && continue
    if [[ "${USED_ENTRIES:-}" != *"$a"* ]]; then
        STALE="$STALE$a
"
    fi
done <<< "$ALLOW_ENTRIES"
STALE="$(printf '%s' "$STALE" | grep . || true)"

# ── Anti-vacuity. A scan that found no CTA at all is not a clean tree: it is a
# scan looking at the wrong place — a renamed directory, a changed extension, a
# predicate narrowed until it matches nothing. That state exits 0 on every other
# check here, which is exactly how a silently inert guard reads like a healthy
# one (mika#2205, and mika#2496 term 5 for the same reason).
CTA_COUNT=$((BUTTON_COUNT + RAW_COUNT))
if [[ "$CTA_COUNT" -eq 0 ]]; then
    echo "ERROR: scanned $FILE_COUNT file(s) and found ZERO CTA elements." >&2
    echo "" >&2
    echo "CTAs are known to exist across dashboard/src, site/src and packages/ui/src," >&2
    echo "and after migration they are still CTAs — rendered by <Button> instead of by" >&2
    echo "hand. Zero means this scan is no longer looking at them: check the perimeter" >&2
    echo "($SCAN_DIRS), the file extensions, the test-file" >&2
    echo "filter, and the <Button> detection." >&2
    exit 3
fi

if [[ -n "$VIOLATIONS" ]]; then
    printf '%s\n' "$VIOLATIONS" | awk -F'\t' '{ printf "%s: hand-rolled CTA — <%s> painting %s\n", $2, $4, $3 }'
    COUNT="$(printf '%s\n' "$VIOLATIONS" | grep -c .)"
    echo ""
    echo "Found $COUNT hand-rolled CTA fill(s)."
    echo ""
    echo "Every CTA on every surface is rendered by <Button> from '@samidarko/ui',"
    echo "which implements rulebook §5 — including the §2 gradient that a flat fill"
    echo "cannot reproduce. Fix, by shape:"
    echo ""
    echo "  solid fill      <button className=\"... bg-accent ...\">   ->  <Button onClick={...}>"
    echo "  on a link       <a href=\"/x\" className=\"... bg-accent\">  ->  <Button as=\"link\" href=\"/x\">"
    echo "  ghost / border  border + translucent fill                 ->  <Button variant=\"secondary\">"
    echo "  text-only       no fill, low-priority action              ->  <Button variant=\"tertiary\">"
    echo ""
    echo "Landing-specific flourishes rulebook §5 does not describe (a lift on hover,"
    echo "a tinted glow) ride along on \`className\`. They are not variants."
    echo ""
    echo "DO NOT add a line to scripts/cta-primitives-allowlist.txt. That list ships"
    echo "empty and its header says why: when this scan fires, the resolution is to"
    echo "route the site to <Button>. A CTA you do not want to route is a CTA to"
    echo "delete (mika#2201 doctrine). If a surface genuinely needs a fill §5 does"
    echo "not offer, that is a rulebook §8 decision reserved to Vincent — it goes in"
    echo "docs/design/luminescent-core.md first, and into <Button> second."
    exit 1
fi

if [[ -n "$STALE" ]]; then
    echo "Stale allowlist entr(y|ies) in ${ALLOWLIST#"$DEFAULT_ROOT"/}:" >&2
    printf '%s\n' "$STALE" | awk -F'\t' '{ printf "  %s  %s\n", $1, $2 }' >&2
    echo "" >&2
    echo "Each of these names a hand-rolled CTA that no longer exists. The exemption" >&2
    echo "has outlived its cause: delete the line. The comparison runs in both" >&2
    echo "directions on purpose — otherwise an exemption would silently keep covering" >&2
    echo "a future namesake." >&2
    exit 1
fi

ALLOW_COUNT="$(printf '%s\n' "$ALLOW_ENTRIES" | grep -c . || true)"
echo "No hand-rolled CTA outside the allowlist"
echo "  scanned:   $FILE_COUNT file(s) under $SCAN_DIRS"
echo "  found:     $CTA_COUNT CTA element(s) — $BUTTON_COUNT via <Button>, $RAW_COUNT hand-rolled"
echo "  allowed:   $ALLOW_COUNT entr(y|ies)"
exit 0
