#!/usr/bin/env bash
# CI lint: no hand-rolled CTA in `dashboard/src` or `site/src` (mika#1801, LC.2).
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
# WHY THE PREDICATE IS "SOLID FILL", AND WHAT THAT DELIBERATELY LEAVES OUT.
#   A CTA is defined by the fill it paints at rest. Three exclusions, each
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
#
#   Exclusion 1 is the one that matters: a false positive here costs a blocked PR
#   and is settled by muting the lint, which is the failure mode to close first.
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

SCAN_DIRS="dashboard/src site/src"

# ── Build the file list.
FILES=""
for d in $SCAN_DIRS; do
    if [[ -d "$SCAN_ROOT/$d" ]]; then
        FILES="$FILES
$(find "$SCAN_ROOT/$d" -type f \( -name '*.tsx' -o -name '*.jsx' \) | sort)"
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
    echo "Ten CTAs are known to exist across dashboard/src and site/src, and after" >&2
    echo "migration they are still ten CTAs — rendered by <Button> instead of by hand." >&2
    echo "Zero means this scan is no longer looking at them: check the perimeter" >&2
    echo "($SCAN_DIRS), the file extensions, and the <Button> detection." >&2
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
