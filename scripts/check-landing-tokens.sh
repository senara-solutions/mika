#!/usr/bin/env bash
# CI lint: the landing (`site/`) carries no colour literal of its own.
#
# THE PROPERTY: every colour the landing renders comes from a design token, and
#   the tokens come from `@samidarko/ui/theme.css` — the rulebook §2 palette.
#   A hex literal or a decimal `rgb()` component triple written into `site/src/`
#   or `site/index.html` is a colour that stops tracking the rulebook the moment
#   the rulebook moves. That is the whole of the defect this guard closes.
#
# THE FOUNDING STATE, measured 2026-09-20 (mika#1804, LC.5 of milestone #1799).
#   `site/src/index.css` declared six colour tokens. Four of them were, word for
#   word, the rulebook's "Override" row (`docs/design/luminescent-core.md:74`) —
#   a row that §2's own Full Token Reference table contradicts, and that LC.1
#   (mika#1800) resolved *against* for `packages/ui`. The landing was therefore
#   not a rogue fork: it was a surface LC.1 never reached. The drift had three
#   channels, and only the first was visible from the ticket:
#
#     1. the six `@theme` declarations in `site/src/index.css`;
#     2. `rgba(124,106,247, …)` — the decimal form of the legacy accent `#7c6af7`
#        — hardcoded in six components (Hero, HowItWorks, OpenSource, Features,
#        Teams, Nav);
#     3. `<body class="bg-[#0d0f12] text-[#a0a8b8]">` in `site/index.html`.
#
#   Closing channel 1 alone would have produced a landing with two different
#   purples and a page background out of tune with its own cards — worse than
#   the state it replaced, which was at least coherent. This guard therefore
#   covers all three, and it covers them by shape rather than by enumeration.
#
# WHY THE RULE IS "NO LITERAL" AND NOT "NOT THESE SEVEN LITERALS".
#   `packages/ui/src/theme.test.ts` bans a named list of superseded values, and
#   that is right for a file whose entire content is the token table. Here the
#   population is application code, where the realistic regression is not
#   someone retyping `#7c6af7` — it is someone writing `bg-[#8b5cf6]` in good
#   faith because it looked close enough. A denylist is green on that commit.
#   So the rule is: no colour literal at all, minus a named allowlist.
#
#   The seven values that list already carries stay worth naming, because they
#   are the ones whose reintroduction would be a *regression* rather than a new
#   drift:
#     #7c6af7  legacy accent    — superseded by canonical primary   #ada3ff
#     #0d0f12  legacy bg        — superseded by canonical background #0c0e11
#     #151820  legacy bg-card   — superseded by surface_container   #171a1d
#     #1e2130  legacy container — superseded by canonical           #1d2024
#     #e8ecf2  legacy heading   — superseded by on_surface          #e8e8ec
#     #a0a8b8  legacy muted     — superseded by on_surface_variant  #aaabaf
#     #ef4444  §5.5 error hex   — resolved to §2 canonical          #ff6e84
#   They need no special case: the shape rule already refuses them.
#
# THE ALLOWLIST IS BY VALUE, NEVER BY FILE. Excluding `Hero.tsx` and `Teams.tsx`
#   — the two files that hold the exception — would have opened a real hole:
#   both are also among the six components that carried channel 2. The named
#   limit, accepted: a future `#ff5f57` written somewhere that is *not* a window
#   pip would pass. That is bounded by these three values not being plausible
#   brand colours. See § Fire-Disposition of the plan for the full reasoning,
#   including why no follow-up tracker and no self-cleaning assertion are
#   attached to this exception (there is no resolution path to track and no
#   obsolescence condition to assert — macOS will not stop drawing its pips).
#
# COMMENTS ARE NOT CODE. A superseded value *mentioned* in a comment is
#   documentation, not a regression — the same rule `theme.test.ts:24-29` had to
#   adopt. A guard that cannot tell the two apart makes it impossible to write
#   down what was fixed.
#
# Usage: check-landing-tokens.sh [repo-root]
#   Defaults to the repo's real root. An explicit argument lets the
#   anti-vacuity harness point the guard at fixtures (see
#   scripts/test-check-landing-tokens.sh).
#
# Exit 0 clean, 1 on a violation, 3 if the perimeter does not exist at all (a
# guard that finds nothing to scan must say so rather than pass).

set -euo pipefail

DEFAULT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCAN_ROOT="${1:-$DEFAULT_ROOT}"

SRC_DIR="$SCAN_ROOT/site/src"
INDEX_HTML="$SCAN_ROOT/site/index.html"

# ── Named exceptions, by value, with the reason at the same place as the value.
#
# macOS window-chrome pips inside a terminal mockup (`Hero.tsx`, `Teams.tsx`).
# These are not brand palette: they are a visual quotation of a third-party
# chrome. Aligning them on canon would destroy the quotation, which is the
# component's entire intent.
ALLOWED_HEX="ff5f57 febc2e 28c840"
ALLOWED_REASON="macOS window-chrome pip in a terminal mockup — a quotation of third-party chrome, not brand palette"

# ── Build the file list.
FILES=""
if [[ -d "$SRC_DIR" ]]; then
    FILES="$(find "$SRC_DIR" -type f \( -name '*.ts' -o -name '*.tsx' -o -name '*.js' -o -name '*.jsx' -o -name '*.css' -o -name '*.html' \) | sort)"
fi
if [[ -f "$INDEX_HTML" ]]; then
    FILES="$FILES
$INDEX_HTML"
fi

FILES="$(printf '%s\n' "$FILES" | grep . || true)"

if [[ -z "$FILES" ]]; then
    echo "ERROR: nothing to scan — neither $SRC_DIR nor $INDEX_HTML exists." >&2
    echo "A guard with an empty perimeter has checked nothing, which is not the same" >&2
    echo "as a guard that found nothing wrong." >&2
    exit 3
fi

# ── Scan.
#
# awk strips comments before looking for colours, tracking `/* */` and `<!-- -->`
# across lines. A `//` is only a comment when it is not preceded by `:` — the
# landing is full of `https://github.com/...`.
#
# A hex colour is a `#` followed by exactly 3, 4, 6 or 8 hex digits and then a
# non-alphanumeric boundary. The boundary is what keeps `href="#features"` out
# of the population: the run of hex digits there is `fea`, a valid 3-digit
# length, but the character after it is `t` — so the match is read as the head
# of an identifier and rejected, rather than accepted as a shorthand colour.
VIOLATIONS="$(printf '%s\n' "$FILES" | while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    awk -v allowed="$ALLOWED_HEX" -v fname="$f" '
        BEGIN {
            n = split(allowed, a, " ")
            for (i = 1; i <= n; i++) { ok[tolower(a[i])] = 1 }
        }
        {
            line = $0

            # Continue an open block comment.
            if (in_block) {
                if (match(line, /\*\//)) { line = substr(line, RSTART + 2); in_block = 0 }
                else { next }
            }
            if (in_html) {
                if (match(line, /-->/)) { line = substr(line, RSTART + 3); in_html = 0 }
                else { next }
            }

            # Strip closed block comments, then open an unterminated one.
            while (match(line, /\/\*[^*]*\*+([^\/*][^*]*\*+)*\//)) {
                line = substr(line, 1, RSTART - 1) substr(line, RSTART + RLENGTH)
            }
            if (match(line, /\/\*/)) { line = substr(line, 1, RSTART - 1); in_block = 1 }

            while (match(line, /<!--.*-->/)) {
                line = substr(line, 1, RSTART - 1) substr(line, RSTART + RLENGTH)
            }
            if (match(line, /<!--/)) { line = substr(line, 1, RSTART - 1); in_html = 1 }

            # Line comment, but never a URL scheme separator. The cursor walks
            # forward by absolute offset rather than rewriting the string: a
            # scan that edits what it is scanning has to be proved to shrink it,
            # and this one does not need to.
            probe = line
            offset = 0
            while (match(probe, /\/\//)) {
                pos = offset + RSTART
                if (pos > 1 && substr(line, pos - 1, 1) == ":") {
                    offset = pos + 1
                    probe = substr(line, offset + 1)
                } else {
                    line = substr(line, 1, pos - 1)
                    break
                }
            }

            # ── Rule 1: colour hex literals.
            rest = line
            while (match(rest, /#[0-9a-fA-F]+/)) {
                run = substr(rest, RSTART + 1, RLENGTH - 1)
                after = substr(rest, RSTART + RLENGTH, 1)
                rest = substr(rest, RSTART + RLENGTH)

                # A longer alphanumeric run is an identifier, not a colour.
                if (after ~ /[0-9a-zA-Z]/) { continue }

                L = length(run)
                if (L != 3 && L != 4 && L != 6 && L != 8) { continue }
                if (tolower(run) in ok) { continue }

                printf "%s:%d: colour literal #%s\n", fname, FNR, run
            }

            # ── Rule 2: the legacy accent in decimal form — channel 2 of the
            # founding drift. `rgba(124,106,247, …)` is `#7c6af7` wearing a
            # different notation, and a guard that only reads hex is green on it.
            if (line ~ /124[ \t]*,[ \t]*106[ \t]*,[ \t]*247/) {
                printf "%s:%d: legacy accent in decimal form (124,106,247 == #7c6af7)\n", fname, FNR
            }
        }
    ' "$f"
done)"

if [[ -n "$VIOLATIONS" ]]; then
    printf '%s\n' "$VIOLATIONS"
    COUNT="$(printf '%s\n' "$VIOLATIONS" | grep -c .)"
    echo ""
    echo "Found $COUNT hardcoded colour(s) in the landing."
    echo ""
    echo "The landing consumes the canonical palette from \`@samidarko/ui/theme.css\`"
    echo "(rulebook §2). Every colour it renders must come from a token, so that a"
    echo "rulebook change reaches this surface without anyone remembering to."
    echo ""
    echo "Fix, by shape:"
    echo "  utility class     bg-[#7c6af7]                  ->  bg-accent"
    echo "                    text-[#a0a8b8]                ->  text-muted"
    echo "  shadow / gradient rgba(124,106,247,0.12)        ->  color-mix(in srgb, var(--color-accent) 12%, transparent)"
    echo "  in a class        shadow-[0_0_30px_rgba(...)]   ->  shadow-[0_0_30px_color-mix(in_srgb,var(--color-accent)_8%,transparent)]"
    echo ""
    echo "Token names available on this surface: bg, bg-card, accent, accent-light,"
    echo "heading, muted (backward-compat aliases), plus the full canonical set"
    echo "(primary, surface-container, on-surface, on-surface-variant, ...)."
    echo "See packages/ui/src/theme.css."
    echo ""
    echo "If the colour genuinely is not brand palette — a quotation of some"
    echo "third-party chrome, as the macOS window pips are — add its value to"
    echo "ALLOWED_HEX in this script WITH its reason, and say so in the PR body."
    echo "Never allowlist a file: two of the files holding the current exception"
    echo "also carried the drift this guard exists to catch."
    exit 1
fi

FILE_COUNT="$(printf '%s\n' "$FILES" | grep -c .)"
echo "Landing carries no colour literal outside the named allowlist"
echo "  scanned:   $FILE_COUNT file(s) under site/"
echo "  allowed:   $ALLOWED_HEX"
echo "  reason:    $ALLOWED_REASON"
exit 0
