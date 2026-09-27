#!/usr/bin/env bash
# CI lint: every issue-comment PATCH is co-located with its target guard (mika#2552).
#
# ─────────────────────────────────────────────────────────────────────────────
# THE RULE, IN ONE SENTENCE
#
# In `scripts/annotate-issue-token-comment.sh`, every `gh api … -X PATCH "$VAR"`
# must be preceded, IN THE SAME FUNCTION BODY, by a call to
# `issue_comment_patch_target_is_valid "$VAR"` on that same variable.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY A SCAN IN ADDITION TO THE BEHAVIOURAL SUITE
#
# `scripts/test-annotate-issue-token-comment.sh` pins that the guard refuses the
# frozen pre-mika#2552 target. It cannot see the guard being REMOVED, because
# removing it makes NO DECISION WRONG on the day it is written: the REST
# resolution still yields a correct `.url`, the target is still an issue-comment
# route, V1/V2 stay green, and only the net disappears — in silence. That is word
# for word the argument mika#2511 had to write for its own case, and it is why the
# two guards are complementary rather than redundant: one starts from the
# BEHAVIOUR, this one from the SHAPE.
#
# ─────────────────────────────────────────────────────────────────────────────
# CARDINALITY IS ASSERTED, AND THAT IS THE HALF NO FIXTURE CAN SEE
#
# A predicate grown too narrow — a renamed function, a reformatted invocation, a
# `gh` call assembled from a variable — would pass by LOOKING AT NOTHING, and a
# silently inert scan reads exactly like a clean tree (the mika#2205 class). The
# expected site count is therefore pinned at 2: the withdraw path and the rewrite
# path. A third legitimate PATCH site is a deliberate edit of this constant, made
# together with the test that covers it.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHAT IS DELIBERATELY *NOT* SCANNED
#
# A repo-wide rule ("no `gh api -X PATCH|POST|PUT` anywhere under
# `.github/workflows/`") was considered and REFUSED. Its population became 0 the
# moment mika#2552 moved the mutation into this script, so its silence would prove
# nothing — a detector vacant by construction, the mika#2205 class applied to the
# detector itself. It would also refuse a future legitimate `gh api -X POST` that
# nobody has measured as dangerous.
#
# ─────────────────────────────────────────────────────────────────────────────
# ALLOWLIST: shipped EMPTY, compared IN BOTH DIRECTIONS.
#
# WHEN THIS FIRES ON A NEW SITE, ROUTE THE SITE THROUGH THE GUARD. Do not add a
# line here (mika#2201 doctrine: a PATCH site you do not want to guard is a site
# to delete). An entry matching no violation fails the build exactly as an
# unguarded site does — the self-cleaning assertion, so that on the day the site
# is fixed the entry must go.
#
# Usage: check-issue-comment-patch-guard.sh [target-script]
#   The optional path lets the negative-control harness point the scan at a
#   mutated copy, on the mika#2103 / test-cwd-guard.sh V3 model.
#
# Exit 0 if clean, 1 with actionable errors otherwise.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-$REPO_ROOT/scripts/annotate-issue-token-comment.sh}"

GUARD_FN='issue_comment_patch_target_is_valid'
EXPECTED_PATCH_SITES=2

# Deliberately empty. See the doctrine above before writing a line here.
# Entry format: <basename>:<function>
ISSUE_COMMENT_PATCH_GUARD_ALLOWLIST=""

VIOLATIONS=0
fail() {
    echo "ERROR: $*"
    VIOLATIONS=$((VIOLATIONS + 1))
}

# ── Anti-vacuity, FIRST. A scan whose subject has moved verifies nothing and
# reads exactly like a clean run (mika#2103, mika#2205).
if [ ! -r "$TARGET" ]; then
    echo "ERROR: target script not readable: $TARGET"
    echo "       A scan with nothing to scan is a vacuous pass, not a clean one."
    echo "       Check for a rename before touching this rule."
    exit 1
fi

TARGET_NAME="$(basename "$TARGET")"

# ── Extract the records.
#
# The unit of analysis is the LOGICAL invocation: continuations are re-joined
# before examination, because `gh api --silent -X PATCH "$prior" \` wraps its
# `-f body=` onto the next physical line and a line-anchored scan would read a
# different string than the shell does. Whole-line comments are stripped BEFORE
# the join (the mika#2496 motif) — this file's own header quotes the pre-fix
# invocation and names the guard, so a scan over the raw text would accuse the
# documentation that explains it.
RECORDS="$(
    awk -v guardfn="$GUARD_FN" '
        {
            raw = $0
            if (raw ~ /^[[:space:]]*#/) raw = ""
            if (cont) { buf = buf " " raw } else { buf = raw; start = FNR }
            if (buf ~ /\\[[:space:]]*$/) {
                sub(/\\[[:space:]]*$/, "", buf)
                cont = 1
                next
            }
            cont = 0
            n++; LL[n] = buf; NN[n] = start
        }
        END {
            if (cont) { n++; LL[n] = buf; NN[n] = start }
            fname = "(top level)"
            for (i = 1; i <= n; i++) {
                L = LL[i]; N = NN[i]

                if (L ~ /^[A-Za-z_][A-Za-z0-9_]*\(\)[[:space:]]*\{/) {
                    fn = L
                    sub(/\(\).*$/, "", fn)
                    fname = fn
                    continue
                }
                if (L ~ /^\}/) { fname = "(top level)"; continue }

                gi = index(L, guardfn)
                if (gi > 0) {
                    v = substr(L, gi + length(guardfn))
                    if (v ~ /^[[:space:]]+"\$/) {
                        sub(/^[[:space:]]+"\$\{?/, "", v)
                        sub(/[}"].*$/, "", v)
                        printf "GUARD\t%s\t%s\t%s\n", fname, v, N
                        continue
                    }
                }

                if (L ~ /gh[[:space:]]+api/ && L ~ /-X[[:space:]]+PATCH/) {
                    v = L
                    sub(/^.*-X[[:space:]]+PATCH[[:space:]]+/, "", v)
                    if (v ~ /^"\$/) {
                        sub(/^"\$\{?/, "", v)
                        sub(/[}"].*$/, "", v)
                        printf "PATCH\t%s\t%s\t%s\n", fname, v, N
                    } else {
                        printf "PATCHLIT\t%s\t%s\t%s\n", fname, "unresolvable-target", N
                    }
                    continue
                }
            }
        }
    ' "$TARGET"
)"

# ── Index the guard calls by function + variable.
declare -A GUARD_AT=()
while IFS=$'\t' read -r kind fn var line; do
    [ "$kind" = "GUARD" ] || continue
    GUARD_AT["$fn|$var"]="$line"
done <<< "$RECORDS"

# ── Allowlist, parsed once.
ALLOW_ENTRIES=()
while IFS= read -r raw; do
    entry="${raw%%#*}"
    entry="$(echo "$entry" | tr -d '[:space:]')"
    [ -n "$entry" ] && ALLOW_ENTRIES+=("$entry")
done <<< "$ISSUE_COMMENT_PATCH_GUARD_ALLOWLIST"

MATCHED_ALLOW=()
PATCH_SITES=0

while IFS=$'\t' read -r kind fn var line; do
    case "$kind" in
        PATCH|PATCHLIT) ;;
        *) continue ;;
    esac
    PATCH_SITES=$((PATCH_SITES + 1))

    site="$TARGET_NAME:$fn"
    allowed=0
    for entry in "${ALLOW_ENTRIES[@]:-}"; do
        if [ "$entry" = "$site" ]; then
            allowed=1
            MATCHED_ALLOW+=("$entry")
        fi
    done
    [ "$allowed" -eq 1 ] && continue

    if [ "$kind" = "PATCHLIT" ]; then
        fail "a PATCH target is not a plain \"\$var\" — $TARGET_NAME:$line (in $fn)"
        echo "       The scan cannot prove a computed or literal target is guarded, so it"
        echo "       refuses it. Bind the target to a variable and guard that variable."
        continue
    fi

    guard_line="${GUARD_AT["$fn|$var"]:-}"
    if [ -z "$guard_line" ]; then
        fail "a PATCH is not guarded — $TARGET_NAME:$line patches \"\$$var\" in $fn()"
        echo "       Call $GUARD_FN \"\$$var\" in the SAME function, before the PATCH."
        echo "       An unguarded PATCH can reach the ISSUE-edit route and replace the"
        echo "       body, taking the grooming callouts the loop reads with it (mika#2552)."
        echo "       Do NOT add this site to ISSUE_COMMENT_PATCH_GUARD_ALLOWLIST: a PATCH"
        echo "       site you do not want to guard is a site to delete."
    elif [ "$guard_line" -gt "$line" ]; then
        fail "the guard runs AFTER the PATCH — $TARGET_NAME:$line patches \"\$$var\", guarded at line $guard_line"
        echo "       A check that runs after the write is not a check."
    fi
done <<< "$RECORDS"

# ── Cardinality. The one failure shape no fixture can see.
if [ "$PATCH_SITES" -ne "$EXPECTED_PATCH_SITES" ]; then
    fail "expected $EXPECTED_PATCH_SITES PATCH site(s) in $TARGET_NAME, found $PATCH_SITES."
    echo "       A predicate grown too narrow passes by looking at nothing, and a"
    echo "       silently inert scan reads exactly like a clean tree (mika#2205)."
    echo "       If a PATCH site was legitimately added or removed, change"
    echo "       EXPECTED_PATCH_SITES together with the test that covers it."
fi

# ── Allowlist, the other direction.
for entry in "${ALLOW_ENTRIES[@]:-}"; do
    [ -n "$entry" ] || continue
    still=0
    for m in "${MATCHED_ALLOW[@]:-}"; do
        [ "$m" = "$entry" ] && still=1
    done
    if [ "$still" -eq 0 ]; then
        fail "allowlist entry '$entry' matches no PATCH site — remove it."
        echo "       The allowlist is compared in both directions on purpose. An entry"
        echo "       that outlives its site is how the list stops being a measurement"
        echo "       of the residue and becomes a junk drawer."
    fi
done

if [ "$VIOLATIONS" -gt 0 ]; then
    echo ""
    echo "Found $VIOLATIONS issue-comment PATCH guard violation(s)."
    echo "Reasoning: docs/plans/2026-09-27-001-fix-2552-le-patch-dannotation-vise-le-commentaire-plan.md"
    exit 1
fi

echo "Every issue-comment PATCH is guarded ($PATCH_SITES PATCH sites scanned, ${#ALLOW_ENTRIES[@]} allowlisted)."
exit 0
