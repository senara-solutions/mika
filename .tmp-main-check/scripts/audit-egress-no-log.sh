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
#   0 — every automated check clean, Layer 2 manual audit still owed
#   1 — leak detected (any layer)
#   2 — Layer 2 explicit warning fired (default on every run, additive to 0/1)
#
# The default posture is exit 2 when the layer-1/layer-3 checks pass, since
# Layer 2 always needs manual attention: it is a *substrate* property (kernel
# iptables, K8s NetworkPolicy, proxy config) that this script cannot inspect
# in-container. Set `MIKA_AUDIT_SUPPRESS_L2_WARN=1` to downgrade to exit 0
# when running from an automated cron and Layer 2 has been confirmed clean
# out-of-band.
#
# Environment (all optional; sensible defaults for a dev machine):
#   MIKA_GATEWAY_LOG_FILE        — path to mika-gateway JSON log file.
#                                  Default: $MIKA_SPIRIT_LOG_FILE or
#                                  ~/.mika/logs/mika-gateway.log
#   MIKA_GATEWAY_DB              — path to the gateway/spirit SQLite DB.
#                                  Default: $HOME/.mika/data/mika.db
#   MIKA_AUDIT_SUPPRESS_L2_WARN  — 1 to downgrade the Layer 2 warning from
#                                  exit 2 to exit 0.

set -euo pipefail

LOG_FILE="${MIKA_GATEWAY_LOG_FILE:-${MIKA_SPIRIT_LOG_FILE:-$HOME/.mika/logs/mika-gateway.log}}"
DB_PATH="${MIKA_GATEWAY_DB:-$HOME/.mika/data/mika.db}"
SUPPRESS_L2_WARN="${MIKA_AUDIT_SUPPRESS_L2_WARN:-0}"

leak_found=0
warnings=0

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

if [[ "$SUPPRESS_L2_WARN" != "1" ]]; then
    warnings=1
    echo "  WARN: Layer 2 manual audit still owed (this run: exit 2)."
    echo "        Set MIKA_AUDIT_SUPPRESS_L2_WARN=1 to downgrade once the"
    echo "        substrate side has been confirmed clean out-of-band."
else
    echo "  NOTE: Layer 2 warning suppressed via MIKA_AUDIT_SUPPRESS_L2_WARN=1."
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
    exit 1
fi

if [[ $warnings -eq 1 ]]; then
    echo "  PASS (Layer 1 + Layer 3), Layer 2 audit still owed."
    exit 2
fi

echo "  PASS: all layers clean (Layer 2 confirmed via MIKA_AUDIT_SUPPRESS_L2_WARN)."
exit 0
