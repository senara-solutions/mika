#!/bin/bash
# Test suite for scripts/pr-origin-report.sh (mika#2026).
#
# The reader's whole job is to refuse to guess. These tests pin the three ways
# it could start lying:
#   1. counting an unmarked PR as "by hand" (the default that looks like an answer);
#   2. classifying anything at all when the marker is not deployed;
#   3. losing a genuinely marked PR because it predates the epoch.
#
# The 2026-08-27 window is the ticket's own regression case: five PRs the loop
# produced, zero of them recorded by the old `pr_url` channel. They MUST come
# back as "inconnue" — never as "à la main".
#
# `gh` is stubbed on PATH and returns a fixed fixture; the code under test is
# the script's real classifier.
#
# Run: bash scripts/test-pr-origin-report.sh
# Expected: all assertions pass, exit 0.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPORT="$SCRIPT_DIR/pr-origin-report.sh"

PASS=0
FAIL=0

assert_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    missing: '$needle'"
        echo "--- output ---"
        printf '%s\n' "$haystack"
    fi
}

assert_count() {
    # Assert the summary table row `<category>` reads `<n>`.
    local label="$1" out="$2" category="$3" expected="$4"
    local actual
    actual=$(printf '%s\n' "$out" | awk -v c="$category" '
        index($0, c) == 1 { print $NF; exit }')
    if [ "$actual" = "$expected" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label ($category = $actual)"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected $category = $expected, got '$actual'"
    fi
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# ── Fixture ──────────────────────────────────────────────────────────────────
# The five real 2026-08-27 merges (unmarked, pre-epoch), plus one PR carrying
# origin:loop that also predates the epoch, one unmarked post-epoch PR, and one
# dependabot PR.
cat > "$WORK/prs.json" <<'JSON'
[
  {"number":2014,"title":"fix(ci): exempt dependabot branches","createdAt":"2026-08-27T08:00:00Z","mergedAt":"2026-08-27T09:20:03Z","labels":[{"name":"bug"}],"author":{"login":"samidarko"},"url":"u"},
  {"number":2015,"title":"fix(llm): parse failure diagnosable","createdAt":"2026-08-27T08:10:00Z","mergedAt":"2026-08-27T09:46:54Z","labels":[],"author":{"login":"samidarko"},"url":"u"},
  {"number":2016,"title":"fix(llm): body-read failures retryable","createdAt":"2026-08-27T10:00:00Z","mergedAt":"2026-08-27T11:20:34Z","labels":[],"author":{"login":"samidarko"},"url":"u"},
  {"number":2017,"title":"fix(ci): secret-scan net","createdAt":"2026-08-27T12:00:00Z","mergedAt":"2026-08-27T13:08:37Z","labels":[],"author":{"login":"samidarko"},"url":"u"},
  {"number":2018,"title":"fix(dispatch): fermer la boucle de re-grooming","createdAt":"2026-08-27T13:00:00Z","mergedAt":"2026-08-27T13:37:15Z","labels":[],"author":{"login":"samidarko"},"url":"u"},
  {"number":2019,"title":"feat: marked loop PR before the epoch","createdAt":"2026-08-27T14:00:00Z","mergedAt":"2026-08-27T15:00:00Z","labels":[{"name":"origin:loop"}],"author":{"login":"samidarko"},"url":"u"},
  {"number":2088,"title":"fix: opened before the marker, merged after","createdAt":"2026-08-29T09:00:00Z","mergedAt":"2026-08-31T09:00:00Z","labels":[],"author":{"login":"samidarko"},"url":"u"},
  {"number":2089,"title":"fix: two producers claimed this one","createdAt":"2026-08-31T08:00:00Z","mergedAt":"2026-08-31T09:30:00Z","labels":[{"name":"origin:loop"},{"name":"origin:manual"}],"author":{"login":"samidarko"},"url":"u"},
  {"number":2090,"title":"fix: unmarked PR after the epoch","createdAt":"2026-08-31T09:00:00Z","mergedAt":"2026-08-31T10:00:00Z","labels":[{"name":"bug"}],"author":{"login":"samidarko"},"url":"u"},
  {"number":2091,"title":"chore(deps): bump serde","createdAt":"2026-08-31T10:30:00Z","mergedAt":"2026-08-31T11:00:00Z","labels":[],"author":{"login":"app/dependabot"},"url":"u"}
]
JSON

mkdir -p "$WORK/bin"
cat > "$WORK/bin/gh" <<STUB
#!/bin/bash
cat "$WORK/prs.json"
STUB
chmod +x "$WORK/bin/gh"
export PATH="$WORK/bin:$PATH"

# The producer's stamp file lives here, and starts absent: with no recorded first
# stamp the epoch is undetermined unless MIKA_PR_ORIGIN_EPOCH says otherwise.
export MIKA_PR_ORIGIN_EPOCH_FILE="$WORK/pr-origin-epoch"

run_report() { bash "$REPORT" --repo mika --since "$1" --until "$2" 2>&1; }

echo "=== pr-origin-report.sh (mika#2026) ==="

# ── 1. The ticket's regression case: 2026-08-27, epoch after it ──────────────
echo "-- 2026-08-27: five loop merges, no marker, epoch later --"
export MIKA_PR_ORIGIN_EPOCH=2026-08-30
out=$(run_report 2026-08-27 2026-08-27)
assert_count "the five unmarked merges read as unknown" "$out" "inconnue" "5"
assert_count "NOT counted as by-hand" "$out" "à la main" "0"
assert_count "NOT counted as not-loop either" "$out" "non-boucle (non marquée)" "0"
assert_count "the marked one is still counted loop despite predating the epoch" "$out" "boucle " "1"
assert_contains "the cut-off date is stated" "$out" "Coupure  : 2026-08-30T00:00:00Z"
assert_contains "and where it came from" "$out" "operator override"
assert_contains "#2014 is listed under unknown" "$out" "#2014"

# ── 2. After the epoch, silence is a fact — but only about PRs that could have
#      been stamped. A PR opened before the marker went live and merged after it
#      was never in a position to carry one.
echo "-- post-epoch window --"
out=$(run_report 2026-08-31 2026-08-31)
assert_count "unmarked PR opened after the epoch reads as not-loop" "$out" "non-boucle (non marquée)" "1"
assert_count "the in-flight PR stays unknown, not not-loop" "$out" "inconnue" "1"
assert_contains "and it is the in-flight one" "$out" "#2088"
assert_count "dependabot is its own bucket" "$out" "dependabot" "1"
assert_count "two claims on one PR surface as a conflict" "$out" "CONFLIT de marqueurs" "1"
assert_contains "the conflicting labels are shown" "$out" "origin:loop, origin:manual"
assert_count "the conflicted PR is NOT silently counted loop" "$out" "boucle " "0"

# ── 3. Marker not deployed: the report classifies NOTHING ────────────────────
# This is the property that keeps the instrument honest between merge and deploy.
echo "-- marker not deployed --"
unset MIKA_PR_ORIGIN_EPOCH
out=$(run_report 2026-08-27 2026-08-31)
assert_contains "says the cut-off is undetermined" "$out" "Coupure  : INDÉTERMINÉE"
assert_contains "warns the measure is not armed" "$out" "MESURE PAS ENCORE ARMÉE"
assert_contains "says where the cut-off will come from" "$out" "premier marquage réel"
assert_count "every unmarked PR reads unknown" "$out" "inconnue" "7"
assert_count "no PR is classified not-loop without an epoch" "$out" "non-boucle (non marquée)" "0"
assert_count "marked PRs still count" "$out" "boucle " "1"

# ── 4. Epoch read from the stamp the PRODUCER wrote ──────────────────────────
# Never from a file mtime: seed_support_dirs rewrites the installed dispatch-lib
# on every daemon start, so an mtime tracks the last restart, not the first stamp.
echo "-- epoch from the producer's own record --"
printf '2026-08-30T00:00:00Z\n' > "$MIKA_PR_ORIGIN_EPOCH_FILE"
out=$(run_report 2026-08-27 2026-08-31)
assert_contains "epoch resolves from the stamp file" "$out" "Coupure  : 2026-08-30T00:00:00Z"
assert_contains "and names that as its source" "$out" "first stamp recorded by dispatch-lib"
assert_count "pre-epoch unmarked stay unknown" "$out" "inconnue" "6"
assert_count "post-epoch unmarked become not-loop" "$out" "non-boucle (non marquée)" "1"

# An empty stamp file must not arm the epoch.
: > "$MIKA_PR_ORIGIN_EPOCH_FILE"
out=$(run_report 2026-08-27 2026-08-31)
assert_contains "an empty stamp file does not arm the epoch" "$out" "Coupure  : INDÉTERMINÉE"
printf '2026-08-30T00:00:00Z\n' > "$MIKA_PR_ORIGIN_EPOCH_FILE"

# ── 4b. Malformed window arguments are refused, never silently mis-answered ──
echo "-- malformed dates --"
rc=0; out=$(bash "$REPORT" --repo mika --since yesterday --until 2026-08-31 2>&1) || rc=$?
assert_contains "names the bad date" "$out" "pr_origin_report.bad_date"
if [ "$rc" -ne 0 ]; then
    PASS=$((PASS + 1)); echo "  ✓ refuses rather than reporting (exit $rc)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ expected non-zero exit, got 0"
fi

# A bare --until covers the whole of that day.
out=$(run_report 2026-08-31 2026-08-31)
assert_contains "a bare --until spans to end of day" "$out" "→ 2026-08-31T23:59:59Z"
assert_contains "so the last day's merges are included" "$out" "#2091"

# ── 5. A window the fetch did not cover is refused, not under-counted ───────
# A full page means there may be more the window holds and this run never saw.
echo "-- truncated window --"
export MIKA_PR_ORIGIN_EPOCH=2026-08-30
rc=0
out=$(bash "$REPORT" --repo mika --since 2026-08-01 --until 2026-09-01 --limit 10 2>&1) || rc=$?
assert_contains "names the truncation" "$out" "pr_origin_report.window_truncated"
assert_contains "says it refuses rather than reports short" "$out" "silently short"
if [ "$rc" -eq 3 ]; then
    PASS=$((PASS + 1)); echo "  ✓ refuses to report (exit 3)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ expected exit 3, got $rc"
fi

# The same window with room to spare reports normally.
rc=0
out=$(bash "$REPORT" --repo mika --since 2026-08-01 --until 2026-09-01 --limit 50 2>&1) || rc=$?
assert_contains "an uncovered-window claim is not made when the fetch fits" "$out" "TOTAL mergées"

echo
echo "PASS: $PASS  FAIL: $FAIL"
[ "$FAIL" -eq 0 ] || exit 1
