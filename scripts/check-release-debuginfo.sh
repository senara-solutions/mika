#!/usr/bin/env bash
# CI lint: the release profile must keep the line tables that make a wedged
# binary readable (mika#1719).
#
# THE PROPERTY: two clauses of `[profile.release]`, and NEITHER works alone.
#   `strip = "none"`  keeps whatever debug info was emitted.
#   `debug = "line-tables-only"`  is what emits it in the first place.
#
#   A release profile defaults to `debug = 0`, so `strip = "none"` alone leaves
#   nothing to keep and `addr2line` answers `??:0`. And `debug` alone emits
#   tables that `strip` then removes. Each clause, read on its own, LOOKS like it
#   carries the setting — which is precisely why a future editor who deletes one
#   will have good reason to believe the other still holds the guarantee.
#
# WHY A GUARD AND NOT A NOTE.
#   `strip = true` was ONE LINE, and its return would be silent. Someone
#   trimming Docker image size puts it back, no test goes red, the binary goes
#   mute — and we find out at the next process-wide wedge, i.e. at the most
#   expensive possible moment. That is the mika#2205 class: *a setting silently
#   cancelled reads exactly like a setting in force.* mika#1719 is the measured
#   cost of a mute binary: five gdb captures answering `?? ()` on every frame, a
#   PIE base recomputed by hand, three falsified hypotheses, and a one-line bug
#   that took four days and five outages to name.
#
# THE PREDICATE IS POSITIVE AND BOUNDED, and both adjectives are load-bearing.
#
#   BOUNDED TO THE BLOCK. Only `[profile.release]` is read, from its header to
#   the next line whose first non-blank character is `[`. Without that bound a
#   legitimate `strip = true` in a `[profile.bench]` further down would redden a
#   clean tree — and a guard that cries wrongly ends up muzzled.
#
#   POSITIVE, NOT A DENYLIST. Term A accepts `strip` ABSENT or `none`, and
#   refuses everything else; it is not a test for `!= true`, because
#   `strip = "symbols"` is equivalent to `true` and would sail past a denylist.
#   Term B requires `debug` PRESENT with a line-table-preserving value, because
#   "not false" also admits absence, which is the default that yields `??:0`.
#
# WHY `split-debuginfo` IS NOT A THIRD TERM. It is a SIZE setting, and measurement
#   shows it does not cost the capability: `packed` deports `.debug_info` and
#   `.debug_str` (types, variables) while KEEPING `.symtab` and `.debug_line` — the
#   name half and the file:line half — in the binary. Removing it makes the binary
#   bigger, never muter. This guard protects the capability; legislating a size
#   knob would make it fragile for no measured gain.
#
# EXIT CODES. 0 clean; 1 on a violation (or on a stale allowlist entry); 2 when
#   a file cannot be read; 3 when the block cannot be found or cannot be audited.
#   A guard that finds nothing to check must SAY SO rather than pass — *a blind
#   scan reads exactly like a clean tree.* (Exit 2 for an unreadable file rather
#   than the plan's 3 follows the fifteen sibling `check-*.sh` of this repo; the
#   property the plan defends — never exit 0 — is what matters and is held.)
#
# WHEN THIS FIRES, REPAIR THE PROFILE. Do not add a line to the allowlist
#   (mika#2201 doctrine, "on déclare, on n'allowliste pas"). A profile you do not
#   want to make diagnosable is a profile to discuss, not one to exempt. The
#   allowlist ships EMPTY and its population is empty by construction: the commit
#   that arms this guard is the commit that sets the conforming value.
#
# Usage: check-release-debuginfo.sh [Cargo.toml] [allowlist.txt]
#   Both default to the repo's real files. Explicit arguments let the
#   anti-vacuity harness point the guard at fixtures (see
#   scripts/test-check-release-debuginfo.sh).
#
# ── STILL TO ARM IN CI, and this is the only item of mika#1719 left open.
#
# `make check-release-debuginfo` runs this guard and its harness, but NO CI job
# calls it yet: the dispatch credential that opened mika#1719's PR lacks the
# `workflow` OAuth scope, so GitHub refused the push that carried the job
# ("refusing to allow a Personal Access Token to create or update workflow
# .github/workflows/ci.yml without `workflow` scope"). A guard the pipeline never
# runs refuses nothing (mika#2103), so the job is recorded HERE — in a versioned
# file — rather than in a PR body nobody re-reads. Add it to
# `.github/workflows/ci.yml`, verbatim, beside the other `*-lint` jobs:
#
#   release-debuginfo-lint:
#     name: Release Debuginfo Lint
#     runs-on: ubuntu-22.04
#     steps:
#       - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1  # v6
#       # `strip = true` in [profile.release] was ONE LINE, and its return would
#       # be silent: no test reddens, the binary goes mute, and we find out at the
#       # next process-wide wedge — the most expensive possible moment.
#       - name: Reject a [profile.release] that strips its line tables (mika#1719)
#         run: bash scripts/check-release-debuginfo.sh
#       # A guard nobody has watched go red is a decoration (mika#2103).
#       - name: Pin the guard's negative behaviour
#         run: bash scripts/test-check-release-debuginfo.sh
#
# Until that job exists, the setting is held by `make` and by review alone —
# which is exactly the silent-regression window this guard was written to close.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CARGO_TOML="${1:-$REPO_ROOT/Cargo.toml}"
ALLOWLIST="${2:-$REPO_ROOT/scripts/release-debuginfo-allowlist.txt}"

if [[ ! -f "$CARGO_TOML" ]]; then
    echo "ERROR: manifest not readable: $CARGO_TOML" >&2
    exit 2
fi

MANIFEST_NAME="$(basename "$CARGO_TOML")"

# ── Extract the `[profile.release]` block, and refuse what cannot be audited.
#
# The header is matched on a line whose first non-blank character starts the
# table name, so a commented `# [profile.release]` never matches. The block ends
# at the next table header — including `[profile.release.package.foo]`, whose
# per-package overrides are deliberately outside the predicate: what this guard
# protects is the profile that builds mika-spirit.
#
# awk exits 3 when no block is found, 4 when the table is declared twice (TOML
# would reject that, but a scan reading two values for one key is a scan that
# cannot answer), 5 when a key this guard owns is assigned twice inside the
# block. All three are reported as exit 3: a guard that cannot see everything
# must not pass.
extract_release_block() {
    awk '
        # A table header: first non-blank char is `[`. Comments cannot reach here
        # because a TOML comment line starts with `#`.
        {
            line = $0
            sub(/^[ \t]+/, "", line)
        }
        line ~ /^#/ { next }
        line ~ /^\[/ {
            if (line ~ /^\[profile\.release\][ \t]*(#.*)?$/) {
                if (found) { bad_dup_table = 1; printf "%d: %s\n", NR, $0 > "/dev/stderr" }
                collecting = 1; found = 1; next
            }
            collecting = 0
            next
        }
        collecting {
            if (line == "") { next }
            # Count assignments of the two keys this guard owns.
            if (line ~ /^strip[ \t]*=/) { strip_n++ }
            if (line ~ /^debug[ \t]*=/) { debug_n++ }
            print line
        }
        END {
            if (!found) { exit 3 }
            if (bad_dup_table) { exit 4 }
            if (strip_n > 1 || debug_n > 1) { exit 5 }
        }
    ' "$1"
}

BLOCK=""
awk_status=0
BLOCK="$(extract_release_block "$CARGO_TOML")" || awk_status=$?

case "$awk_status" in
    0) ;;
    3)
        echo "ERROR: no \`[profile.release]\` table found in $CARGO_TOML" >&2
        echo "This guard has nothing to check, which is NOT the same as agreement:" >&2
        echo "a renamed or relocated profile would make it silently inert, and an" >&2
        echo "inert scan reads exactly like a clean tree (mika#2205)." >&2
        echo "" >&2
        echo "Fix: restore the \`[profile.release]\` table carrying \`strip = \"none\"\`" >&2
        echo "and \`debug = \"line-tables-only\"\`, or point this guard at the table that" >&2
        echo "replaced it." >&2
        exit 3
        ;;
    4)
        echo "ERROR: \`[profile.release]\` is declared more than once in $CARGO_TOML (line(s) above)." >&2
        echo "TOML rejects a duplicate table, and a scan reading two blocks for one" >&2
        echo "profile cannot say which one builds the binary. Keep a single table." >&2
        exit 3
        ;;
    5)
        echo "ERROR: \`strip\` or \`debug\` is assigned twice inside \`[profile.release]\` in $CARGO_TOML." >&2
        echo "TOML rejects a duplicate key, and this guard would have to guess which" >&2
        echo "assignment wins. Keep one assignment per key." >&2
        exit 3
        ;;
    *)
        echo "ERROR: could not parse $CARGO_TOML (awk exit $awk_status)" >&2
        exit 2
        ;;
esac

if [[ -z "$BLOCK" ]]; then
    echo "ERROR: \`[profile.release]\` in $CARGO_TOML is empty." >&2
    echo "An empty profile carries neither \`strip\` nor \`debug\`, so the binary is" >&2
    echo "mute — and a guard that passed on it would be reporting the absence of a" >&2
    echo "measurement as the presence of the setting (mika#2205)." >&2
    echo "" >&2
    echo "Fix: add \`strip = \"none\"\` and \`debug = \"line-tables-only\"\`." >&2
    exit 3
fi

# ── Read one key's raw value out of the block.
#
# Returns the empty string when the key is absent; the two terms below
# distinguish absent from present-but-wrong themselves, because for `strip`
# absence is CONFORMING (the default is no stripping) and for `debug` absence is
# the defect (the default is 0).
#
# A trailing comment is stripped. When the value is quoted, the closing quote
# ends it, so a `#` inside it would be content — neither key takes such a value
# today, and reading it this way costs nothing.
read_key() {
    local key="$1"
    awk -v key="$key" '
        $0 ~ "^" key "[ \t]*=" {
            v = $0
            sub("^" key "[ \t]*=[ \t]*", "", v)
            if (v ~ /^"/)       { sub(/^"/, "", v); sub(/".*$/, "", v); print "\"" v "\"" }
            else if (v ~ /^'"'"'/) { sub(/^'"'"'/, "", v); sub(/'"'"'.*$/, "", v); print "\"" v "\"" }
            else                { sub(/[ \t]*#.*$/, "", v); sub(/[ \t]+$/, "", v); print v }
            exit
        }
    ' <<< "$BLOCK"
}

STRIP_RAW="$(read_key strip)"
DEBUG_RAW="$(read_key debug)"

# Unquote for comparison; keep the raw form for messages.
unquote() { sed -e 's/^"//' -e 's/"$//' <<< "$1"; }
STRIP_VAL="$(unquote "$STRIP_RAW")"
DEBUG_VAL="$(unquote "$DEBUG_RAW")"

VIOLATED_KEYS=()

# ── Term A — `strip`: absent, `none`, or `false`.
#
# POSITIVE. `strip = "symbols"` and `strip = "debuginfo"` both remove what
# term B emits (`debuginfo` leaves `.symtab`: names, but no `file:line`), and
# both would pass a `!= true` test. So the accept set is named instead.
#
# `false` is in the accept set although mika#1719's plan named only `none`:
# cargo reads `strip = false` as "do not strip", so it satisfies the intent
# exactly. Refusing a value that holds the guarantee would be a false positive,
# and a guard that cries wrongly ends up muzzled — the same reasoning that bounds
# the predicate to the block.
STRIP_OK=0
if [[ -z "$STRIP_RAW" ]]; then
    STRIP_OK=1  # absent: cargo strips nothing by default.
elif [[ "$STRIP_VAL" == "none" || "$STRIP_VAL" == "false" ]]; then
    STRIP_OK=1
fi

# ── Term B — `debug`: present, with a line-table-preserving value.
#
# This is the clause a future editor would think redundant. `[profile.release]`
# defaults to `debug = 0`, so its absence is the defect and not a neutral state.
DEBUG_OK=0
case "$DEBUG_VAL" in
    line-tables-only|limited|full|true|1|2) DEBUG_OK=1 ;;
esac

# ── The allowlist: exemptions, compared IN BOTH DIRECTIONS.
#
# Format, one per line: `<manifest-basename>:<key>  # why`
# An entry exempts a violation of that key. An entry that matches NO real
# violation fails the build exactly as an unlisted violation does — that is the
# self-cleaning assertion: on the day the profile is repaired, the build goes red
# and the stale entry must go, rather than surviving its motive and silently
# exempting a future namesake.
#
# An ABSENT allowlist file means no exemptions, never an error: it is a
# convenience for a population that is empty by construction, so a tree without
# it is a tree with nothing exempted — the state this guard wants.
ALLOWED=()
if [[ -f "$ALLOWLIST" ]]; then
    while IFS= read -r raw; do
        entry="${raw%%#*}"
        entry="$(tr -d '[:space:]' <<< "$entry")"
        [[ -z "$entry" ]] && continue
        ALLOWED+=("$entry")
    done < "$ALLOWLIST"
fi

is_allowed() {
    local needle="$1" e
    for e in "${ALLOWED[@]+"${ALLOWED[@]}"}"; do
        [[ "$e" == "$needle" ]] && return 0
    done
    return 1
}

VIOLATIONS=0
EXEMPTED=()

if [[ $STRIP_OK -eq 0 ]]; then
    if is_allowed "$MANIFEST_NAME:strip"; then
        EXEMPTED+=("$MANIFEST_NAME:strip")
    else
        echo "ERROR: [profile.release] \`strip = $STRIP_RAW\` removes the debug info that makes a wedged binary readable."
        echo "       Fix: \`strip = \"none\"\` (with \`debug = \"line-tables-only\"\` — neither works alone)."
        echo "       Why: mika#1719 — a stripped mika-spirit turned a one-line deadlock into"
        echo "       four days of gdb archaeology across five outages."
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
    VIOLATED_KEYS+=("$MANIFEST_NAME:strip")
fi

if [[ $DEBUG_OK -eq 0 ]]; then
    if is_allowed "$MANIFEST_NAME:debug"; then
        EXEMPTED+=("$MANIFEST_NAME:debug")
    else
        if [[ -z "$DEBUG_RAW" ]]; then
            echo "ERROR: [profile.release] has no \`debug\` key, so the profile defaults to \`debug = 0\` and emits no line tables."
            echo "       \`strip = \"none\"\` alone keeps nothing: \`addr2line\` answers \`??:0\`."
        else
            echo "ERROR: [profile.release] \`debug = $DEBUG_RAW\` emits no line tables."
            echo "       Without them an offset resolves to no \`file:line\`, which is the half"
            echo "       of the mika#1719 chain that stayed open the longest."
        fi
        echo "       Fix: \`debug = \"line-tables-only\"\` (with \`strip = \"none\"\` — neither works alone)."
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
    VIOLATED_KEYS+=("$MANIFEST_NAME:debug")
fi

# ── Second direction: an entry matching no real violation.
STALE=0
for e in "${ALLOWED[@]+"${ALLOWED[@]}"}"; do
    hit=0
    for v in "${VIOLATED_KEYS[@]+"${VIOLATED_KEYS[@]}"}"; do
        [[ "$e" == "$v" ]] && hit=1
    done
    if [[ $hit -eq 0 ]]; then
        echo "ERROR: stale allowlist entry \`$e\` in $(basename "$ALLOWLIST") — it exempts no real violation."
        echo "       The profile it named is conforming now. Delete the entry: an exemption"
        echo "       that outlives its motive silently exempts a future namesake."
        STALE=$((STALE + 1))
    fi
done

if [[ $((VIOLATIONS + STALE)) -gt 0 ]]; then
    echo ""
    echo "release-debuginfo: $VIOLATIONS violation(s), $STALE stale allowlist entry(ies)."
    echo ""
    echo "The two clauses are one setting (mika#1719):"
    echo "    [profile.release]"
    echo "    strip = \"none\"              # keep what debug emits"
    echo "    debug = \"line-tables-only\"  # emit it in the first place"
    echo ""
    echo "When this guard fires, repair the profile. Do NOT add an allowlist line:"
    echo "a profile you do not want to make diagnosable is a profile to discuss"
    echo "(mika#2201)."
    exit 1
fi

if [[ ${#EXEMPTED[@]} -gt 0 ]]; then
    echo "release-debuginfo: conforming, with ${#EXEMPTED[@]} allowlisted exemption(s): ${EXEMPTED[*]}"
else
    echo "release-debuginfo: [profile.release] keeps line tables (strip=${STRIP_RAW:-<absent, defaults to no strip>}, debug=$DEBUG_RAW)."
fi
exit 0
