#!/usr/bin/env bash
# CI lint (mika#2055 — SIGPIPE-under-pipefail structural guard):
# in shell under `scripts/**` and `skills/bundled/**`, a data producer
# (`printf`/`echo`) piped into a short-circuiting consumer (`grep` with a
# `-q` flag) is a latent bug. `grep -q` exits at the FIRST match and closes
# the pipe; the producer, still writing, takes SIGPIPE and exits 141; under
# `set -o pipefail` that 141 is promoted to the pipeline's status — so the
# test reports "absent" for a value that is present (a false failure), or
# masks a real one. Any such pipeline fails the build.
#
# TWO CLASSES THIS GUARD DID NOT SEE UNTIL mika#3130. Both were populated in the
# tree, and the second one went red in CI while sitting inside this scan field:
#
#   T1 — line continuation. The producer on one line, the `| grep -q` on the
#        next after a `\`. A line-at-a-time matcher sees neither half. This is
#        the shape of `test-dispatch-lib.sh`'s mika#2211 `APPENDED` assertion,
#        which failed the `Check` job on attempt 1 of run 43079844489 (PR #3075)
#        and passed on attempt 2 with an unchanged diff — a RE-TRIED run is the
#        signature of this fault.
#   T2 — intermediate link. The old middle class `[^|#]*` could cross neither an
#        intermediate `|` (`printf … | grep -v … | grep -qE …`) nor a literal `#`
#        inside a pipeline string (`'^[[:space:]]*#'`) nor a `|` enclosed in an
#        expansion (`"${result#*|}"`).
#
# Both are closed by normalising to LOGICAL lines and removing the end-of-line
# comment before matching a widened ERE — see `_logical_lines`,
# `_strip_eol_comment` and `FORBIDDEN` below.
#
# Still OUT of scope, deliberately: the producer vocabulary stays `printf|echo`.
# Other producers that can take SIGPIPE (`sed`, `git show`, `head`, `cat`, a
# shell function) are a distinct and far more populated class — roughly twenty
# sites — and sorting them needs a per-site judgement on whether the producer is
# bounded under the pipe buffer. Tracked in senara-solutions/mika#2983.
#
# Root-cause writeup:
#   docs/solutions/test-failures/bash-assert-sigpipe-and-host-coupling-before-ci-gate-2026-08-29.md
# Prior recurrence (n=2 in days) is the argument for a structural guard over
# prose — see docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md
#
# The remedy is a here-string, which has no pipeline and therefore no SIGPIPE:
#   BEFORE:  if printf '%s' "$haystack" | grep -qF "$needle"; then
#   AFTER:   if grep -qF -- "$needle" <<<"$haystack"; then
# The `--` matters independently: a needle beginning with `-` would otherwise
# be read as an option.
#
# Scope (why not gated on a literal in-file `set … pipefail`):
# a shell *library* like `skills/bundled/_shared/dispatch-lib.sh` does not set
# `pipefail` itself — it inherits it by being sourced into pipefail contexts
# (its test suite and the dispatch handlers). Gating on an in-file
# `set … pipefail` would miss exactly the file the mika#2055 measurement table
# flags as `pipefail: oui`. So this guard denies the shape across every bash
# file under the roots: any of them may set `pipefail` now or later, or be
# sourced into a pipefail context, and the here-string remedy is strictly
# equivalent and safe in a non-pipefail bash context too — you cannot decide
# pipefail-ness statically, so deny the fragile shape outright.
#
# One class is EXEMPT: a pure-POSIX `#!/bin/sh` script that never sets
# `pipefail`. `pipefail` is not POSIX and here-strings (`<<<`) are undefined in
# POSIX sh (shellcheck SC3011), so the shape is neither dangerous there nor
# fixable with the here-string remedy. `skills/bundled/address-pr-comments/handlers/run.sh`
# is the one such file (mika#2055 table: `pipefail: non`); it is skipped.
#
# Discipline analog (same shape, same "construis l'incapacité" doctrine):
#   scripts/verify-voice-non-transit.sh (#1796), scripts/check-byte-slices.sh
#   (#764), scripts/check-loop-select.sh (#848).
#
# Escape hatch (per line):
#   Append `# sigpipe-safe: #<ticket>` (e.g. `#2055`) and use it only for a
#   line that genuinely cannot SIGPIPE (e.g. a producer whose entire output is
#   smaller than a pipe buffer AND the consumer is documented not to
#   short-circuit). The ticket-citation form is enforced structurally: a bare
#   `# sigpipe-safe` marker WITHOUT a `#<digits>` citation does NOT suppress
#   the violation (protects against "add the marker to bypass the gate").
#   On a T1 (continuation) violation, the marker on ANY physical line of the
#   logical group suppresses — the exemption is read before the end-of-line
#   comment is removed (mika#3130).
#
# Exit codes:
#   0 — clean
#   1 — violation(s) found
#
# Usage: verify-no-sigpipe-grep.sh [ROOT]
#   ROOT defaults to the repo root. When ROOT contains scripts/ or
#   skills/bundled/, those are scanned; otherwise ROOT itself is scanned
#   (used by the anti-vacuity harness against a fixture tree).

set -euo pipefail

ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"

# The forbidden shape, as an ERE:
#   a producer word (printf|echo), then anything up to a pipe that stays in the
#   same COMMAND (no `;`, no `&` — so no `&&`, no `||`, no background), then
#   `grep` with a run of flag tokens the LAST of which carries a `q` (`-q`,
#   `-qF`, `-qiE`, `-Fq`…).
# The q-flag must appear before the (quoted) pattern, so a `-q`-shaped needle
# does not self-trigger.
#
# mika#3130: the middle class was `[^|#]*` — the producer had to sit
# IMMEDIATELY adjacent to the `grep -q`. That missed every pipeline with an
# intermediate link (`printf … | grep -v … | grep -qE …`), and every one whose
# `|` or `#` lives inside a string or an expansion (`"${result#*|}"`,
# `grep -v '^[[:space:]]*#'`). `[^;&]*` lets those through while keeping the
# producer and the `grep -q` in the SAME command: a `;`, a `&&`, a `||` or a
# `&` cuts the match, so `echo "msg"; cmd | grep -q x` is not flagged.
# Widening is only safe because the end-of-line comment is removed first — see
# `_strip_eol_comment` below.
FORBIDDEN='(^|[^[:alnum:]_.])(printf|echo)[^;&]*\|[[:space:]]*grep[[:space:]]+(-[A-Za-z]+[[:space:]]+)*-[A-Za-z]*q[A-Za-z]*([[:space:]]|$)'

# ── Normalisation, stage 1: physical lines → LOGICAL lines (mika#3130, class T1)
# A producer on one line and its `| grep -q` on the next, joined by a trailing
# `\`, is ONE pipeline — but a line-at-a-time matcher sees neither half. That is
# exactly the shape that went red on run 43079844489 (PR #3075) while living for
# months inside this guard's own scan field.
#
# Emits one record per logical line, `N:<joined text>` — the same format as
# `grep -n`, so the reading loop below is unchanged. N is the FIRST physical line
# of the group: the line the operator must go to. The `\` becomes a space, as the
# shell's own joining does for token purposes.
#
# A full-line comment is NEVER joined to the next line: in bash the comment
# swallows a trailing `\` (`# prose \` does not continue), so joining it would
# hide the following line from the matcher.
#
# The invariant that makes this stage safe the other way round: a continuation
# requires the `\` as the LAST character of the line, so a comment can never
# precede a continuation on the same physical line. Joining cannot glue code
# behind a `#`.
_logical_lines() {
    awk '
        function _trailing_bs(s,   n, i) {
            n = 0
            for (i = length(s); i >= 1; i--) {
                if (substr(s, i, 1) == "\\") n++; else break
            }
            return n
        }
        {
            if (buf == "") start = NR
            if (buf == "" && $0 ~ /^[[:space:]]*#/) { print start ":" $0; next }
            if (_trailing_bs($0) % 2 == 1) {
                buf = buf substr($0, 1, length($0) - 1) " "
                next
            }
            print start ":" buf $0
            buf = ""
        }
        END { if (buf != "") print start ":" buf }
    ' "$1"
}

# ── Normalisation, stage 2: drop the END-OF-LINE comment (mika#3130, no new FPs)
# Quote-aware: truncates at the first `#` that is outside quotes AND preceded by
# start-of-text or whitespace. This is what lets the ERE widen to `[^;&]*`
# without buying a false positive on prose that merely CITES the forbidden shape
# — while a `#` living inside a string stays visible, which the real population
# needs (`printf … | grep -v '^[[:space:]]*#' | grep -qE …`).
#
# ORDER IS IMPERATIVE: the `# sigpipe-safe: #<ticket>` escape IS an end-of-line
# comment, so the exemption filter runs BEFORE this stage (see the pipeline in
# the scan loop). That order also gives the T1 escape for free: a marker on ANY
# physical line of the group lands in the logical line and suppresses.
#
# Direction of error, said honestly: this stage only REMOVES text, so it can
# produce a false negative, never a false positive. The residual case is a
# MULTI-LINE string (no `\`) one of whose lines carries a `#`: the quote state,
# computed per logical line, is wrong there. Such a false negative would be on
# data text, not on an executed pipeline — and the pre-mika#3130 guard saw
# nothing of that class either. No occurrence in the current tree.
_strip_eol_comment() {
    awk '
        BEGIN { SQ = sprintf("%c", 39) }
        {
            p = index($0, ":")
            pre = substr($0, 1, p)
            s = substr($0, p + 1)
            out = ""; sq = 0; dq = 0; n = length(s)
            for (i = 1; i <= n; i++) {
                c = substr(s, i, 1)
                if (c == "\\" && !sq) {
                    out = out c
                    if (i < n) { i++; out = out substr(s, i, 1) }
                    continue
                }
                if (c == SQ && !dq) { sq = !sq; out = out c; continue }
                if (c == "\"" && !sq) { dq = !dq; out = out c; continue }
                if (c == "#" && !sq && !dq) {
                    prev = (i == 1) ? "" : substr(s, i - 1, 1)
                    if (prev == "" || prev == " " || prev == "\t") break
                }
                out = out c
            }
            print pre out
        }
    '
}

# The guard and its own test embed FORBIDDEN as a string literal; never scan
# them (they would self-match).
SELF="$(basename "$0")"
EXCLUDE_BASENAMES=("$SELF" "test-verify-no-sigpipe-grep.sh")

# Assemble the scan roots.
SCAN_DIRS=()
for sub in scripts skills/bundled; do
    [ -d "$ROOT/$sub" ] && SCAN_DIRS+=("$ROOT/$sub")
done
[ "${#SCAN_DIRS[@]}" -eq 0 ] && SCAN_DIRS=("$ROOT")

# Collect shell files: *.sh plus extensionless files with a bash/sh shebang.
mapfile -d '' -t FILES < <(
    find "${SCAN_DIRS[@]}" -type f \( -name '*.sh' -o ! -name '*.*' \) -print0
)

is_excluded() {
    local base="$1"
    for ex in "${EXCLUDE_BASENAMES[@]}"; do
        [ "$base" = "$ex" ] && return 0
    done
    return 1
}

# Enforce on bash files only. A pure-POSIX `#!/bin/sh` script is EXEMPT (see
# the Scope note above): no pipefail, and here-strings are undefined there.
should_enforce() {
    local f="$1" shebang
    shebang="$(head -1 "$f" 2>/dev/null)"
    case "$shebang" in
        *bash*) return 0 ;;          # explicit bash — enforce
        '#!'*sh|'#!'*sh\ *) return 1 ;;  # #!/bin/sh, #!/usr/bin/env sh … — POSIX, exempt
    esac
    # No recognizable shell shebang: a *.sh here is a bash-dialect file (often a
    # sourced library with `#!/bin/bash` or none) — enforce; anything else skip.
    case "$f" in
        *.sh) return 0 ;;
        *) return 1 ;;
    esac
}

VIOLATIONS=0
for f in "${FILES[@]}"; do
    is_excluded "$(basename "$f")" && continue
    should_enforce "$f" || continue
    # What enters the matcher is a LOGICAL line, not a physical one (mika#3130).
    # Drop full-line comments before matching; the line number is the first
    # physical line of the logical group. A correctly-cited escape suppresses it.
    while IFS= read -r hit; do
        [ -n "$hit" ] || continue
        lineno="${hit%%:*}"
        text="${hit#*:}"
        # Skip full-line comments (the pattern can legitimately appear in prose).
        case "$text" in
            \#*|[[:space:]]*\#*) continue ;;
        esac
        echo "ERROR: producer | grep -q under pipefail (SIGPIPE trap): $f:$lineno"
        echo "       $text"
        echo "       Fix: grep -q… -- PATTERN <<<\"\$DATA\"  (no pipeline, no SIGPIPE)."
        VIOLATIONS=$((VIOLATIONS + 1))
    done < <(
        # Chain order is load-bearing: stage 1 → exemption → stage 2 → match.
        # None of these links short-circuits, so the guard cannot fall into the
        # very SIGPIPE class it denies.
        # `-a` on both greps: a stream GNU grep decides is binary (an invalid
        # multibyte sequence anywhere in the file is enough) would otherwise
        # report `binary file matches` on stderr and SWALLOW the matching line —
        # a fail-open in the one place that must not have one.
        _logical_lines "$f" \
            | grep -a -Ev '# sigpipe-safe: #[0-9]+' \
            | _strip_eol_comment \
            | grep -a -E "$FORBIDDEN" \
            || true
    )
done

if [ "$VIOLATIONS" -gt 0 ]; then
    echo ""
    echo "::error::SIGPIPE-under-pipefail guard: $VIOLATIONS occurrence(s) of a data producer piped into 'grep -q'."
    echo "Rewrite as a here-string: 'grep -q… -- PATTERN <<<\"\$DATA\"'. See mika#2055 and"
    echo "docs/solutions/test-failures/bash-assert-sigpipe-and-host-coupling-before-ci-gate-2026-08-29.md."
    echo "Genuine exceptions: append '# sigpipe-safe: #<ticket>' (a bare marker without a '#<digits>' citation does NOT suppress)."
    exit 1
fi

echo "no-sigpipe-grep: clean (scanned ${#FILES[@]} file(s))."
