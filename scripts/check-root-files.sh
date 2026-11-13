#!/usr/bin/env bash
# CI lint: every file this repository tracks at its ROOT must be declared in
# `scripts/root-files-declared.txt`, and every declared entry must still name a
# tracked root file.
#
# THE PROPERTY: two lists that must be EQUAL.
#   Side A is `git ls-files` restricted to the root. Side B is the declared
#   list. They are the same vocabulary — the legitimate content of the repo
#   root — not a subset of one another, which is why the comparison runs both
#   ways (same reasoning as check-dispatch-seats-declared.sh, mika#2092).
#
# THE FOUNDING INCIDENT is mika#2760. Three pilot work files were tracked at the
# root of `main`:
#   - `groom-brief-mika-789-pass2.md` (127 lines), an architect-pass groom brief
#     the pilot committed in its own PR — `4dcb2c65`, 2026-05-19. It sat there
#     for five months.
#   - `followup-issue-body.md` and `followup-issue.md`, two follow-up ticket
#     bodies written for `gh issue create --body-file` and never filed, swept in
#     by the rescue sites of dispatch-lib (`b99c6219`, `50e98793`).
# No file in the repository referenced any of them, and because they were on
# `main` every new dispatch worktree carried them along.
#
# WHY A LINT AND NOT ONLY AN EXCLUSION. mika#2760 closed the rescue path at the
# source (`RESCUE_EXCLUDE_PATHSPEC` now excludes `followup-issue*.md` and
# `groom-brief-*.md`). But the May file came through the OTHER path: the pilot
# `git add`ed and committed it itself, inside its own PR, and QA review let it
# pass. No rescue exclusion can stop that one. A declaration of the root is the
# only guard that covers it — and it stays the only one that catches the symptom
# while the cause lives on (the groom commands prescribe writing the brief under
# `/tmp`, which the sandbox permission-policy refuses, so the pilot falls back to
# the worktree root: senara-solutions/mika-platform#207).
#
# WHY `ls-files` AND NOT `ls-tree HEAD`. The index counts: a draft a pilot has
# merely STAGED is already a violation, and that is the right moment to say so.
#
# WHEN YOU ADD A ROOT FILE, declare it in the same commit. "On déclare, on
# n'allowliste pas" (mika#2201): there is no exemption file here. The fix for a
# red run is a declared line, or a file moved out of the root — never an
# exception. The reverse direction keeps the list self-cleaning: a declared
# entry cannot outlive the file it declares.
#
# Usage: check-root-files.sh [repo-dir] [declared-file]
#   Both default to the repo's real files. Explicit arguments let the
#   anti-vacuity harness point the guard at fixtures (see
#   scripts/test-check-root-files.sh).
#
# Exit 0 clean, 1 on divergence in either direction, 2 if a path cannot be read,
# 3 if either side could not be read at all — zero tracked root files (or a
# directory that is not a git repository), or a declared file with no parseable
# entry. A guard that read nothing must not say "agreed".

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO_DIR="${1:-$REPO_ROOT}"
DECLARED_FILE="${2:-$REPO_ROOT/scripts/root-files-declared.txt}"

if [[ ! -d "$REPO_DIR" || ! -r "$REPO_DIR" ]]; then
    echo "ERROR: repository directory not readable: $REPO_DIR" >&2
    exit 2
fi
if [[ ! -f "$DECLARED_FILE" || ! -r "$DECLARED_FILE" ]]; then
    echo "ERROR: declaration file not readable: $DECLARED_FILE" >&2
    exit 2
fi

# ── Side A: what the repository tracks at its root.
#
# `-z` + `core.quotePath=false` because this repository's population writes in
# French: under the default quoting an accented name comes back escaped
# (`"archiv\303\251.md"`), so it would never equal its declared line. `-z` then
# protects the split from a path containing a newline.
#
# `|| true` on the pipeline: `grep -v '/'` exits 1 when every tracked file lives
# in a subdirectory, and `git ls-files` exits non-zero when $REPO_DIR is not a
# git repository. Under `pipefail` both would abort here; each is an EMPTY side
# A, reported as exit 3 below rather than as a crash.
extract_root_files() {
    git -C "$1" -c core.quotePath=false ls-files -z 2>/dev/null \
        | tr '\0' '\n' \
        | grep -v '/' || true
}

# ── Side B: the declared list. One entry per line; blank lines and `#` comments
# are ignored, surrounding whitespace is trimmed.
extract_declared_files() {
    awk '
        {
            sub(/^[ \t]+/, "")
            sub(/[ \t]+$/, "")
            if ($0 == "" || $0 ~ /^#/) { next }
            print
        }
    ' "$1"
}

TRACKED="$(extract_root_files "$REPO_DIR")"
if [[ -z "$TRACKED" ]]; then
    echo "ERROR: no tracked file at the root of $REPO_DIR" >&2
    echo "Either that directory is not a git repository, or it tracks nothing at its" >&2
    echo "root. A guard that read nothing must not report agreement." >&2
    exit 3
fi

DECLARED="$(extract_declared_files "$DECLARED_FILE")"
if [[ -z "$DECLARED" ]]; then
    echo "ERROR: $DECLARED_FILE holds no parseable entry (empty, or comments only)." >&2
    echo "Expected one root file name per line. An empty declaration would refuse" >&2
    echo "every root file, or accept every one, depending on which way it is read —" >&2
    echo "in both cases the guard no longer measures the repository." >&2
    exit 3
fi

# ── Compare the two sets, both ways. `comm` on two LC_ALL=C-sorted lists, not a
# double `grep -qxF` loop.
#
# THE LOCALE IS PINNED ON `comm` AS WELL AS ON `sort`, and that is not belt and
# braces. `comm` compares with the collation of ITS OWN environment: given input
# sorted in C but compared under a UTF-8 locale, the two streams de-synchronise
# at the first entry the two collations order differently (here `.env.example`
# and the uppercase names), and `comm` then reports a divergence that is real
# in neither direction — measured against this harness, which went red on four
# named-file assertions while still exiting 1 for the wrong reason. The locale is
# set here rather than inherited: the CI runner does not carry the operator's.
VIOLATIONS=0

UNDECLARED="$(LC_ALL=C comm -23 \
    <(printf '%s\n' "$TRACKED" | LC_ALL=C sort) \
    <(printf '%s\n' "$DECLARED" | LC_ALL=C sort))"

STALE="$(LC_ALL=C comm -13 \
    <(printf '%s\n' "$TRACKED" | LC_ALL=C sort) \
    <(printf '%s\n' "$DECLARED" | LC_ALL=C sort))"

while IFS= read -r file; do
    [[ -z "$file" ]] && continue
    echo "ERROR: tracked root file \`$file\` is not declared in $(basename "$DECLARED_FILE")"
    VIOLATIONS=$((VIOLATIONS + 1))
done <<< "$UNDECLARED"

while IFS= read -r file; do
    [[ -z "$file" ]] && continue
    echo "ERROR: \`$file\` is declared in $(basename "$DECLARED_FILE") but is not a tracked file at the repo root"
    VIOLATIONS=$((VIOLATIONS + 1))
done <<< "$STALE"

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS undeclared or stale root file(s)."
    echo ""
    echo "Why this is blocking (mika#2760): a pilot work file committed at the root is"
    echo "read by nobody, referenced by nothing, and every dispatch worktree carries it"
    echo "along from \`main\` — three did, one of them for five months. No rescue"
    echo "exclusion stops the one the pilot commits itself."
    echo ""
    echo "Fix, in the SAME commit as the file:"
    echo "  - a file that belongs at the root → add its name to $DECLARED_FILE"
    echo "  - a work draft → put it under .pilot-scratch/ (dispatch rule mika#2548)"
    echo "    or file it as an issue; it does not belong in the tree"
    echo "  - a root file legitimately deleted → remove its declared line too"
    exit 1
fi

COUNT="$(printf '%s\n' "$TRACKED" | grep -c .)"
echo "Root files are declared ($COUNT tracked root file(s) checked)."
exit 0
