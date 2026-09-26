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
#   same pipeline segment (no `|`, no `#` comment start), then `grep` with a run
#   of flag tokens the LAST of which carries a `q` (`-q`, `-qF`, `-qiE`, `-Fq`…).
# The q-flag must appear before the (quoted) pattern, so a `-q`-shaped needle
# does not self-trigger.
FORBIDDEN='(^|[^[:alnum:]_.])(printf|echo)[^|#]*\|[[:space:]]*grep[[:space:]]+(-[A-Za-z]+[[:space:]]+)*-[A-Za-z]*q[A-Za-z]*([[:space:]]|$)'

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
    # Drop full-line comments before matching; keep line numbers via grep -n on
    # the original, then filter. A correctly-cited escape suppresses the line.
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
        grep -nE "$FORBIDDEN" "$f" \
            | grep -Ev '# sigpipe-safe: #[0-9]+' \
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
