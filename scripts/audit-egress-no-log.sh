#!/usr/bin/env bash
# Runtime audit runbook for the egress-search no-log invariant (mika#1810 E4).
#
# Runs against a live-or-lived-in mika-gateway environment (dev or prod).
# Complements `scripts/verify-egress-no-log.sh` (build-time source lint) with
# runtime checks the source lint cannot cover: real log content, live env
# config, on-disk DB state.
#
# Layer coverage (mirrors the runbook doc — crates/mika-gateway/docs/egress-search-no-log-audit.md):
#
#   Layer 1 — application logs   — CHECKED (source lint + this script)
#   Layer 2 — network metadata   — MANUAL AUDIT REQUIRED (iptables/nft, proxy)
#   Layer 3 — persistence         — CHECKED (source lint + this script)
#
# Exit codes:
#   0 — every automated check clean AND Layer 2 carries a valid attestation
#   1 — leak detected (any layer)
#   2 — Layer 2 attestation absent, unreadable, expired or off-target
#   3 — usage error (an `--attest` invocation that cannot be honoured)
#
# LAYER 2 IS ATTESTED, NOT ASSUMED (mika#1806).
#
# Layer 2 is a *substrate* property (kernel iptables, K8s NetworkPolicy, proxy
# config) that this script cannot inspect in-container, so the only control
# available on this side is the operator's statement that they checked. Until
# mika#1806 that statement was the environment variable
# `MIKA_AUDIT_SUPPRESS_L2_WARN=1`, which the runbook called "the operator's
# signed statement". It was signed with nothing: it carried no instant, no
# target and no expiry, so once it sat in a cron file, an EnvironmentFile or a
# shell profile it printed "all layers clean" on every later run, for ever,
# with no verification having taken place. "Attested a minute ago on this
# cluster" and "attested six months ago on another" produced the same bytes.
#
# It is now an attestation RECORD — a small JSON file that names when, on what,
# and by whom. Every term is read fail-closed: absent, unreadable, undated,
# expired or naming another target all mean *owed*, i.e. exit 2 — which is the
# state of every installation that has not attested yet, so nothing regresses.
# A PASS now prints the instant, the target and the author, so a green can no
# longer be read without knowing what it is green about.
#
#   Write:  scripts/audit-egress-no-log.sh --attest \
#               --target mika-cloud/prd/gateway --by samidarko
#   Read:   scripts/audit-egress-no-log.sh          (every ordinary run)
#
# Environment (all optional; sensible defaults for a dev machine):
#   MIKA_GATEWAY_LOG_FILE        — path to mika-gateway JSON log file.
#                                  Default: $MIKA_SPIRIT_LOG_FILE or
#                                  ~/.mika/logs/mika-gateway.log
#   MIKA_GATEWAY_DB              — path to the gateway/spirit SQLite DB.
#                                  Default: $HOME/.mika/data/mika.db
#   MIKA_EGRESS_L2_ATTESTATION_FILE
#                                — path to the Layer 2 attestation record.
#                                  Default: ~/.mika/state/egress-l2-attestation.json
#   MIKA_EGRESS_L2_ATTESTATION_TTL_DAYS
#                                — how long an attestation stays valid.
#                                  Default 30. Absent/empty → default;
#                                  unreadable, 0 or negative → default with a
#                                  WARN naming the value. `0` deliberately does
#                                  NOT disarm the expiry: on a doctrinal
#                                  control, a typo that made an attestation
#                                  eternal is the very failure this closes.
#   MIKA_EGRESS_L2_TARGET        — when set, the deployment this run is auditing.
#                                  An attestation recorded against another
#                                  target is refused, both values named.
#   MIKA_AUDIT_SUPPRESS_L2_WARN  — REMOVED (mika#1806). Still set? The script
#                                  says so and changes no exit code; it no
#                                  longer suppresses anything.
#
# WHY A FILE AND NOT A DATABASE ROW. The script must work during an incident,
# on the gateway side, with no database reachable. `~/.mika/state/` is the
# house location for exactly this kind of state — `pr-origin-epoch`
# (mika#2026), `auto-pull-stop` (mika#2329), `pilot-gitconfig`.
#
# WHY NO `jq`. The record is parsed by one reader, written below, that needs no
# external tool: this script runs on bare gateway hosts where `jq` is not a
# given, and a second parser behind a `command -v` is a second predicate that
# can drift from the first. The parser is deliberately minimal and fails
# closed on anything it cannot read.

set -euo pipefail

readonly EXIT_OK=0
readonly EXIT_LEAK=1
readonly EXIT_L2_OWED=2
readonly EXIT_USAGE=3

LOG_FILE="${MIKA_GATEWAY_LOG_FILE:-${MIKA_SPIRIT_LOG_FILE:-$HOME/.mika/logs/mika-gateway.log}}"
DB_PATH="${MIKA_GATEWAY_DB:-$HOME/.mika/data/mika.db}"
ATTESTATION_FILE="${MIKA_EGRESS_L2_ATTESTATION_FILE:-$HOME/.mika/state/egress-l2-attestation.json}"
EXPECTED_TARGET="${MIKA_EGRESS_L2_TARGET:-}"

readonly ATTESTATION_TTL_DEFAULT_DAYS=30
# An attestation record is a handful of fields. Anything larger is not one, and
# refusing to slurp it keeps a mis-pointed path from reading a log file into a
# shell variable.
readonly ATTESTATION_MAX_BYTES=65536

leak_found=0
warnings=0

# ------------------------------------------------------------------
# Usage / argument parsing
# ------------------------------------------------------------------

usage() {
    cat <<'USAGE'
usage: audit-egress-no-log.sh [--attest --target <deployment> --by <author>]

  (no arguments)   run the three-layer audit and read the Layer 2 attestation
  --attest         record a Layer 2 attestation instead of auditing
  --target <t>     the deployment the attestation covers (required with --attest)
  --by <who>       who is attesting (required with --attest)

exit 0 = every layer clean, Layer 2 attested
exit 1 = leak detected
exit 2 = Layer 2 owed (no valid, in-date, on-target attestation)
exit 3 = usage error
USAGE
}

MODE="audit"
ATTEST_TARGET=""
ATTEST_BY=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --attest)
            MODE="attest"
            shift
            ;;
        --target)
            [[ $# -ge 2 ]] || { echo "audit-egress-no-log.sh: --target needs a value." >&2; usage >&2; exit "$EXIT_USAGE"; }
            ATTEST_TARGET="$2"
            shift 2
            ;;
        --by)
            [[ $# -ge 2 ]] || { echo "audit-egress-no-log.sh: --by needs a value." >&2; usage >&2; exit "$EXIT_USAGE"; }
            ATTEST_BY="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit "$EXIT_OK"
            ;;
        *)
            echo "audit-egress-no-log.sh: unknown argument '$1'." >&2
            usage >&2
            exit "$EXIT_USAGE"
            ;;
    esac
done

# ------------------------------------------------------------------
# Attestation record — one writer, one reader, every term fail-closed
# ------------------------------------------------------------------

# The TTL in force for THIS run. The record also carries the `ttl_days` that was
# in force when it was written; that field documents the policy of the day, the
# decision below applies the policy of now.
resolve_ttl_days() {
    local raw="${MIKA_EGRESS_L2_ATTESTATION_TTL_DAYS:-}"
    if [[ -z "$raw" ]]; then
        printf '%s' "$ATTESTATION_TTL_DEFAULT_DAYS"
        return 0
    fi
    # Bounded digit count before any arithmetic: a 30-digit "number" would wrap
    # in shell arithmetic and could read as positive. Anything that long is not
    # a day count, so it joins the unreadable tier rather than being clamped.
    if [[ ! "$raw" =~ ^-?[0-9]{1,9}$ ]] || (( raw <= 0 )); then
        echo "  WARN: MIKA_EGRESS_L2_ATTESTATION_TTL_DAYS is \"$raw\" — not a positive" >&2
        echo "        number of days. Falling back to $ATTESTATION_TTL_DEFAULT_DAYS." >&2
        printf '%s' "$ATTESTATION_TTL_DEFAULT_DAYS"
        return 0
    fi
    printf '%s' "$raw"
}

# Echo the string value of a top-level field, or nothing. No pipes: a
# `producer | head` form takes SIGPIPE under `pipefail` and would make a
# PRESENT field read as absent (the shape `scripts/verify-no-sigpipe-grep.sh`
# exists to reject).
attestation_field() {
    local name="$1" raw value
    raw=$(grep -m1 -oE "\"$name\"[[:space:]]*:[[:space:]]*\"[^\"]*\"" "$ATTESTATION_FILE" 2>/dev/null || true)
    [[ -n "$raw" ]] || return 0
    value="${raw#*:}"
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value#\"}"
    value="${value%\"}"
    printf '%s' "$value"
}

# RFC 3339 UTC, and only that shape. The regex is the strict half: `date -d`
# alone would happily read "2026" as a year and hand back an epoch, which is
# how a malformed record becomes a green.
parse_attested_at_epoch() {
    local v="$1" epoch=""
    [[ "$v" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]] || return 1
    epoch=$(date -u -d "$v" +%s 2>/dev/null) || epoch=""
    if [[ -z "$epoch" ]]; then
        # BSD/macOS date, for an operator auditing from a laptop.
        epoch=$(date -u -j -f "%Y-%m-%dT%H:%M:%SZ" "$v" +%s 2>/dev/null) || epoch=""
    fi
    [[ -n "$epoch" ]] || return 1
    printf '%s' "$epoch"
}

# Escape a value for embedding in the JSON record. The two characters JSON
# cannot carry raw inside a string are the backslash and the double quote;
# escaping them in that order is what keeps `--target 'a"b'` from producing a
# record its own reader would reject.
json_escape() {
    local v="$1"
    v="${v//\\/\\\\}"
    v="${v//\"/\\\"}"
    printf '%s' "$v"
}

write_attestation() {
    local target="$1" by="$2" ttl_days="$3"
    local now dir tmp

    if [[ -z "${target//[[:space:]]/}" ]] || [[ -z "${by//[[:space:]]/}" ]]; then
        echo "audit-egress-no-log.sh: --attest requires BOTH --target and --by." >&2
        echo "  An attestation that names neither what it covers nor who made it" >&2
        echo "  is the environment variable this record replaced." >&2
        usage >&2
        return "$EXIT_USAGE"
    fi

    now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    dir=$(dirname "$ATTESTATION_FILE")
    mkdir -p "$dir"
    tmp="$ATTESTATION_FILE.tmp.$$"

    cat > "$tmp" <<JSON
{
  "attested_at": "$(json_escape "$now")",
  "target": "$(json_escape "$target")",
  "attested_by": "$(json_escape "$by")",
  "ttl_days": $ttl_days
}
JSON
    # Atomic: a reader never sees a half-written record, and a failed write
    # leaves the previous attestation (or its absence) intact.
    mv -f "$tmp" "$ATTESTATION_FILE"

    echo "Layer 2 attestation recorded."
    echo "  file:        $ATTESTATION_FILE"
    echo "  attested_at: $now"
    echo "  target:      $target"
    echo "  attested_by: $by"
    echo "  ttl_days:    $ttl_days"
    return "$EXIT_OK"
}

# Sets L2_VERDICT to "attested" or "owed" and fills L2_LINES with what to print.
L2_VERDICT="owed"
L2_LINES=()

evaluate_attestation() {
    local ttl_days="$1"
    local size raw_compact attested_at target by epoch now age_secs ttl_secs age_days

    L2_VERDICT="owed"
    L2_LINES=()

    if [[ ! -f "$ATTESTATION_FILE" ]]; then
        L2_LINES+=("OWED: no Layer 2 attestation at $ATTESTATION_FILE")
        L2_LINES+=("      Record one once the substrate side has been checked:")
        L2_LINES+=("        scripts/audit-egress-no-log.sh --attest \\")
        L2_LINES+=("            --target <deployment> --by <who>")
        return 0
    fi

    size=$(wc -c < "$ATTESTATION_FILE" 2>/dev/null || echo 0)
    if [[ ! "$size" =~ ^[0-9]+$ ]] || (( size == 0 )) || (( size > ATTESTATION_MAX_BYTES )); then
        L2_LINES+=("OWED: attestation at $ATTESTATION_FILE is not a readable record")
        L2_LINES+=("      (size: $size bytes). An unreadable attestation is not an attestation.")
        return 0
    fi

    # Minimal structural check: a JSON object, and nothing else. Deliberately
    # coarse — the point is to refuse what this parser cannot model, not to
    # validate JSON.
    raw_compact=$(tr -d '[:space:]' < "$ATTESTATION_FILE" 2>/dev/null || true)
    if [[ "$raw_compact" != "{"*"}" ]]; then
        L2_LINES+=("OWED: attestation at $ATTESTATION_FILE is not a JSON object.")
        return 0
    fi

    attested_at=$(attestation_field "attested_at")
    target=$(attestation_field "target")
    by=$(attestation_field "attested_by")

    if [[ -z "$attested_at" ]]; then
        L2_LINES+=("OWED: attestation at $ATTESTATION_FILE carries no \"attested_at\".")
        L2_LINES+=("      An undated attestation is the environment variable it replaced.")
        return 0
    fi

    if ! epoch=$(parse_attested_at_epoch "$attested_at"); then
        L2_LINES+=("OWED: attestation \"attested_at\" is \"$attested_at\" — not an RFC 3339")
        L2_LINES+=("      UTC instant (expected YYYY-MM-DDTHH:MM:SSZ).")
        return 0
    fi

    now=$(date -u +%s)
    age_secs=$(( now - epoch ))
    ttl_secs=$(( ttl_days * 86400 ))

    if (( age_secs < 0 )); then
        # Not in the plan's table, and refused on the same reasoning as the rest
        # of it: an attestation dated in the future cannot be the record of a
        # check that happened, and left valid it is the one hand-edit that buys
        # an eternal green.
        L2_LINES+=("OWED: attestation is dated in the future ($attested_at).")
        L2_LINES+=("      An attestation records a check that has taken place.")
        return 0
    fi

    age_days=$(( age_secs / 86400 ))

    if (( age_secs > ttl_secs )); then
        L2_LINES+=("OWED: attestation has expired — attested $attested_at")
        L2_LINES+=("      (${age_days}d ago), TTL is ${ttl_days}d.")
        L2_LINES+=("      A network attestation covers a topology that moves at every")
        L2_LINES+=("      rotation. Re-check the substrate side, then re-attest.")
        return 0
    fi

    if [[ -n "$EXPECTED_TARGET" ]] && [[ "$target" != "$EXPECTED_TARGET" ]]; then
        L2_LINES+=("OWED: attestation covers another deployment.")
        L2_LINES+=("        attested target: \"$target\"")
        L2_LINES+=("        expected target: \"$EXPECTED_TARGET\"  (MIKA_EGRESS_L2_TARGET)")
        L2_LINES+=("      The honest discriminant of a network attestation is not its age")
        L2_LINES+=("      but the topology it covered.")
        return 0
    fi

    L2_VERDICT="attested"
    L2_LINES+=("ATTESTED: Layer 2 confirmed out-of-band.")
    L2_LINES+=("        attested_at: $attested_at (${age_days}d ago, TTL ${ttl_days}d)")
    L2_LINES+=("        target:      ${target:-<unnamed>}")
    L2_LINES+=("        attested_by: ${by:-<unnamed>}")
    L2_LINES+=("        record:      $ATTESTATION_FILE")
    return 0
}

TTL_DAYS=$(resolve_ttl_days)

if [[ "$MODE" == "attest" ]]; then
    write_attestation "$ATTEST_TARGET" "$ATTEST_BY" "$TTL_DAYS"
    exit $?
fi

if [[ -n "$ATTEST_TARGET" ]] || [[ -n "$ATTEST_BY" ]]; then
    echo "audit-egress-no-log.sh: --target/--by are only meaningful with --attest." >&2
    usage >&2
    exit "$EXIT_USAGE"
fi

emit_header() {
    echo ""
    echo "=========================================================="
    echo "  $1"
    echo "=========================================================="
}

# ------------------------------------------------------------------
# Layer 1a — env config for verbosity / storage flags
# ------------------------------------------------------------------

emit_header "Layer 1a — env config (verbosity / storage flags)"

# MIKA_LOG_LEVEL / RUST_LOG — surface if elevated to trace/debug on the gateway.
current_level="${RUST_LOG:-${MIKA_LOG_LEVEL:-info}}"
echo "  RUST_LOG / MIKA_LOG_LEVEL = $current_level"
case "$current_level" in
    *trace*|*TRACE*|*debug*|*DEBUG*)
        echo "  NOTE: elevated log level detected. The E4 invariant survives"
        echo "        because the substrate emits ZERO debug/trace lines — but"
        echo "        third-party crates (reqwest/hyper) may become chattier."
        echo "        Re-run the log-content check (Layer 1c) with production"
        echo "        traffic and confirm no query bytes leak into their lines."
        ;;
    *)
        echo "  OK: log level is not elevated to trace/debug."
        ;;
esac

# MIKA_STORE_LLM_CALLS / MIKA_STORE_TOOL_CALLS — orthogonal to egress-search
# (they gate agent-side persistence, not gateway egress). Reported here for
# completeness so an operator running this on a mika-agent host can spot a
# misconfiguration that would persist query content downstream.
echo "  MIKA_STORE_LLM_CALLS  = ${MIKA_STORE_LLM_CALLS:-<unset, default true>}"
echo "  MIKA_STORE_TOOL_CALLS = ${MIKA_STORE_TOOL_CALLS:-<unset, default true>}"
echo "  MIKA_LOG_LLM_BODIES   = ${MIKA_LOG_LLM_BODIES:-<unset, default false>}"
echo "  (These gate agent-side storage of LLM/tool traffic — not gateway"
echo "   egress-search. The substrate never touches them.)"

# ------------------------------------------------------------------
# Layer 1b — log file audit: shape check on emitted search_* events
# ------------------------------------------------------------------

emit_header "Layer 1b — log file audit (search_* event shape)"

if [[ ! -f "$LOG_FILE" ]]; then
    echo "  SKIP: log file not found at $LOG_FILE"
    echo "  Set MIKA_GATEWAY_LOG_FILE to the actual path and re-run."
else
    echo "  Log file: $LOG_FILE"

    # Count each event name — if the substrate emitted anything else, list it.
    total_requested=$(grep -c '"search_requested"' "$LOG_FILE" 2>/dev/null || echo 0)
    total_egress=$(grep -c '"search_egress"' "$LOG_FILE" 2>/dev/null || echo 0)
    echo "  search_requested events: $total_requested"
    echo "  search_egress events:    $total_egress"

    # Any other event name with 'search' in it that isn't one of the two
    # allowlisted events is a discipline break.
    unexpected=$(grep -oE '"event":"search[^"]*"' "$LOG_FILE" 2>/dev/null \
        | sort -u \
        | grep -Ev '"event":"search_(requested|egress)"' \
        || true)
    if [[ -n "$unexpected" ]]; then
        echo "  LEAK (Layer 1): unexpected search_* event names in log:"
        printf "%s\n" "$unexpected" | sed 's/^/    /'
        leak_found=1
    else
        echo "  OK: only allowlisted event names appear on search_* lines."
    fi

    # Field-shape check on the two allowlisted events. If jq is available, use
    # it — otherwise fall back to a coarser grep-based rejection list.
    if command -v jq >/dev/null 2>&1; then
        forbidden_field_hits=$(
            grep '"search_requested"\|"search_egress"' "$LOG_FILE" 2>/dev/null \
            | jq -c 'select(
                has("query") or has("tenant_id") or has("tenant_hash") or
                has("user_id") or has("chat_id") or has("customer_id") or
                has("api_key") or has("retry_after") or has("url")
            )' 2>/dev/null \
            | head -5 \
            || true
        )
        if [[ -n "$forbidden_field_hits" ]]; then
            echo "  LEAK (Layer 1): forbidden field name on a search_* event (first 5):"
            printf "%s\n" "$forbidden_field_hits" | sed 's/^/    /'
            leak_found=1
        else
            echo "  OK: no forbidden field names (query/tenant/user/chat/api_key/url)"
            echo "      appeared on search_* events."
        fi
    else
        echo "  NOTE: jq not installed — falling back to grep-based field check."
        for forbidden_field in '"query":' '"tenant_id":' '"tenant_hash":' \
                                '"user_id":' '"chat_id":' '"customer_id":' \
                                '"api_key":' '"retry_after":' '"url":'; do
            hits=$(grep '"search_requested"\|"search_egress"' "$LOG_FILE" 2>/dev/null \
                | grep -F "$forbidden_field" \
                | head -3 \
                || true)
            if [[ -n "$hits" ]]; then
                echo "  LEAK (Layer 1): field '$forbidden_field' on search_* line:"
                printf "%s\n" "$hits" | sed 's/^/    /'
                leak_found=1
            fi
        done
    fi
fi

# ------------------------------------------------------------------
# Layer 2 — network metadata SPEC (MANUAL AUDIT REQUIRED)
# ------------------------------------------------------------------

emit_header "Layer 2 — network metadata (MANUAL AUDIT REQUIRED)"

cat <<'EOF'
  Layer 2 is a substrate property (iptables/nft, proxy config, K8s
  NetworkPolicy) that this script CANNOT inspect from user-space. See
  crates/mika-gateway/docs/egress-search-no-log-audit.md § Layer 2 for
  the required checks:

    * iptables/nft rules on the mika-gateway egress chain MUST NOT
      carry `--log-prefix` or `NFLOG` targets on the search-upstream
      hops (RFC-recommended: absent entirely).
    * HAProxy / Envoy / nginx (if in path):
        - HAProxy: `option httplog` OFF on the search-upstream backend,
          or `no log` on the frontend for that route.
        - Envoy: `access_log: []` on the listener carrying search
          upstream traffic.
        - nginx: `access_log off;` on the location serving the
          search-upstream proxy path.
    * cloud/K8s VPC flow logs — enabled at broad scope must exclude
      the search-upstream destination (or document the residual risk).

  This ticket does NOT implement the K8s / iptables config — that's a
  mika-cloud follow-up (see the runbook doc).
EOF

# `MIKA_AUDIT_SUPPRESS_L2_WARN` was removed by mika#1806. It is still read here
# for one purpose only: to tell an operator who believes they attested that they
# did not. Silently ignoring it would reproduce the defect one notch lower —
# same reasoning and same shape as `auto_pull_stop_stale_env_knob` (mika#2329).
stale_suppress="${MIKA_AUDIT_SUPPRESS_L2_WARN:-}"
if [[ -n "$stale_suppress" ]] && [[ "$stale_suppress" != "0" ]]; then
    echo ""
    echo "  WARN: MIKA_AUDIT_SUPPRESS_L2_WARN is set and no longer suppresses"
    echo "        anything — record an attestation with --attest instead:"
    echo "          scripts/audit-egress-no-log.sh --attest \\"
    echo "              --target <deployment> --by <who>"
    echo "        (This warning changes no exit code. An operator who thinks"
    echo "         they have attested must learn it here, not on an incident.)"
fi

echo ""
evaluate_attestation "$TTL_DAYS"
for line in ${L2_LINES[@]+"${L2_LINES[@]}"}; do
    echo "  $line"
done

if [[ "$L2_VERDICT" != "attested" ]]; then
    warnings=1
    echo "  (this run: exit 2 — Layer 2 owed)"
fi

# ------------------------------------------------------------------
# Layer 3 — persistence audit (SQLite DB scan)
# ------------------------------------------------------------------

emit_header "Layer 3 — persistence audit (on-disk state)"

if [[ ! -f "$DB_PATH" ]]; then
    echo "  SKIP: no SQLite DB at $DB_PATH"
    echo "  (mika-gateway itself uses Postgres — this DB path is the agent"
    echo "   container's data dir. If you deployed egress-search inside an"
    echo "   agent, set MIKA_GATEWAY_DB to point at the correct file.)"
else
    if ! command -v sqlite3 >/dev/null 2>&1; then
        echo "  SKIP: sqlite3 CLI not installed — cannot probe $DB_PATH"
        echo "  Install with: apt install sqlite3  (or: emerge dev-db/sqlite)"
    else
        echo "  DB: $DB_PATH"

        # Substrate-shaped table names — a table named `search_egress*` /
        # `brave_egress*` / `search_upstream*` only exists if someone added
        # substrate-side persistence, which is the exact discipline break E4
        # forbids. Non-substrate tables that merely contain the word "search"
        # (fts_search, vec_search, search_content — all KG-lexical
        # infrastructure) are called out separately as NOTE, not LEAK.
        substrate_shaped=$(
            sqlite3 "$DB_PATH" <<'SQL' 2>/dev/null || true
.mode list
SELECT name FROM sqlite_master
WHERE type='table'
  AND (lower(name) LIKE 'search_egress%'
    OR lower(name) LIKE 'brave%'
    OR lower(name) LIKE 'search_upstream%'
    OR lower(name) LIKE 'search_cache%');
SQL
        )
        if [[ -n "$substrate_shaped" ]]; then
            echo "  LEAK (Layer 3): substrate-shaped tables found — the E4 invariant"
            echo "  forbids ANY substrate-side persistence of egress traffic:"
            printf "%s\n" "$substrate_shaped" | sed 's/^/    /'
            leak_found=1
        else
            echo "  OK: no substrate-shaped tables (search_egress*, brave*, etc.)."
        fi

        # Informational sweep — any table with 'search' in the name (usually
        # KG-lexical: fts_search, vec_search, search_content). NOT flagged as
        # leak; the operator should confirm these are the expected KG surfaces
        # and hold no substrate-originated egress content.
        other_search_tables=$(
            sqlite3 "$DB_PATH" <<'SQL' 2>/dev/null || true
.mode list
SELECT name FROM sqlite_master
WHERE type='table'
  AND lower(name) LIKE '%search%'
  AND lower(name) NOT LIKE 'search_egress%'
  AND lower(name) NOT LIKE 'brave%'
  AND lower(name) NOT LIKE 'search_upstream%'
  AND lower(name) NOT LIKE 'search_cache%';
SQL
        )
        if [[ -n "$other_search_tables" ]]; then
            echo "  NOTE: tables with 'search' in the name (KG-lexical surfaces —"
            echo "  spot-check they hold no substrate-originated egress content):"
            printf "%s\n" "$other_search_tables" | sed 's/^/    /'
        fi

        # Sanity check — list any tables with a `query` column so an operator
        # can spot-check their content isn't full of Brave queries.
        query_columned=$(
            sqlite3 "$DB_PATH" <<'SQL' 2>/dev/null || true
.mode list
SELECT DISTINCT m.name
FROM sqlite_master m, pragma_table_info(m.name) p
WHERE m.type='table' AND lower(p.name)='query';
SQL
        )
        if [[ -n "$query_columned" ]]; then
            echo "  NOTE: tables with a 'query' column (spot-check content):"
            printf "%s\n" "$query_columned" | sed 's/^/    /'
            echo "  These are usually agent-side (tool_calls, kg_*, memory) —"
            echo "  gated by MIKA_STORE_TOOL_CALLS. Confirm they hold no"
            echo "  substrate-originated search-egress content."
        fi
    fi
fi

# ------------------------------------------------------------------
# Final report
# ------------------------------------------------------------------

emit_header "Result"

if [[ $leak_found -eq 1 ]]; then
    echo "  FAIL: at least one leak detected. See lines above."
    exit "$EXIT_LEAK"
fi

if [[ $warnings -eq 1 ]]; then
    echo "  PASS (Layer 1 + Layer 3), Layer 2 owed — see the Layer 2 section above."
    exit "$EXIT_L2_OWED"
fi

# A green must carry what it is green about: the whole point of mika#1806 is
# that "attested now, here" and "attested long ago, elsewhere" stop printing the
# same bytes.
echo "  PASS: all layers clean."
for line in ${L2_LINES[@]+"${L2_LINES[@]}"}; do
    echo "  $line"
done
exit "$EXIT_OK"
