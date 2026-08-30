#!/usr/bin/env bash
# pr-origin-report.sh — merged-PR count over a window, split by origin (mika#2026)
#
# The origin of a PR — produced by the autonomous loop, or opened by hand — is
# read from a label stamped by its PRODUCER at the moment of production
# (`origin:loop`, applied in shell by dispatch-lib's `_stamp_pr_origin`).
#
# What this script deliberately does NOT do: infer origin from a branch name, an
# author, or a time window. The orchestrator uses the same `derive-branch-name`
# as the loop and both push as `samidarko`, so all three inferences are wrong
# exactly on the day the answer matters.
#
# An unmarked PR reads "unknown", never "by hand". A default that looks like an
# answer is how an instrument lies.
#
# Usage:
#   scripts/pr-origin-report.sh [--repo mika] [--since <ISO|YYYY-MM-DD>]
#                               [--until <ISO|YYYY-MM-DD>] [--limit N]
#
# Defaults: --repo mika, window = last 24h, --limit 200.

set -euo pipefail

REPO="mika"
SINCE=""
UNTIL=""
LIMIT="200"

usage() {
    sed -n '2,21p' "$0" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --repo)  REPO="${2:?--repo needs a value}";  shift 2 ;;
        --since) SINCE="${2:?--since needs a value}"; shift 2 ;;
        --until) UNTIL="${2:?--until needs a value}"; shift 2 ;;
        --limit) LIMIT="${2:?--limit needs a value}"; shift 2 ;;
        -h|--help) usage 0 ;;
        *) echo "unknown argument: $1" >&2; usage 2 ;;
    esac
done

# Normalise a date argument to an ISO-8601 UTC instant, then insist on the shape.
# A bare YYYY-MM-DD means the whole of that day, UTC: midnight for --since, the
# last second for --until. An `--until 2026-08-31` that silently dropped Aug 31
# would be one more instrument answering a question nobody asked.
#
# Anything else is refused outright. `--since yesterday` or `--since 08/30/2026`
# would sail through a lexical string comparison and produce an empty or wrong
# window — a confident, silent, wrong answer, which is the one failure mode this
# instrument exists to refuse.
_norm_instant() {
    local v="$1" bound="$2" out
    case "$v" in
        [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9])
            if [ "$bound" = "until" ]; then out="${v}T23:59:59Z"; else out="${v}T00:00:00Z"; fi ;;
        *) out="$v" ;;
    esac
    case "$out" in
        [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z) ;;
        *)
            echo "pr_origin_report.bad_date: --${bound} '${v}' is neither YYYY-MM-DD nor an ISO-8601 UTC instant (YYYY-MM-DDTHH:MM:SSZ). Refusing rather than reporting a window that may not be the one you asked for." >&2
            return 2 ;;
    esac
    printf '%s' "$out"
}

[ -n "$SINCE" ] || SINCE=$(date -u -d '24 hours ago' +%Y-%m-%dT%H:%M:%SZ)
[ -n "$UNTIL" ] || UNTIL=$(date -u +%Y-%m-%dT%H:%M:%SZ)
SINCE=$(_norm_instant "$SINCE" since)
UNTIL=$(_norm_instant "$UNTIL" until)

# ── Marker epoch ─────────────────────────────────────────────────────────────
#
# The epoch is the instant from which the ABSENCE of `origin:loop` is informative.
# A PR that CARRIES the label counts as loop whatever its date; the epoch only
# governs what silence means.
#
# It is never hand-written into this file, and never inferred from a file's mtime.
# The producer records it — once — the first time it successfully stamps anything
# (`_record_pr_origin_epoch` in dispatch-lib.sh). An mtime would be worse than
# useless here: `seed_support_dirs` rewrites the installed dispatch-lib.sh on every
# daemon start, so its mtime tracks the last restart and the cut-off would walk
# forward all day, quietly re-opening the blind window after every bounce.
#
# Resolution order:
#   1. $MIKA_PR_ORIGIN_EPOCH — explicit operator override (ISO-8601).
#   2. The producer's own stamp file.
#   3. Undetermined → classify nothing, and say so.
EPOCH=""
EPOCH_SOURCE=""
EPOCH_FILE="${MIKA_PR_ORIGIN_EPOCH_FILE:-${MIKA_HOME:-$HOME/.mika}/state/pr-origin-epoch}"

if [ -n "${MIKA_PR_ORIGIN_EPOCH:-}" ]; then
    EPOCH=$(_norm_instant "$MIKA_PR_ORIGIN_EPOCH" since)
    EPOCH_SOURCE="MIKA_PR_ORIGIN_EPOCH (operator override)"
elif [ -r "$EPOCH_FILE" ] && [ -s "$EPOCH_FILE" ]; then
    EPOCH=$(_norm_instant "$(tr -d '[:space:]' < "$EPOCH_FILE")" since)
    EPOCH_SOURCE="first stamp recorded by dispatch-lib ($EPOCH_FILE)"
else
    EPOCH_SOURCE="undetermined — dispatch-lib has never recorded a first stamp at $EPOCH_FILE"
fi

# Fetch on the SAME axis the window filters on. `gh pr list --state merged` orders
# and pages by CREATION date, so a long-lived PR — a wip-rescue draft can sit open
# for weeks (mika#1631) — created before the fetched page but merged inside the
# window would simply be absent, and the count would be quietly short.
PRS_JSON=$(gh pr list \
    --repo "senara-solutions/${REPO}" \
    --state merged \
    --search "merged:>=${SINCE} merged:<=${UNTIL}" \
    --limit "$LIMIT" \
    --json number,title,createdAt,mergedAt,labels,author,url)

PRS_FILE=$(mktemp)
trap 'rm -f "$PRS_FILE"' EXIT
printf '%s' "$PRS_JSON" > "$PRS_FILE"

REPORT=$(
    SINCE="$SINCE" UNTIL="$UNTIL" EPOCH="$EPOCH" LIMIT="$LIMIT" PRS_FILE="$PRS_FILE" \
    python3 <<'PYEOF'
import json, os, sys

with open(os.environ["PRS_FILE"]) as fh:
    prs = json.load(fh)
since = os.environ["SINCE"]
until = os.environ["UNTIL"]
epoch = os.environ["EPOCH"]

ORDER = ["loop", "spawn", "manual", "dependabot", "not-loop (unmarked)",
         "unknown", "conflict"]
LABEL_FR = {
    "loop":                "boucle",
    "spawn":               "spawn",
    "manual":              "\u00e0 la main",
    "dependabot":          "dependabot",
    "not-loop (unmarked)": "non-boucle (non marqu\u00e9e)",
    "unknown":             "inconnue",
    "conflict":            "CONFLIT de marqueurs",
}
VALUES = {"loop", "spawn", "manual"}
buckets = {k: [] for k in ORDER}

# Truncation guard. The fetch is bounded by --limit; a full page means there may
# be more the window contains and this run never saw. An under-count that looks
# exactly like a real one is the failure this whole ticket is about, so refuse to
# report rather than quietly report short.
limit = int(os.environ["LIMIT"])
if len(prs) >= limit:
    sys.stderr.write(
        "pr_origin_report.window_truncated: gh returned {n} PRs, the full --limit {lim}. "
        "The window may hold more than this run saw; re-run with a larger --limit. "
        "Refusing to report a count that could be silently short.\n".format(
            n=len(prs), lim=limit))
    sys.exit(3)

for pr in prs:
    merged = pr.get("mergedAt") or ""
    if not (since <= merged < until):
        continue
    names = {l["name"] for l in pr.get("labels", [])}
    author = (pr.get("author") or {}).get("login", "")
    marks = sorted(n for n in names if n.startswith("origin:") and n[7:] in VALUES)
    created = pr.get("createdAt") or ""

    # dependabot is not an inference: the author IS the producer. It is the one
    # case where authorship states an origin instead of guessing at one.
    if author in ("app/dependabot", "dependabot", "dependabot[bot]"):
        cat = "dependabot"
    elif len(marks) > 1:
        # Two producers claimed the same PR. Resolving that by precedence would
        # pick a winner and hide the disagreement; surface it instead.
        cat = "conflict"
    elif marks:
        cat = marks[0][7:]
    elif epoch and created and created >= epoch:
        # Gate on creation, not merge. The stamp goes on when the PR is produced,
        # so a loop PR opened before the marker went live and merged after it was
        # never in a position to be stamped — calling that "not-loop" would be a
        # confident false answer about exactly the PRs in flight across the cutover.
        cat = "not-loop (unmarked)"
    else:
        # Before the epoch (or with no epoch at all) silence says nothing.
        cat = "unknown"
    buckets[cat].append(pr)

total = sum(len(v) for v in buckets.values())
w = max(len(LABEL_FR[k]) for k in ORDER)
print("Origine".ljust(w) + "  Nombre")
print("\u2500" * (w + 8))
for k in ORDER:
    print(LABEL_FR[k].ljust(w) + "  " + str(len(buckets[k])).rjust(5))
print("\u2500" * (w + 8))
print("TOTAL merg\u00e9es".ljust(w) + "  " + str(total).rjust(5))

for k in ORDER:
    if not buckets[k]:
        continue
    print()
    print("\u2500\u2500 " + LABEL_FR[k] + " (" + str(len(buckets[k])) + ")")
    for pr in sorted(buckets[k], key=lambda p: p["mergedAt"]):
        num = ("#" + str(pr["number"])).ljust(7)
        extra = ""
        if k == "conflict":
            extra = "  [" + ", ".join(sorted(
                n for n in (l["name"] for l in pr.get("labels", []))
                if n.startswith("origin:"))) + "]"
        print("   " + num + " " + pr["mergedAt"] + "  " + pr["title"][:70] + extra)
PYEOF
)

echo "Répartition des PR mergées par origine — senara-solutions/${REPO}"
echo "Fenêtre : ${SINCE} → ${UNTIL} (UTC)"
if [ -n "$EPOCH" ]; then
    echo "Coupure  : ${EPOCH}"
    echo "Source   : ${EPOCH_SOURCE}"
    echo
    echo "Une PR ouverte avant la coupure n'a jamais pu être marquée : elle est « inconnue »,"
    echo "jamais « à la main ». Après, le producteur marque tout ce qu'il ouvre — le silence"
    echo "y est donc un fait. Le test porte sur la date d'OUVERTURE, pas de merge : une PR"
    echo "à cheval sur la mise en service ne peut pas être déclarée non-boucle."
else
    echo "Coupure  : INDÉTERMINÉE"
    echo "Source   : ${EPOCH_SOURCE}"
    echo
    echo "⚠  MESURE PAS ENCORE ARMÉE. Aucune PR non marquée ne peut être classée : tout ce"
    echo "   qui ne porte pas de label origin:* est compté « inconnue ». Ce rapport ne devine pas."
    echo "   La coupure naît du premier marquage réel, pas du déploiement : après \`make deploy\`,"
    echo "   elle s'inscrit au premier dispatch qui ouvre une PR."
    echo "   Coupure connue par ailleurs :  MIKA_PR_ORIGIN_EPOCH=2026-08-30 $0 ..."
fi
echo
echo "$REPORT"
