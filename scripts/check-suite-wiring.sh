#!/usr/bin/env bash
# CI lint (mika#2761): a test suite that lives in the tree and that no workflow
# can reach must SAY SO — in a registry, with a mode, a reason and a ticket.
#
# THE PROPERTY: three sets that must agree, in both directions.
#
#   D  the suites on disk. Files `*.sh`/`*.py` under `scripts/` and
#      `skills/bundled/_shared/` whose basename carries one of the repository's
#      five suite prefixes (`test-`, `test_`, `check-`, `verify-`, `measure-`).
#   C  the suites CI can reach. Proven on two syntactic hops, H1 and H2 below.
#   R  the suites the registry (`scripts/suite-wiring.tsv`) declares as
#      deliberately outside CI.
#
#   The rule is  D \ C == R  — stated as four directions:
#     D1  d ∈ D, d ∉ C, d ∉ R   a suite nothing runs that nobody declared
#     D2  r ∈ R, r ∈ C          a declaration that outlived its reason
#     D3  r ∈ R, r ∉ D          an entry naming a path that is not a suite
#     D4  r ∈ R malformed       a declaration that is not one
#
# WHY THIS EXISTS, and why the class is not the case.
#   Three times a suite was written, reviewed, merged — and kept nothing,
#   because no workflow called it.
#     mika#2578  `test-sandbox-gh-usable` and `test-github-token-not-in-sandbox`
#                were local-only; both were hand-wired into ci.yml afterwards.
#     mika#2752  `test-egress-push-bound` (143 assertions, mika#2736) and
#                `test-measure-webhook-turn-cost` (mika#2680 vol 2) each missed
#                their ci.yml entry, for two DIFFERENT reasons.
#   The doctrine has been written since mika#2578 — *a guard the pipeline never
#   runs refuses nothing* — and nothing enforced it. A suite present on disk and
#   absent from the workflows is invisible: it passes review, it passes CI (of
#   course it does), and you learn about it the day the defect it would have
#   caught reaches production. The class is named "layer 0" in
#   docs/solutions/best-practices/une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md
#   (mika#1801), whose census of 2026-09-28 named four gaps — all four were
#   still unreached twenty days later. A census nobody holds keeps nothing.
#
# WHY D1 IS NOT ENOUGH, i.e. why the registry is bidirectional.
#   A list of suites exempt from "must be reachable from CI" IS an allowlist,
#   and mika#2309 is the standing argument against one. What made that allowlist
#   dangerous was double: its entries carried no positive content ("skip me"),
#   and nothing could ever expire them. Both are closed here by construction —
#   D4 demands mode + reason + ticket, and D2 makes the entry go RED the moment
#   someone wires the suite. The very gesture that makes an exception pointless
#   is the gesture that makes it fail. An exception cannot outlive its reason.
#
# WHAT THIS LINT DOES NOT PROVE, stated plainly because a silent
# under-approximation is a false green waiting to happen.
#   C is computed on two SYNTACTIC hops. It does NOT prove reachability through
#   the shell body of another suite — `skills/bundled/_shared/test-dispatch-lib.sh`
#   invokes `scripts/check-pilot-turn-ceiling-labels.sh` from its own body, and
#   proving that in general would mean interpreting shell. Interpreting shell is
#   wrong in both directions, and the dangerous direction is the FALSE GREEN: a
#   suite that READS as wired and is not — which is the very defect. So C is
#   deliberately UNDER-approximated, and the only error this lint can make is a
#   false RED: noisy, never silent.
#   Consequence to own: a suite reached only by a shell runner reads as
#   unreached. Its author must choose — wire it directly, or declare it
#   `operator` and name the runner in the reason. No suite is in that state as
#   of 2026-10-18 (measured), so no `via-runner` mode is shipped: that would be
#   YAGNI over an empty set.
#   Second limit, equally deliberate: D is defined by a NAMING CONVENTION, not
#   by a proven property. A suite that carries none of the five prefixes escapes
#   D entirely (`scripts/deploy-info-test.sh` is today's one such file, and it
#   IS reachable). The population floor below catches a collapse of the
#   convention, never a single escapee.
#   Third limit: a workflow is a workflow. `release-pr.yml` is
#   `workflow_dispatch`-only (mika#2048) and `eval-calibration.yml` is manual;
#   a suite named only by one of those counts as reached here. No suite is in
#   that state today either.
#
# THE TWO HOPS.
#   H1  a `.github/workflows/*.yml` names the suite's repo-relative path
#       literally — `bash scripts/x.sh`, `python3 -B scripts/x.py` — including
#       inside a `run: |` block.
#   H2  a workflow names `make <t>`, and the recipe of `<t>` — plus its
#       prerequisites, transitively, plus any `make <u>` / `$(MAKE) <u>` it
#       recurses into — names the path literally.
#   Comments are stripped from both the workflows and the Makefile before any
#   path is looked for. A path mentioned in a comment is prose, and prose runs
#   nothing: counting it would be exactly the false green above.
#
# Usage: check-suite-wiring.sh [ROOT]
#   ROOT defaults to the repo root. An explicit ROOT lets the anti-vacuity
#   harness point the lint at a fixture tree (see
#   scripts/test-check-suite-wiring.sh) — it is never pointed at the real tree
#   by that harness.
#
# Exit codes:
#   0 — D \ C == R, every entry well formed
#   1 — one or more of the four directions found a violation
#   2 — a required input is unreadable
#   3 — the lint found nothing to check (population under its declared floor,
#       no workflow files, no parsable registry). A guard that inspects an empty
#       population must say so rather than report agreement.

set -euo pipefail

ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
ROOT="${ROOT%/}"

REGISTRY="$ROOT/scripts/suite-wiring.tsv"
WORKFLOW_DIR="$ROOT/.github/workflows"
MAKEFILE="$ROOT/Makefile"

# What the operator is told to go and edit. Repo-relative, because an absolute
# path to a fixture tree is noise in a message whose only useful content is
# "which file do I open" — and under a real checkout the two spellings name the
# same file anyway.
REGISTRY_REL="scripts/suite-wiring.tsv"
WORKFLOW_DIR_REL=".github/workflows"
MAKEFILE_REL="Makefile"

SUITE_ROOTS=("scripts" "skills/bundled/_shared")
SUITE_PREFIX_RE='^(test-|test_|check-|verify-|measure-)'
VALID_MODES=("operator" "unwired")

if [[ ! -f "$REGISTRY" ]]; then
    echo "ERROR: suite-wiring registry not readable: $REGISTRY" >&2
    exit 2
fi
if [[ ! -d "$WORKFLOW_DIR" ]]; then
    echo "ERROR: workflow directory not readable: $WORKFLOW_DIR" >&2
    exit 2
fi
if [[ ! -f "$MAKEFILE" ]]; then
    echo "ERROR: Makefile not readable: $MAKEFILE" >&2
    exit 2
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# ── Comment stripping, shared by both readers.
#
# A `#` is a comment in YAML and in shell when it starts a line or follows
# whitespace — the same rule in both, which is why one filter serves the
# workflows and the Makefile recipes alike. A `#` NOT preceded by whitespace is
# content (`https://host/x#frag`) and is left alone.
strip_comments() {
    sed -E 's/(^|[[:space:]])#.*$//' "$@"
}

# ── Side D: the suites on disk.
for root in "${SUITE_ROOTS[@]}"; do
    [[ -d "$ROOT/$root" ]] || continue
    find "$ROOT/$root" -type f \( -name '*.sh' -o -name '*.py' \) -print
done | while IFS= read -r abs; do
    base="${abs##*/}"
    [[ "$base" =~ $SUITE_PREFIX_RE ]] || continue
    printf '%s\n' "${abs#"$ROOT"/}"
done | LC_ALL=C sort -u > "$WORK/D"

D_COUNT="$(grep -c . "$WORK/D" || true)"

# ── The population floor (mika#2205, and layer 2 of the mika#1801 taxonomy:
# "empty population"). Declared in the registry header so it travels with the
# registry it bounds, with the date it was measured.
FLOOR=""
FLOOR="$(sed -nE 's/^#[[:space:]]*min-population:[[:space:]]*([0-9]+).*$/\1/p' "$REGISTRY" | head -n1)"
if [[ -z "$FLOOR" ]]; then
    echo "ERROR: $REGISTRY_REL declares no \`# min-population: <n>\` floor." >&2
    echo "Without it this lint cannot tell an empty population from a clean one." >&2
    exit 3
fi
if [[ "$D_COUNT" -lt "$FLOOR" ]]; then
    echo "ERROR: suite population collapsed: found $D_COUNT, declared floor is $FLOOR." >&2
    echo "Roots scanned: ${SUITE_ROOTS[*]} (under $ROOT)" >&2
    echo "Either the enumeration broke (renamed root, changed prefix convention)," >&2
    echo "or suites were genuinely removed — in which case lower the floor in" >&2
    echo "$REGISTRY_REL and say in the commit why. A lint that looks at nothing must" >&2
    echo "not report agreement." >&2
    exit 3
fi

# ── Side C, hop H1: a workflow names the path.
shopt -s nullglob
WORKFLOW_FILES=("$WORKFLOW_DIR"/*.yml "$WORKFLOW_DIR"/*.yaml)
shopt -u nullglob
if [[ ${#WORKFLOW_FILES[@]} -eq 0 ]]; then
    echo "ERROR: no workflow file found under $WORKFLOW_DIR." >&2
    echo "This lint has no notion of \"reachable from CI\" to compare against." >&2
    exit 3
fi
strip_comments "${WORKFLOW_FILES[@]}" > "$WORK/workflows.txt"

# ── Side C, hop H2: a workflow names `make <t>`, and <t>'s recipe names the path.
#
# Step 1 — model the Makefile: one `T` record per target (its prerequisites) and
# one `R` record per recipe line. Variable assignments (`FOO := bar`) and the
# dot-targets (`.PHONY`) are not targets.
strip_comments "$MAKEFILE" | awk -F '\t' '
    /^\t/ {
        if (cur != "") { line = $0; sub(/^\t/, "", line); print "R\t" cur "\t" line }
        next
    }
    /^[A-Za-z0-9_][A-Za-z0-9_.-]*[ \t]*:/ {
        head = $0
        sub(/:.*$/, "", head)
        rest = $0
        sub(/^[^:]*:/, "", rest)
        if (substr(rest, 1, 1) == "=") { cur = ""; next }   # FOO := bar
        n = split(head, names, /[ \t]+/)
        cur = ""
        for (i = 1; i <= n; i++) {
            if (names[i] == "") continue
            if (cur == "") cur = names[i]
            print "T\t" names[i] "\t" rest
        }
        next
    }
    # A blank line — including one a stripped comment left behind — does not end
    # a recipe in GNU make, so it must not end it here either. Resetting `cur`
    # on it would drop every recipe line after an interleaved comment.
    /^[[:space:]]*$/ { next }
    { cur = "" }
' > "$WORK/make-model"

# Step 2 — the seed set: every `make <t>` a workflow names. `make the …` in prose
# survives comment-stripping only if it was not in a comment; a seed that names
# no real target contributes nothing, so a stray word is inert here.
grep -hoE '(^|[^A-Za-z0-9_-])(make|\$\(MAKE\))[[:space:]]+(-[A-Za-z]+[[:space:]]+)*[A-Za-z0-9_][A-Za-z0-9_.-]*' \
    "$WORK/workflows.txt" 2>/dev/null \
    | awk '{ print $NF }' | LC_ALL=C sort -u > "$WORK/make-seeds" || true

# Step 3 — transitive closure over prerequisites and recursive make, collecting
# the recipe text of every reachable target.
: > "$WORK/make-recipes.txt"
: > "$WORK/make-seen"
QUEUE="$WORK/make-queue"
cp "$WORK/make-seeds" "$QUEUE"
while [[ -s "$QUEUE" ]]; do
    target="$(head -n1 "$QUEUE")"
    tail -n +2 "$QUEUE" > "$QUEUE.rest"
    mv "$QUEUE.rest" "$QUEUE"
    [[ -n "$target" ]] || continue
    if grep -qxF -- "$target" "$WORK/make-seen"; then continue; fi
    printf '%s\n' "$target" >> "$WORK/make-seen"

    # Its recipe lines, and the targets they recurse into.
    grep -F "$(printf 'R\t%s\t' "$target")" "$WORK/make-model" 2>/dev/null \
        | cut -f3- >> "$WORK/make-recipes.txt" || true

    # Its prerequisites.
    grep -F "$(printf 'T\t%s\t' "$target")" "$WORK/make-model" 2>/dev/null \
        | cut -f3- | tr ' \t' '\n\n' >> "$QUEUE" || true
done
# Recursive make inside the recipes of reachable targets: fold back in until
# nothing new appears. One extra pass per depth level; the repo is flat today
# (zero `$(MAKE)` sites), so this is insurance, not load.
while :; do
    grep -hoE '(^|[^A-Za-z0-9_-])(make|\$\(MAKE\))[[:space:]]+(-[A-Za-z]+[[:space:]]+)*[A-Za-z0-9_][A-Za-z0-9_.-]*' \
        "$WORK/make-recipes.txt" 2>/dev/null \
        | awk '{ print $NF }' | LC_ALL=C sort -u > "$WORK/make-rec" || true
    NEW="$(LC_ALL=C comm -23 "$WORK/make-rec" <(LC_ALL=C sort -u "$WORK/make-seen"))"
    [[ -n "$NEW" ]] || break
    while IFS= read -r t; do
        [[ -n "$t" ]] || continue
        printf '%s\n' "$t" >> "$WORK/make-seen"
        grep -F "$(printf 'R\t%s\t' "$t")" "$WORK/make-model" 2>/dev/null \
            | cut -f3- >> "$WORK/make-recipes.txt" || true
    done <<< "$NEW"
done

# ── Does $2 name the path $1 as a path, rather than as a fragment of a longer
# one? The repo has sibling pairs (`check-x.sh` / `test-check-x.sh`) that the
# directory separator already keeps apart, but a `.bak` or `.orig` sibling would
# not be — so both ends are bounded explicitly.
names_path() {
    local path="$1" blob="$2" esc
    esc="${path//./\\.}"
    grep -qE "(^|[^A-Za-z0-9._/-])${esc}([^A-Za-z0-9._/-]|$)" "$blob"
}

: > "$WORK/C"
while IFS= read -r suite; do
    [[ -n "$suite" ]] || continue
    if names_path "$suite" "$WORK/workflows.txt"; then
        printf '%s\n' "$suite" >> "$WORK/C"
    elif names_path "$suite" "$WORK/make-recipes.txt"; then
        printf '%s\n' "$suite" >> "$WORK/C"
    fi
done < "$WORK/D"
LC_ALL=C sort -u "$WORK/C" -o "$WORK/C"
C_COUNT="$(grep -c . "$WORK/C" || true)"

# ── Side R: the registry. Four tab-separated columns: path, mode, reason, ticket.
VIOLATIONS=0
: > "$WORK/R"
LINENO_IN=0
while IFS= read -r line || [[ -n "$line" ]]; do
    LINENO_IN=$((LINENO_IN + 1))
    [[ -z "${line//[[:space:]]/}" ]] && continue
    [[ "${line:0:1}" == "#" ]] && continue

    # NOT `IFS=$'\t' read -r a b c d`. TAB is an IFS *whitespace* character, so
    # `read` collapses runs of it: the row `path<TAB>unwired<TAB><TAB>mika#2761`
    # — an entry whose reason is empty, which is precisely the mika#2309 shape
    # this lint exists to name — would parse with the TICKET sitting in the
    # reason field, and be refused for a missing ticket instead. Fail-closed,
    # but with a message pointing at the wrong column: the operator fixes the
    # ticket, the lint still refuses, and that is how a guard gets disabled
    # rather than obeyed. `mapfile -d` splits on the delimiter without
    # collapsing, so an empty field stays an empty field.
    # Caught by `the empty-reason failure cites why emptiness is the defect`
    # in scripts/test-check-suite-wiring.sh § 9.
    FIELDS=()
    mapfile -td $'\t' FIELDS < <(printf '%s' "$line")

    if [[ ${#FIELDS[@]} -gt 4 ]]; then
        echo "ERROR [D4] $REGISTRY_REL:$LINENO_IN — more than four tab-separated columns."
        VIOLATIONS=$((VIOLATIONS + 1))
        continue
    fi
    r_path="${FIELDS[0]-}"
    r_mode="${FIELDS[1]-}"
    r_reason="${FIELDS[2]-}"
    r_ticket="${FIELDS[3]-}"
    if [[ -z "${r_path:-}" || -z "${r_mode:-}" ]]; then
        echo "ERROR [D4] $REGISTRY_REL:$LINENO_IN — expected: <path>\\t<mode>\\t<reason>\\t<ticket>"
        VIOLATIONS=$((VIOLATIONS + 1))
        continue
    fi

    printf '%s\n' "$r_path" >> "$WORK/R"

    mode_ok=0
    for m in "${VALID_MODES[@]}"; do [[ "$r_mode" == "$m" ]] && mode_ok=1; done
    if [[ $mode_ok -eq 0 ]]; then
        echo "ERROR [D4] $REGISTRY_REL:$LINENO_IN — mode \`$r_mode\` is outside the vocabulary (${VALID_MODES[*]}): $r_path"
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
    if [[ -z "${r_reason//[[:space:]]/}" ]]; then
        echo "ERROR [D4] $REGISTRY_REL:$LINENO_IN — empty reason: $r_path"
        echo "       An entry with no reason is the allowlist mika#2309 condemns: it says \"skip me\" and nothing else."
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
    if ! grep -qE 'mika#[0-9]+' <<< "${r_ticket:-}"; then
        echo "ERROR [D4] $REGISTRY_REL:$LINENO_IN — ticket column must cite at least one \`mika#<n>\`: $r_path"
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done < "$REGISTRY"

LC_ALL=C sort "$WORK/R" -o "$WORK/R"
DUPES="$(LC_ALL=C uniq -d "$WORK/R")"
if [[ -n "$DUPES" ]]; then
    while IFS= read -r d; do
        [[ -n "$d" ]] || continue
        echo "ERROR [D4] $REGISTRY_REL declares \`$d\` more than once."
        VIOLATIONS=$((VIOLATIONS + 1))
    done <<< "$DUPES"
fi
LC_ALL=C sort -u "$WORK/R" -o "$WORK/R"
R_COUNT="$(grep -c . "$WORK/R" || true)"

# ── The four directions, by set arithmetic.
LC_ALL=C comm -23 "$WORK/D" "$WORK/C" > "$WORK/orphans"      # D \ C

# D1 — on disk, unreached, undeclared.
while IFS= read -r s; do
    [[ -n "$s" ]] || continue
    if ! grep -qxF -- "$s" "$WORK/R"; then
        echo "ERROR [D1] $s is in the tree, no workflow reaches it, and the registry does not declare it."
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done < "$WORK/orphans"

# D2 — declared, but CI reaches it: the declaration outlived its reason.
while IFS= read -r s; do
    [[ -n "$s" ]] || continue
    if grep -qxF -- "$s" "$WORK/C"; then
        echo "ERROR [D2] $s is declared in the registry, but CI now reaches it — delete the entry."
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done < "$WORK/R"

# D3 — declared, but not a suite in D.
while IFS= read -r s; do
    [[ -n "$s" ]] || continue
    if ! grep -qxF -- "$s" "$WORK/D"; then
        if [[ -e "$ROOT/$s" ]]; then
            echo "ERROR [D3] $s is declared in the registry but is not in the suite population."
            echo "       The file exists; its name carries none of the suite prefixes (test-, test_, check-, verify-, measure-),"
            echo "       or it lives outside ${SUITE_ROOTS[*]}. Remove the entry, or rename the file into the convention."
        else
            echo "ERROR [D3] $s is declared in the registry but does not exist — delete the entry."
        fi
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done < "$WORK/R"

UNWIRED_COUNT="$(awk -F '\t' '$1 !~ /^#/ && $2 == "unwired" { n++ } END { print n + 0 }' "$REGISTRY")"

# ── The report. A lint that prints only "ok" does not let you see that it
# stopped looking (mika#2205).
echo "suite wiring: |D|=$D_COUNT (floor $FLOOR)  |C|=$C_COUNT  |R|=$R_COUNT  unwired=$UNWIRED_COUNT"

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS suite-wiring violation(s)."
    echo ""
    echo "Sources of truth, which must agree in the SAME commit:"
    echo "  disk      ${SUITE_ROOTS[*]}  (*.sh / *.py with a suite prefix)"
    echo "  reachable $WORKFLOW_DIR_REL + $MAKEFILE_REL  (H1: a workflow names the path; H2: via a \`make <t>\` a workflow names)"
    echo "  declared  $REGISTRY_REL"
    echo ""
    echo "Why this is blocking (mika#2761): a suite the pipeline never runs refuses"
    echo "nothing. It passes review, it passes CI, and the defect it would have caught"
    echo "lands in production instead. That happened in mika#2578 and twice in"
    echo "mika#2752; the doctrine existed and nothing enforced it."
    echo ""
    echo "Fix, in order of preference:"
    echo "  1. WIRE IT. Add a step to a .github/workflows/*.yml naming the path, or a"
    echo "     \`make <target>\` whose recipe names it. Three lines; it is the normal answer."
    echo "  2. DECLARE IT, if it legitimately cannot be a CI gate — it needs the outside"
    echo "     world (network, a token, a live service, a pid) or it is a deliberate"
    echo "     measuring instrument. Add one tab-separated line to $REGISTRY_REL:"
    echo "         <path>\\t<operator|unwired>\\treason\\tmika#<n>"
    echo "     \`operator\` = needs the outside world. \`unwired\` = runs offline, nothing"
    echo "     runs it: that is the backlog, and \`grep -c unwired\` is its size."
    echo "  3. DELETE IT. A suite nobody runs and nobody will wire keeps nothing."
    echo ""
    echo "A [D2] or [D3] line means the registry drifted, not the tree: the suite got"
    echo "wired (good — remove the entry) or the file is gone (remove the entry)."
    exit 1
fi

exit 0
