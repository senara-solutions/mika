#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-root-files.sh (mika#2760).
#
# "Delete the thing the test protects; confirm the test goes red."
#
# The guard compares two lists that must be equal: what git tracks at the repo
# root, and what scripts/root-files-declared.txt declares. The acceptance
# criterion of mika#2760 is explicit — *a new `.md` at the root must redden the
# lint* — and case 2 below is that control, built from the REAL declared list so
# that a declaration file emptied by accident reddens the case rather than
# letting it pass.
#
# An equality guard passes for the wrong reason in three ways, all exit 3 here:
# a side A that read nothing (no tracked root file, or no git repository at all)
# and a side B with no parseable entry (empty file, or comments only). Each is
# watched below.
#
# Fixtures are shaped on the REAL root of this repository — the dot entries, the
# uppercase names, an accented name — never on an invented form.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-root-files.sh"
DECLARED_REAL="$REPO_ROOT/scripts/root-files-declared.txt"

PASS=0
FAIL=0

# Run the guard against ($1 = repo-dir, $2 = declared-file); assert exit == $3.
assert_exit() {
    local repo="$1" declared="$2" want="$3" name="$4"
    local got=0
    bash "$GUARD" "$repo" "$declared" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

assert_output_contains() {
    local repo="$1" declared="$2" needle="$3" name="$4"
    local out
    out="$(bash "$GUARD" "$repo" "$declared" 2>&1 || true)"
    if [[ "$out" == *"$needle"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not contain: $needle)"
        FAIL=$((FAIL + 1))
    fi
}

fail_now() {
    echo "FAIL: $1"
    FAIL=$((FAIL + 1))
}

# One root, removed by a trap. The make_* helpers run inside command
# substitutions (subshells), so registering dirs in an array would be lost —
# same reasoning as scripts/test-check-pilot-turn-ceiling-labels.sh.
FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

# The real declared list, parsed exactly as the guard parses it. Every fixture
# root below is built FROM THIS, never from a list typed into this file: a
# root-files-declared.txt emptied by accident then makes case 2 exit 3 (red)
# instead of silently passing.
REAL_DECLARED=()
while IFS= read -r line; do
    REAL_DECLARED+=("$line")
done < <(awk '
    { sub(/^[ \t]+/, ""); sub(/[ \t]+$/, ""); if ($0 == "" || $0 ~ /^#/) next; print }
' "$DECLARED_REAL")

# A fixture repo tracking exactly the paths in $@ (index only — the guard reads
# `git ls-files`, so no commit is needed and no git identity is required).
make_repo() {
    local dir f
    dir="$(mktemp -d -p "$FIXTURE_ROOT")"
    git -C "$dir" init -q
    for f in "$@"; do
        mkdir -p "$dir/$(dirname "$f")"
        # Deliberately NOT the file's own name: a fixture `.gitignore` whose
        # content is `.gitignore` would make git ignore it, and the guard would
        # then report it as a stale declaration.
        printf 'fixture content\n' > "$dir/$f"
    done
    git -C "$dir" add -A
    echo "$dir"
}

# A declared-list fixture holding the entries in $@, one per line.
make_declared() {
    local dir
    dir="$(mktemp -d -p "$FIXTURE_ROOT")"
    if [ "$#" -eq 0 ]; then
        : > "$dir/declared.txt"
    else
        printf '%s\n' "$@" > "$dir/declared.txt"
    fi
    echo "$dir/declared.txt"
}

# ── 1. POSITIVE CONTROL — this repository's own root against its own list, and
#    the green line must announce a NON-ZERO count. If this goes red, either a
#    root file arrived without its declared line or a declared line outlived
#    its file.
REAL_COUNT="$(git -C "$REPO_ROOT" -c core.quotePath=false ls-files -z \
    | tr '\0' '\n' | grep -vc '/' || true)"
if [ -z "$REAL_COUNT" ] || [ "$REAL_COUNT" -eq 0 ]; then
    fail_now "this repository tracks no file at its root — the positive control is vacuous"
else
    assert_exit "$REPO_ROOT" "$DECLARED_REAL" 0 \
        "the repo's real root matches its real declaration"
    assert_output_contains "$REPO_ROOT" "$DECLARED_REAL" \
        "$REAL_COUNT tracked root file(s) checked" \
        "the real run announces a NON-ZERO count ($REAL_COUNT)"
fi

# ── 2. NEGATIVE CONTROL, AC3 of mika#2760 — a new `.md` at the root reddens the
#    lint. The fixture root is the real declared list PLUS `NOTES.md`.
REPO_PLUS_MD="$(make_repo "${REAL_DECLARED[@]}" "NOTES.md")"
assert_exit "$REPO_PLUS_MD" "$DECLARED_REAL" 1 "a new .md at the root is rejected"
assert_output_contains "$REPO_PLUS_MD" "$DECLARED_REAL" \
    "tracked root file \`NOTES.md\` is not declared" \
    "the failure names the undeclared file"

# ── 3. The rule is on EVERY tracked root file, not only on `.md`.
REPO_PLUS_CFG="$(make_repo "${REAL_DECLARED[@]}" "setup.cfg")"
assert_exit "$REPO_PLUS_CFG" "$DECLARED_REAL" 1 "a non-markdown undeclared root file is rejected"
assert_output_contains "$REPO_PLUS_CFG" "$DECLARED_REAL" \
    "tracked root file \`setup.cfg\` is not declared" \
    "the failure names setup.cfg"

# ── 4. THE FOUNDING STATE — the three files of mika#2760, put back. All three
#    must be named: a guard that reports only the first leaves two behind.
REPO_2760="$(make_repo "${REAL_DECLARED[@]}" \
    "followup-issue.md" "followup-issue-body.md" "groom-brief-mika-789-pass2.md")"
assert_exit "$REPO_2760" "$DECLARED_REAL" 1 "the three mika#2760 files are rejected"
for f in followup-issue.md followup-issue-body.md groom-brief-mika-789-pass2.md; do
    assert_output_contains "$REPO_2760" "$DECLARED_REAL" \
        "tracked root file \`$f\` is not declared" \
        "the failure names $f"
done

# ── 5. THE OTHER DIRECTION — a declared entry that no longer matches a tracked
#    file. This is the self-cleaning assertion: the list cannot accumulate
#    residue, and a root file legitimately deleted must lose its line in the
#    same commit.
STALE_TARGET="cliff.toml"
REPO_MINUS=()
for entry in "${REAL_DECLARED[@]}"; do
    [ "$entry" = "$STALE_TARGET" ] && continue
    REPO_MINUS+=("$entry")
done
if [ "$(( ${#REAL_DECLARED[@]} - ${#REPO_MINUS[@]} ))" -ne 1 ]; then
    fail_now "\`$STALE_TARGET\` is not declared — case 5 would test nothing"
else
    REPO_STALE="$(make_repo "${REPO_MINUS[@]}")"
    assert_exit "$REPO_STALE" "$DECLARED_REAL" 1 "a stale declared entry is rejected"
    assert_output_contains "$REPO_STALE" "$DECLARED_REAL" \
        "\`$STALE_TARGET\` is declared in root-files-declared.txt but is not a tracked file" \
        "the failure names the stale entry"
fi

# ── 6. SCOPE — the rule is on the ROOT. A `.md` under a subdirectory, and a
#    plan under docs/plans/, are the normal state of this tree.
REPO_NESTED="$(make_repo "${REAL_DECLARED[@]}" \
    "sub/NOTES.md" "docs/plans/2026-10-18-001-x-plan.md")"
assert_exit "$REPO_NESTED" "$DECLARED_REAL" 0 "files outside the root are not violations"

# ── 7-10. THE WAYS AN EQUALITY GUARD PASSES FOR THE WRONG REASON — all exit 3.
REPO_OK="$(make_repo "${REAL_DECLARED[@]}")"
assert_exit "$REPO_OK" "$(make_declared)" 3 "an empty declaration file exits 3, not 0"
assert_exit "$REPO_OK" "$(make_declared '# only a comment' '' '   # another')" 3 \
    "a comments-only declaration exits 3, not 0"

REPO_NO_ROOT="$(make_repo "sub/only.md")"
assert_exit "$REPO_NO_ROOT" "$DECLARED_REAL" 3 "a repo tracking nothing at its root exits 3, not 0"

NOT_A_REPO="$(mktemp -d -p "$FIXTURE_ROOT")"
printf 'fixture content\n' > "$NOT_A_REPO/README.md"
assert_exit "$NOT_A_REPO" "$DECLARED_REAL" 3 "a directory that is not a git repo exits 3, not 0"

# ── 11. Unreadable inputs exit 2 — distinct from "read nothing" (3).
assert_exit "/nonexistent/repo" "$DECLARED_REAL" 2 "an unreadable repo-dir exits 2"
assert_exit "$REPO_OK" "/nonexistent/root-files-declared.txt" 2 "an unreadable declared-file exits 2"

# ── 12. ACCENTED NAME — our actual population writes French, and under git's
#    default quoting an accented path comes back escaped and would never equal
#    its declared line.
REPO_ACCENT="$(make_repo "${REAL_DECLARED[@]}" "archivé.md")"
DECLARED_ACCENT="$(make_declared "${REAL_DECLARED[@]}" "archivé.md")"
assert_exit "$REPO_ACCENT" "$DECLARED_ACCENT" 0 "a declared accented root file is green"
assert_exit "$REPO_ACCENT" "$DECLARED_REAL" 1 "an UNdeclared accented root file is still rejected"

echo ""
echo "check-root-files anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
