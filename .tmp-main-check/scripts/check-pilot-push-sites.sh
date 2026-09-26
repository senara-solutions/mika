#!/usr/bin/env bash
# CI lint: a skill handler does not push (mika#2520).
#
# ─────────────────────────────────────────────────────────────────────────────
# THE RULE, IN ONE SENTENCE
#
# No git push invocation may appear in `skills/bundled/*/handlers/*.sh`.
#
# A handler that needs to publish calls the guarded shared helper,
# `skills/bundled/_shared/pr-push-guard.sh`, which resolves its target from the
# PR head, refuses five ways, and pushes once with an explicit destination
# refspec and a pre-session lease.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY THE PREDICATE IS LEXICAL AND POSITIONAL, NOT SEMANTIC
#
# The founding defect was a line of PROMPT PROSE inside a handler:
#
#     6. Push: git push --force-with-lease
#
# It is not in shell command position — it sits inside a `PROMPT="..."` string,
# after `6. Push: `. So a rule keyed on command position (the mika#2496 model)
# would miss the exact defect this guard exists to catch. The predicate is
# therefore the presence of the invocation anywhere in the file, and the file set
# is what does the discriminating:
#
#   * `_shared/` is OUT of the population — it carries the one legitimate,
#     guarded push site.
#   * `system_prompt.md` is OUT of the population — dev-groom's prompt carries
#     PROHIBITIONS spelled out in full, and a semantic scan would have to tell a
#     prohibition from a prescription. This one does not even have to ask.
#
# DO NOT ADD A SPECIAL CASE FOR A "NEARLY RIGHT" FORM. A handler writing
# `git push --force-with-lease=<b>:<sha> origin HEAD:refs/heads/<b>` inline is
# still a second push site: the refusals live in the helper, not in the argv.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHAT THE MATCH COVERS
#
# `git` followed by any run of options (and their values) and then the verb
# `push` — so `git push`, `git -C "$WT" push` and `git --no-pager push` all
# match, while `git commit` next to an unrelated word `push` does not. Comment
# lines are stripped BEFORE the examination (the mika#2496 motif), which is what
# lets a handler explain in its header why it no longer pushes.
#
# Known limit, stated rather than discovered: this is a line-oriented lexical
# scan. An invocation split across a line continuation, or assembled from a
# variable, escapes it. The structural half is that the push now lives in the
# shared helper and the handlers have no reason to build one.
#
# ALLOWLIST: `scripts/pilot-push-allowlist.txt`, compared IN BOTH DIRECTIONS —
# an entry matching no violation fails the build exactly as an unlisted site
# does. That is the self-cleaning assertion: on the day the site is fixed, the
# build goes red and the entry must go.
#
# WHEN THIS FIRES ON A NEW SITE, ROUTE THE SITE TO THE GUARDED HELPER. Do not add
# a line to the allowlist (mika#2201 doctrine: a push site you do not want to
# guard is a site to route, never to exempt).
#
# Usage: check-pilot-push-sites.sh [scan-root]
#   The optional scan root lets the anti-vacuity harness
#   (scripts/test-check-pilot-push-sites.sh) point the guard at a fixture tree,
#   on the mika#2103 model.
#
# Exit 0 if clean, 1 with actionable errors otherwise.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCAN_ROOT="${1:-$REPO_ROOT}"

HANDLERS_GLOB_DIR="$SCAN_ROOT/skills/bundled"
ALLOWLIST_FILE="$SCAN_ROOT/scripts/pilot-push-allowlist.txt"

# `git` + any run of options (and their values) + the verb `push`.
#
# The right-hand boundary is "anything that is not a word character", NOT
# whitespace. The first draft of this expression required whitespace and
# therefore MISSED the founding defect itself — `PROMPT="5. Push with: git push"`
# ends the invocation on a double quote, so the scan read that handler as clean
# while catching its `--force-with-lease` sibling. The negative-control harness
# is what surfaced that, which is the whole reason it exists.
PUSH_RE='(^|[^[:alnum:]_./-])git([[:space:]]+-[^[:space:]]+([[:space:]]+[^[:space:]-][^[:space:]]*)?)*[[:space:]]+push([^[:alnum:]_-]|$)'

VIOLATIONS=0

fail() {
    echo "ERROR: $*"
    VIOLATIONS=$((VIOLATIONS + 1))
}

# ── Collect the population. ──────────────────────────────────────────────────
#
# A run where the population is EMPTY is refused rather than reported clean: a
# scan with nothing to scan reads exactly like a clean tree (the mika#2205
# class), and the population can empty itself silently through a directory
# rename or an extension change.
collect_handlers() {
    [ -d "$HANDLERS_GLOB_DIR" ] || return 0
    find "$HANDLERS_GLOB_DIR" -mindepth 3 -maxdepth 3 \
        -path "*/handlers/*.sh" -type f \
        | sed "s|^$SCAN_ROOT/||" \
        | sort
}

FILES="$(collect_handlers)"

if [ -z "$FILES" ]; then
    echo "ERROR: no handler scripts found under '$HANDLERS_GLOB_DIR'."
    echo "       A scan with nothing to scan is a vacuous pass, not a clean one."
    echo "       The population is skills/bundled/*/handlers/*.sh — check for a"
    echo "       directory rename or an extension change before touching this rule."
    exit 1
fi

# ── Find the violations. ─────────────────────────────────────────────────────
RAW_VIOLATIONS=()

while IFS= read -r rel; do
    [ -n "$rel" ] || continue
    while IFS= read -r entry; do
        [ -n "$entry" ] || continue
        RAW_VIOLATIONS+=("$rel:$entry")
    done < <(
        # Strip whole-line comments first, keeping line numbers intact.
        awk '{ if ($0 ~ /^[[:space:]]*#/) print ""; else print }' "$SCAN_ROOT/$rel" \
            | grep -nE "$PUSH_RE" || true
    )
done <<< "$FILES"

# ── Allowlist, compared in both directions. ──────────────────────────────────
ALLOW_ENTRIES=()
if [ -f "$ALLOWLIST_FILE" ]; then
    while IFS= read -r raw; do
        line="${raw%%#*}"
        line="$(echo "$line" | tr -d '[:space:]')"
        [ -n "$line" ] && ALLOW_ENTRIES+=("$line")
    done < "$ALLOWLIST_FILE"
fi

MATCHED_ALLOW=()

for viol in "${RAW_VIOLATIONS[@]:-}"; do
    [ -n "$viol" ] || continue
    vpath="${viol%%:*}"
    vrest="${viol#*:}"
    vline="${vrest%%:*}"
    vtext="${vrest#*:}"

    allowed=0
    for entry in "${ALLOW_ENTRIES[@]:-}"; do
        if [ "$entry" = "$vpath" ]; then
            allowed=1
            MATCHED_ALLOW+=("$entry")
        fi
    done

    if [ "$allowed" -eq 0 ]; then
        fail "a skill handler invokes git push — $vpath:$vline"
        echo "       $(echo "$vtext" | sed 's/^[[:space:]]*//')"
        echo "       Handlers do not push (mika#2520). Source"
        echo "       skills/bundled/_shared/pr-push-guard.sh and call"
        echo "       pr_push_guard_push after the session, so the target comes from"
        echo "       the PR head, five refusals apply, and the lease is pinned to a"
        echo "       SHA captured before the pilot ran."
        echo "       Do NOT add this path to scripts/pilot-push-allowlist.txt: a push"
        echo "       site you do not want to guard is a site to route."
    fi
done

for entry in "${ALLOW_ENTRIES[@]:-}"; do
    [ -n "$entry" ] || continue
    still_matches=0
    for m in "${MATCHED_ALLOW[@]:-}"; do
        [ "$m" = "$entry" ] && still_matches=1
    done
    if [ "$still_matches" -eq 0 ]; then
        fail "allowlist entry '$entry' matches no push site — remove it."
        echo "       The allowlist is compared in both directions on purpose. An entry"
        echo "       that outlives its site is how the list stops being a measurement"
        echo "       of the residue and becomes a junk drawer."
    fi
done

if [ "$VIOLATIONS" -gt 0 ]; then
    echo ""
    echo "Found $VIOLATIONS unguarded push site(s) in skill handlers."
    echo "Reasoning: docs/plans/2026-09-25-001-fix-2520-le-push-du-skill-porte-une-refspec-explicite-plan.md"
    exit 1
fi

echo "No unguarded push sites in skill handlers ($(echo "$FILES" | wc -l | tr -d ' ') scanned, ${#ALLOW_ENTRIES[@]} allowlisted)."
exit 0
